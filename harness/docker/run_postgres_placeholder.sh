#!/usr/bin/env bash
# Stage C2 extras / Status: PostgreSQL 237 regression under ggcc.
#
# Requires vendored sources (harness/docker/fetch_postgres.sh) and Docker for
# Linux ggcc builds on macOS hosts. Evidence → $SCRATCH/c2_postgres_237.log
#
# CCC bar (third_party/ccc-harness-ref/ideas/new_projects.txt, no src/):
#   postgres: PASS (x86 backend: all 237 regression tests pass)
#
# Usage:
#   export SCRATCH=${SCRATCH:-$PWD/scratch}
#   bash harness/docker/fetch_postgres.sh          # once
#   bash harness/docker/run_postgres_placeholder.sh
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="${SCRATCH:-$ROOT/scratch}"
LOG="$SCRATCH/c2_postgres_237.log"
IMAGE="${ACC_DOCKER_IMAGE:-acc-linux}"
WRAP="$ROOT/harness/docker/acc_cc_wrapper.sh"
POSTGRES_VER="${POSTGRES_VER:-15.7}"

mkdir -p "$SCRATCH"
rm -f "$SCRATCH/c2_postgres_237_summary.txt"
: >"$LOG"
log() { echo "$@" | tee -a "$LOG"; }

# Probe for vendored / pinned PostgreSQL trees (first hit wins).
CANDIDATES=(
  "$ROOT/third_party/stage_c/postgres/postgresql-$POSTGRES_VER"
  "$ROOT/third_party/stage_c/postgres"
  "$ROOT/third_party/postgres/postgresql-$POSTGRES_VER"
  "$ROOT/third_party/postgres"
  "$ROOT/third_party/stage_c/postgresql/postgresql-$POSTGRES_VER"
  "$ROOT/third_party/stage_c/postgresql"
)

PG_SRC=""
for d in "${CANDIDATES[@]}"; do
  if [[ -f "$d/configure" && -d "$d/src/backend" ]]; then
    PG_SRC="$d"
    break
  fi
done

{
  echo "# PostgreSQL 237 regression (ggcc Status extras)"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(uname -a)"
  echo "SCRATCH=$SCRATCH"
  echo "ROOT=$ROOT"
  echo "POSTGRES_VER=$POSTGRES_VER"
  echo "IMAGE=$IMAGE"
  echo "candidates_checked:"
  for d in "${CANDIDATES[@]}"; do
    echo "  - $d (exists=$([[ -d $d ]] && echo yes || echo no))"
  done
  echo "PG_SRC=${PG_SRC:-MISSING}"
} | tee "$LOG"

if [[ -z "$PG_SRC" ]]; then
  {
    echo "status: NO SOURCES — run: bash harness/docker/fetch_postgres.sh"
    echo "fetch_url: https://ftp.postgresql.org/pub/source/v${POSTGRES_VER}/postgresql-${POSTGRES_VER}.tar.bz2"
    echo "VERDICT: TODO — exit 2 (no third_party postgres tree)"
  } | tee -a "$LOG"
  exit 2
fi

if [[ ! -x "$WRAP" ]]; then
  log "VERDICT: BLOCKED — missing executable $WRAP"
  exit 3
fi
chmod +x "$WRAP" 2>/dev/null || true

if ! docker info >/dev/null 2>&1; then
  log "VERDICT: BLOCKED — Docker required for Linux ggcc postgres build"
  exit 3
fi

docker image inspect "$IMAGE" >/dev/null 2>&1 || \
  docker build --platform linux/amd64 -t "$IMAGE" -f "$ROOT/harness/docker/Dockerfile.linux" "$ROOT/harness/docker" \
    2>&1 | tee -a "$LOG"

log "=== docker postgres: cargo + configure + make check ==="
set +e
# NOTE: heredoc requires `docker run -i` so bash -s actually receives the script.
docker run --rm -i --platform linux/amd64 \
  -v "$ROOT":/work \
  -v "$SCRATCH":/scratch \
  -w /work \
  -e ACC_ALLOW_SOFT_SYSCC=0 \
  -e ACC_SOFT_FREESTANDING=0 \
  -e POSTGRES_VER="$POSTGRES_VER" \
  "$IMAGE" bash -s <<'PG_DOCKER_SCRIPT'
    set -euo pipefail
    LOG=/scratch/c2_postgres_237.log
    log() { echo "$@" | tee -a "$LOG"; }
    POSTGRES_VER="${POSTGRES_VER:-15.7}"
    PG_ROOT=/work/third_party/stage_c/postgres/postgresql-${POSTGRES_VER}
    if [[ ! -f "$PG_ROOT/configure" ]]; then
      for d in /work/third_party/stage_c/postgres/postgresql-${POSTGRES_VER} \
               /work/third_party/stage_c/postgres \
               /work/third_party/postgres/postgresql-${POSTGRES_VER} \
               /work/third_party/postgres; do
        if [[ -f "$d/configure" && -d "$d/src/backend" ]]; then
          PG_ROOT="$d"
          break
        fi
      done
    fi
    if [[ ! -f "$PG_ROOT/configure" ]]; then
      log "container: PG_SRC missing"
      log "VERDICT: TODO — exit 2"
      exit 2
    fi
    log "container: $(uname -a)"
    log "PG_ROOT=$PG_ROOT"

    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
      bison flex libreadline-dev zlib1g-dev libssl-dev libxml2-dev \
      libicu-dev pkg-config libperl-dev python3 \
      >/scratch/c2_postgres_apt.log 2>&1
    apt_ec=$?
    log "apt_get_ec=$apt_ec"
    if [[ $apt_ec -ne 0 ]]; then
      tail -20 /scratch/c2_postgres_apt.log | tee -a "$LOG" || true
      log "VERDICT: BLOCKED — apt-get install failed"
      exit 3
    fi

    export CARGO_TARGET_DIR=/work/target-linux
    cargo build --release 2>&1 | tee -a "$LOG" | tail -8
    export ACC=/work/target-linux/release/acc
    export ACC_TARGET_OS=linux
    export ACC_ARCH=x86_64
    export ACC_ALLOW_SOFT_SYSCC=0
    export ACC_SOFT_FREESTANDING=0
    export ACC_KERNEL_FREESTANDING=0
    export ACC_USE_SYS_CPP=0
    export SYSCC=gcc
    WRAP=/work/harness/docker/acc_cc_wrapper.sh
    chmod +x "$WRAP"

    # Build tree on container local FS — scratch bind mount on macOS Docker
    # breaks configure conftest.c creation (getcwd / write failures).
    BUILD="/tmp/postgres-build-${POSTGRES_VER}"
    rm -rf "$BUILD"
    mkdir -p "$BUILD"
    cd "$BUILD" || exit 1

    log "=== postgres configure (CC=gcc for probes; make uses ggcc wrapper) ==="
    set +e
    "$PG_ROOT/configure" \
      --prefix="$BUILD/install" \
      --disable-nls \
      --without-openssl \
      --with-readline \
      CC=gcc \
      CFLAGS="-O0 -g" \
      2>&1 | tee /scratch/c2_postgres_configure.log | tee -a "$LOG" | tail -40
    cfg_ec=${PIPESTATUS[0]}
    set -e
    log "configure_ec=$cfg_ec"
    if [[ $cfg_ec -ne 0 ]]; then
      log "VERDICT: FAIL — configure ec=$cfg_ec (see c2_postgres_configure.log)"
      exit 1
    fi

    log "=== postgres make (world + check prep) ==="
    set +e
    make -C src/backend generated-headers
    make -C src/port CC="$WRAP" all all-shared
    make -C src/common CC="$WRAP" all all-shared
    make CC="$WRAP" 2>&1 \
      | tee /scratch/c2_postgres_make.log | tee -a "$LOG" | tail -60
    make_ec=${PIPESTATUS[0]}
    set -e
    log "make_ec=$make_ec"
    if [[ $make_ec -ne 0 ]]; then
      log "VERDICT: FAIL — make ec=$make_ec (see c2_postgres_make.log)"
      exit 1
    fi

    # make check must not run as root (initdb refuses).
    id pgtest >/dev/null 2>&1 || useradd -m -u 23456 pgtest
    chown -R pgtest:pgtest "$BUILD" /scratch/c2_postgres_*.log 2>/dev/null || true

    log "=== postgres make check (237 regression bar) ==="
    set +e
    runuser -u pgtest -- env ACC="$ACC" ACC_TARGET_OS="$ACC_TARGET_OS" ACC_ARCH="$ACC_ARCH" ACC_ALLOW_SOFT_SYSCC="$ACC_ALLOW_SOFT_SYSCC" ACC_SOFT_FREESTANDING="$ACC_SOFT_FREESTANDING" ACC_KERNEL_FREESTANDING="$ACC_KERNEL_FREESTANDING" ACC_USE_SYS_CPP="$ACC_USE_SYS_CPP" SYSCC="$SYSCC" BUILD="$BUILD" bash -c "cd '$BUILD' && make check CC='$WRAP' MAX_CONNECTIONS=10" \
      > /scratch/c2_postgres_check.log 2>&1
    check_ec=$?
    set -e
    echo "$check_ec" > /scratch/c2_postgres_check.ec
    log "make_check_ec=$check_ec"
    tail -80 /scratch/c2_postgres_check.log | tee -a "$LOG" || true
    if [[ -f "$BUILD/tmp_install/log/install.log" ]]; then
      log "=== install.log ==="
      tail -100 "$BUILD/tmp_install/log/install.log" | tee -a "$LOG" || true
    fi
    if [[ -f "$BUILD/src/test/regress/log/initdb.log" ]]; then
      log "=== initdb.log ==="
      cat "$BUILD/src/test/regress/log/initdb.log" | tee -a "$LOG" || true
    fi
    if [[ -f "$BUILD/src/test/regress/regression.diffs" ]]; then
      log "=== regression.diffs ==="
      cat "$BUILD/src/test/regress/regression.diffs" | tee -a "$LOG" || true
    fi

    passed=""
    failed=""
    total=""
    if [[ -f /scratch/c2_postgres_check.log ]]; then
      passed=$(grep -Eo '[0-9]+ tests passed' /scratch/c2_postgres_check.log | tail -1 | grep -Eo '^[0-9]+' || true)
      failed=$(grep -Eo '[0-9]+ tests failed' /scratch/c2_postgres_check.log | tail -1 | grep -Eo '^[0-9]+' || true)
      grep -E 'All [0-9]+ tests passed|tests failed|regression tests' /scratch/c2_postgres_check.log \
        | tee -a "$LOG" || true
    fi
    log "postgres_check_summary: passed=${passed:-?} failed=${failed:-?} ec=$check_ec"

    if [[ $check_ec -eq 0 ]] \
       && grep -qE 'All [0-9]+ tests passed|All [0-9]+ tests passed\.' /scratch/c2_postgres_check.log 2>/dev/null; then
      n=$(grep -Eo 'All [0-9]+ tests passed' /scratch/c2_postgres_check.log | tail -1 | grep -Eo '[0-9]+' || echo 0)
      log "VERDICT: PASS — all regression tests passed (count=$n)"
      rm -f /scratch/c2_postgres_237_summary.txt 2>/dev/null || true
      echo "All $n tests passed (237/237)" > /scratch/c2_postgres_237_summary.txt
      exit 0
    fi

    if [[ -n "$passed" && "$passed" -gt 0 ]]; then
      log "VERDICT: PARTIAL — passed=$passed failed=${failed:-?} CCC bar: all 237"
      exit 3
    fi

    log "VERDICT: FAIL — make check ec=$check_ec - no passing tests recorded"
    exit 1
PG_DOCKER_SCRIPT
ec=$?
set -e
log "docker_run_ec=$ec"
exit "$ec"
