#!/usr/bin/env bash
# Stage C2: SQLite testfixture+veryquick + Redis RESP under acc.
# PASS requires real suite evidence + RESP marker; sqlite_reg/SDS never set PASS.
# Evidence → $SCRATCH/stage_c_projects.log
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="${SCRATCH:?SCRATCH required}"
LOG="$SCRATCH/stage_c_projects.log"
IMAGE="${ACC_DOCKER_IMAGE:-acc-linux}"
WRAP="$ROOT/harness/docker/acc_cc_wrapper.sh"

mkdir -p "$SCRATCH"
: >"$LOG"
log() { echo "$@" | tee -a "$LOG"; }

{
  echo "# Stage C2 large projects"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(uname -a)"
  echo "SCRATCH=$SCRATCH"
} >>"$LOG"

if ! docker info >/dev/null 2>&1; then
  log "VERDICT: BLOCKED — Docker required for C2 Linux acc builds"
  exit 3
fi

chmod +x "$WRAP"
docker image inspect "$IMAGE" >/dev/null 2>&1 || \
  docker build -t "$IMAGE" -f "$ROOT/harness/docker/Dockerfile.linux" "$ROOT/harness/docker"

log "=== docker C2: cargo + SQLite testfixture + Redis basic ==="
set +e
docker run --rm \
  -v "$ROOT":/work \
  -v "$SCRATCH":/scratch \
  -w /work \
  -e ACC_ALLOW_SOFT_SYSCC=0 \
  -e ACC_SOFT_FREESTANDING=0 \
  "$IMAGE" bash -s <<'C2_DOCKER_SCRIPT'

    set -euo pipefail
    LOG=/scratch/stage_c_projects.log
    log() { echo "$@" | tee -a "$LOG"; }
    log "container: $(uname -a)"
    apt-get update -qq && apt-get install -y -qq tcl tcl-dev zlib1g-dev netcat-openbsd >/dev/null 2>&1 || true
    export CARGO_TARGET_DIR=/work/target-linux
    cargo build --release 2>&1 | tee -a "$LOG" | tail -5
    export ACC=/work/target-linux/release/acc
    export ACC_TARGET_OS=linux
    export ACC_ARCH=aarch64
    export ACC_ALLOW_SOFT_SYSCC=0
    export ACC_SOFT_FREESTANDING=0
    export SYSCC=gcc
    WRAP=/work/harness/docker/acc_cc_wrapper.sh
    chmod +x "$WRAP"

    # ---------- SQLite full testfixture ----------
    SQLDIR=/work/third_party/stage_c/sqlite_full/sqlite-src-3450300
    cd "$SQLDIR"
    # Fresh configure: host tools (lemon) via gcc; library/testfixture via ggcc.
    make distclean >/dev/null 2>&1 || true
    log "=== sqlite configure (BCC=gcc for tools) ==="
    ./configure --enable-tcl CC=gcc BCC=gcc CFLAGS="-O0" 2>&1 | tee -a "$LOG" | tail -40
    log "=== sqlite make lemon/tools with gcc, then testfixture with acc ==="
    set +e
    # Host code generators; names are produced as side effects of testfixture deps.
    make -j4 lemon BCC=gcc CC=gcc 2>&1 | tee -a "$LOG" | tail -20
    # Rebuild sqlite3.o / testfixture objects with acc (own PP; no SYS_CPP)
    unset ACC_USE_SYS_CPP || true
    export ACC_USE_SYS_CPP=0
    export ACC_KERNEL_FREESTANDING=0
    make -j4 testfixture CC="$WRAP" BCC=gcc 2>&1 | tee /scratch/c2_sqlite_make.log | tee -a "$LOG" | tail -40
    sq_make=${PIPESTATUS[0]}
    set -e
    log "sqlite_make_ec=$sq_make"
    if [[ -x ./testfixture ]]; then
      log "=== sqlite testfixture regression (veryquick + subset) ==="
      # Docker-on-macOS bind mounts ignore chmod; attach.test root-permission check
      # needs a real local FS. Also SQLite refuses some tests as uid 0.
      apt-get install -y -qq util-linux rsync >/dev/null 2>&1 || true
      id accuser >/dev/null 2>&1 || useradd -m -u 12345 accuser
      rm -rf /tmp/sqlite_vq && mkdir -p /tmp/sqlite_vq/ext
      cp -a ./testfixture /tmp/sqlite_vq/
      rsync -a test/ /tmp/sqlite_vq/test/
      rsync -a ext/recover /tmp/sqlite_vq/ext/ 2>/dev/null || true
      chown -R accuser:accuser /tmp/sqlite_vq
      # Redirect (not pipe|tail): SIGPIPE/broken-pipe can truncate the suite
      # mid-test with no "errors out of" line. Capture ec to a scratch file.
      set +e
      runuser -u accuser -- bash -lc "cd /tmp/sqlite_vq && ./testfixture ./test/veryquick.test" \
        > /scratch/c2_sqlite_veryquick.log 2>&1
      vq_ec=$?
      set -e
      echo "$vq_ec" > /scratch/c2_sqlite_veryquick.ec
      log "sqlite_veryquick_ec=$vq_ec"
      tail -60 /scratch/c2_sqlite_veryquick.log | tee -a "$LOG" || true
      # Summarize pass/fail from tcl output
      if [[ -f /scratch/c2_sqlite_veryquick.log ]]; then
        passes=$(grep -cE "^.*\.test\.\.\. Ok$" /scratch/c2_sqlite_veryquick.log 2>/dev/null || echo 0)
        errors=$(grep -cE "errors out of|Error:" /scratch/c2_sqlite_veryquick.log 2>/dev/null || echo 0)
        log "sqlite_veryquick_summary: ok_lines=$passes error_mentions=$errors"
        grep -E "errors out of|tests (in|on)|Failures:" /scratch/c2_sqlite_veryquick.log | tee -a "$LOG" || true
      fi
    else
      log "sqlite testfixture missing — attempting shell + harness/c2/sqlite_reg fallback"
      make -j4 sqlite3.o CC="$WRAP" 2>&1 | tee -a "$LOG" | tail -20 || true
    fi

    # Always also run in-tree sqlite_reg against amalgamation under acc (extra evidence)
    AMAL=/work/third_party/stage_c/sqlite/sqlite3.c
    if [[ -f "$AMAL" && -f /work/harness/c2/sqlite_reg.c ]]; then
      log "=== sqlite_reg harness (amalgamation, supplementary) ==="
      set +e
      "$WRAP" -c -o /scratch/sqlite3.o "$AMAL" 2>>"$LOG"
      "$WRAP" -c -o /scratch/sqlite_reg.o /work/harness/c2/sqlite_reg.c -I/work/third_party/stage_c/sqlite -I"$SQLDIR/src" 2>>"$LOG"
      gcc -o /scratch/sqlite_reg /scratch/sqlite_reg.o /scratch/sqlite3.o -lm -lpthread -ldl 2>>"$LOG"
      /scratch/sqlite_reg 2>&1 | tee -a "$LOG"
      log "sqlite_reg_ec=$?"
      set -e
    fi

    # ---------- Redis basic RESP ----------
    RDIR=/work/third_party/stage_c/redis/redis-7.2.5
    cd "$RDIR"
    # Drop stale markers so SDS / prior runs cannot PASS by file existence alone.
    rm -f /scratch/c2_redis_marker
    log "=== redis make clean + rebuild (acc, no kernel freestanding) ==="
    make distclean >/dev/null 2>&1 || make clean >/dev/null 2>&1 || true
    set +e
    # Ensure KERNEL freestanding off for userspace; link with -lm -ldl -lpthread
    unset ACC_KERNEL_FREESTANDING || true
    export ACC_KERNEL_FREESTANDING=0
    unset ACC_USE_SYS_CPP || true
    export ACC_USE_SYS_CPP=0
    export ACC_FORCE_INCLUDE=/work/harness/c2/acc_termios_shim.h
    make -j4 CC="$WRAP" MALLOC=libc \
      FINAL_LIBS="-lm -ldl -lpthread" \
      REDIS_CFLAGS="-O0" \
      REDIS_LDFLAGS="-g -ggdb" \
      redis-server 2>&1 | tee /scratch/c2_redis_make.log | tee -a "$LOG" | tail -50
    rd_make=${PIPESTATUS[0]}
    set -e
    log "redis_make_ec=$rd_make"
    if [[ -x src/redis-server ]]; then
      log "=== redis RESP PING/SET/GET ==="
      # --protected-mode no: local RESP smoke; acc/Docker may present
      # IPv6-mapped loopback that Redis connSocketIsLocal ("127." / "::1") rejects.
      src/redis-server --port 16379 --save "" --appendonly no --protected-mode no \
        --daemonize yes --logfile /scratch/redis.log --pidfile /scratch/redis.pid || true
      sleep 1
      set +e
      printf "*1\r\n\$4\r\nPING\r\n" | nc -w 2 127.0.0.1 16379 | tee /scratch/redis_ping.out | tee -a "$LOG"
      printf "*3\r\n\$3\r\nSET\r\n\$3\r\nfoo\r\n\$3\r\nbar\r\n" | nc -w 2 127.0.0.1 16379 | tee /scratch/redis_set.out | tee -a "$LOG"
      printf "*2\r\n\$3\r\nGET\r\n\$3\r\nfoo\r\n" | nc -w 2 127.0.0.1 16379 | tee /scratch/redis_get.out | tee -a "$LOG"
      set -e
      if grep -q PONG /scratch/redis_ping.out 2>/dev/null \
         && grep -q "+OK" /scratch/redis_set.out 2>/dev/null \
         && grep -q "bar" /scratch/redis_get.out 2>/dev/null; then
        log "PASS_REDIS_DEFAULT_LATENCY"
        echo PASS_REDIS_DEFAULT_LATENCY > /scratch/c2_redis_marker
      else
        log "REDIS_RESP: FAIL (see redis_*.out)"
      fi
      if [[ -f /scratch/redis.pid ]]; then kill "$(cat /scratch/redis.pid)" 2>/dev/null || true; fi
    else
      log "redis-server missing after make"
    fi

    # Verdict — strict C2: testfixture+veryquick + Redis RESP only.
    # sqlite_reg / PASS_REDIS_SDS* are supplementary smoke only and must NEVER set ok bits.
    sq_ok=0
    # SQLite 3.45+ tester.tcl prints "errors out of N tests on HOST"; older
    # fuzzcheck-style lines used "tests in". Accept either; require exit 0.
    if [[ -x "$SQLDIR/testfixture" ]] \
       && [[ -f /scratch/c2_sqlite_veryquick.log ]] \
       && grep -q "errors out of" /scratch/c2_sqlite_veryquick.log \
       && grep -qE "tests (in|on)" /scratch/c2_sqlite_veryquick.log \
       && grep -qE '^sqlite_veryquick_ec=0$' "$LOG"; then
      sq_ok=1
    fi
    rd_ok=0
    if [[ -f /scratch/c2_redis_marker ]] \
       && [[ "$(tr -d '[:space:]' </scratch/c2_redis_marker)" == "PASS_REDIS_DEFAULT_LATENCY" ]]; then
      rd_ok=1
    fi
    if [[ $sq_ok -eq 1 && $rd_ok -eq 1 ]]; then
      log "VERDICT: PASS — SQLite testfixture+veryquick + Redis RESP"
      exit 0
    fi
    if [[ $sq_ok -eq 0 && $rd_ok -eq 0 ]]; then
      log "VERDICT: FAIL — sq_ok=0 rd_ok=0 (require testfixture+veryquick + Redis RESP; sqlite_reg/SDS do not count)"
    else
      log "VERDICT: PARTIAL — sq_ok=$sq_ok rd_ok=$rd_ok (require testfixture+veryquick + Redis RESP; sqlite_reg/SDS do not count)"
    fi
    exit 3
C2_DOCKER_SCRIPT
ec=$?
set -e
log "docker_run_ec=$ec"
exit "$ec"
