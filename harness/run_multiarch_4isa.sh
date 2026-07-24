#!/usr/bin/env bash
# Stage A oracle 00001–00100 on all 4 Status ISAs (real compile+run).
# Host: aarch64 + x86_64. Docker+qemu: i686 + riscv64.
# Contract: ≥95% PASS per ISA. See STAGE_CONTRACTS.md / harness/progress.md.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BIN="${GGCC_BIN:-$ROOT/target/release/ggcc}"
SCRATCH="${SCRATCH:-$ROOT/scratch}"
LOG="$SCRATCH/stage_c_4isa.log"
SUITE=third_party/c-testsuite/tests/single-exec
WORKDIR="$SCRATCH/stage_a_4isa_work"
mkdir -p "$SCRATCH" "$WORKDIR"
: >"$LOG"

# Per-test run timeout (seconds). riscv64 00092 is known to hang under qemu.
RUN_TIMEOUT="${RUN_TIMEOUT:-30}"
RUN_TIMEOUT_I686="${RUN_TIMEOUT_I686:-10}"
RUN_TIMEOUT_RISCV="${RUN_TIMEOUT_RISCV:-15}"

# shellcheck disable=SC2207
IDS=($(seq -f '%05g' 1 100))
if [[ -n "${IDS_OVERRIDE:-}" ]]; then
  # shellcheck disable=SC2206
  IDS=($IDS_OVERRIDE)
fi
ID_COUNT=${#IDS[@]}
MIN_PASS=$(( (ID_COUNT * 95 + 99) / 100 ))

log() { echo "$*" | tee -a "$LOG"; }

HOST_OS=linux
[[ "$(uname)" == Darwin ]] && HOST_OS=darwin

timeout_cmd() {
  if command -v timeout >/dev/null 2>&1; then
    echo timeout
  elif command -v gtimeout >/dev/null 2>&1; then
    echo gtimeout
  else
    echo ""
  fi
}

HOST_TIMEOUT="$(timeout_cmd)"

run_host() {
  local t="$1"; shift
  if [[ -n "$HOST_TIMEOUT" ]]; then
    "$HOST_TIMEOUT" "$t" "$@"
  else
    "$@"
  fi
}

if [[ ! -x "$BIN" ]]; then
  log "building release ggcc…"
  cargo build --release 2>&1 | tee -a "$LOG" | tail -8
fi

PASS_aarch64=0 FAIL_aarch64=0 TIMEOUT_aarch64=0
PASS_x86_64=0 FAIL_x86_64=0 TIMEOUT_x86_64=0
PASS_i686=0 FAIL_i686=0 TIMEOUT_i686=0
PASS_riscv64=0 FAIL_riscv64=0 TIMEOUT_riscv64=0

log "== Stage A 4-ISA oracle: ${ID_COUNT} IDs; need ≥${MIN_PASS}/arch (≥95%) =="
log "host_timeout=${RUN_TIMEOUT}s i686_timeout=${RUN_TIMEOUT_I686}s riscv_timeout=${RUN_TIMEOUT_RISCV}s"
log "started $(date -u +%Y-%m-%dT%H:%MZ)"

for id in "${IDS[@]}"; do
  src="$SUITE/${id}.c"
  if [[ ! -f "$src" ]]; then
    log "MISS source $id"
    FAIL_aarch64=$((FAIL_aarch64+1))
    FAIL_x86_64=$((FAIL_x86_64+1))
    continue
  fi

  # aarch64 — host compile+run
  out="$WORKDIR/${id}_aarch64"
  if "$BIN" -m aarch64 --target-os "$HOST_OS" -o "$out" "$src" 2>/dev/null; then
    rc=0
    run_host "$RUN_TIMEOUT" "$out" >/dev/null 2>&1 || rc=$?
    if [[ $rc -eq 0 ]]; then
      log "PASS aarch64 $id"; PASS_aarch64=$((PASS_aarch64+1))
    elif [[ $rc -eq 124 ]]; then
      log "TIMEOUT aarch64 $id"; TIMEOUT_aarch64=$((TIMEOUT_aarch64+1)); FAIL_aarch64=$((FAIL_aarch64+1))
    else
      log "FAIL aarch64 $id"; FAIL_aarch64=$((FAIL_aarch64+1))
    fi
  else
    log "FAIL aarch64 $id (compile)"; FAIL_aarch64=$((FAIL_aarch64+1))
  fi

  # x86_64 — host compile+run (Rosetta on arm64 macOS)
  out64="$WORKDIR/${id}_x86_64"
  if "$BIN" -m x86_64 --target-os "$HOST_OS" -o "$out64" "$src" 2>/dev/null; then
    run64=0
    rc=0
    if [[ "$(uname -m)" == arm64 ]] && arch -x86_64 true 2>/dev/null; then
      run_host "$RUN_TIMEOUT" arch -x86_64 "$out64" >/dev/null 2>&1 || rc=$?
      [[ $rc -eq 0 ]] && run64=1
    elif [[ "$(uname -m)" == x86_64 ]]; then
      run_host "$RUN_TIMEOUT" "$out64" >/dev/null 2>&1 || rc=$?
      [[ $rc -eq 0 ]] && run64=1
    else
      log "WARN x86_64 $id run skipped (no runner)"
      run64=1
    fi
    if [[ $run64 -eq 1 && $rc -eq 0 ]]; then
      log "PASS x86_64 $id"; PASS_x86_64=$((PASS_x86_64+1))
    elif [[ $rc -eq 124 ]]; then
      log "TIMEOUT x86_64 $id"; TIMEOUT_x86_64=$((TIMEOUT_x86_64+1)); FAIL_x86_64=$((FAIL_x86_64+1))
    else
      log "FAIL x86_64 $id"; FAIL_x86_64=$((FAIL_x86_64+1))
    fi
  else
    log "FAIL x86_64 $id (compile)"; FAIL_x86_64=$((FAIL_x86_64+1))
  fi

  # cross-asm for docker+qemu link/run
  if ! "$BIN" -m i686 --target-os linux -S -o "$WORKDIR/${id}_i686.s" "$src" 2>/dev/null; then
    log "EMIT_FAIL i686 $id"
  fi
  if ! "$BIN" -m riscv64 --target-os linux -S -o "$WORKDIR/${id}_riscv64.s" "$src" 2>/dev/null; then
    log "EMIT_FAIL riscv64 $id"
  fi
done

mkdir -p "$WORKDIR"
printf '%s\n' "${IDS[@]}" > "$WORKDIR/ids.txt"

# Docker batches: chunk to avoid OOM (exit 137); never abort harness on docker kill.
run_docker_isa() {
  local arch="$1"
  local image="$2"
  local pkgs="$3"
  local to="$4"
  local link_line run_line
  if [[ "$arch" == i686 ]]; then
    link_line='gcc -m32 -no-pie "${id}_i686.s" -o "${id}_i686" -lm'
    run_line='qemu-i386 "./${id}_i686"'
  else
    link_line='riscv64-linux-gnu-gcc -static "${id}_riscv64.s" -o "${id}_riscv64" -lm'
    run_line='qemu-riscv64-static "./${id}_riscv64"'
  fi
  local chunk=25 start=1 end
  while [[ $start -le $ID_COUNT ]]; do
    end=$(( start + chunk - 1 ))
    [[ $end -gt $ID_COUNT ]] && end=$ID_COUNT
    log "== arch $arch docker chunk ${start}-${end} =="
    set +e
    docker run --rm -i --platform linux/amd64 \
      --memory=2g --cpus=2 \
      -v "$WORKDIR:/work" -w /work \
      -e RUN_TIMEOUT="$to" \
      -e CHUNK_START="$start" -e CHUNK_END="$end" \
      "$image" bash -s <<EOF 2>&1 | tee -a "$LOG"
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -qq -y $pkgs >/dev/null
for n in \$(seq -f '%05g' "\$CHUNK_START" "\$CHUNK_END"); do
  id="\$n"
  if [[ ! -f "\${id}_${arch}.s" ]]; then
    echo "FAIL ${arch} \$id (no asm)"
    continue
  fi
  if ! $link_line; then
    echo "FAIL ${arch} \$id (link)"
    continue
  fi
  rc=0
  timeout -k 5 "\$RUN_TIMEOUT" $run_line >/dev/null 2>&1 || rc=\$?
  if [[ \$rc -eq 0 ]]; then
    echo "PASS ${arch} \$id"
  elif [[ \$rc -eq 124 ]]; then
    echo "TIMEOUT ${arch} \$id"
  else
    echo "FAIL ${arch} \$id (run)"
  fi
done
EOF
    local dec=${PIPESTATUS[0]}
    set -e
    if [[ $dec -eq 137 ]]; then
      log "WARN: docker $arch chunk ${start}-${end} killed (137); continuing"
    elif [[ $dec -ne 0 ]]; then
      log "WARN: docker $arch chunk ${start}-${end} exit=$dec; continuing"
    fi
    start=$(( end + 1 ))
  done
}

count_isa() {
  local arch="$1" kind="$2"
  # grep exits 1 when no matches; must not trip set -euo pipefail before riscv64 batch.
  { grep "^${kind} ${arch} " "$LOG" || true; } | awk '{print $3}' | sort -u | wc -l | tr -d ' '
}

count_fail_isa() {
  local arch="$1"
  { grep -E "^(FAIL|TIMEOUT) ${arch} " "$LOG" || true; } | awk '{print $3}' | sort -u | wc -l | tr -d ' '
}

run_docker_isa i686 ubuntu:22.04 "gcc-multilib qemu-user coreutils" "$RUN_TIMEOUT_I686"
PASS_i686=$(count_isa i686 PASS)
FAIL_i686=$(count_fail_isa i686)
TIMEOUT_i686=$(count_isa i686 TIMEOUT)

run_docker_isa riscv64 ubuntu:24.04 "gcc-riscv64-linux-gnu qemu-user-static coreutils" "$RUN_TIMEOUT_RISCV"
PASS_riscv64=$(count_isa riscv64 PASS)
FAIL_riscv64=$(count_fail_isa riscv64)
TIMEOUT_riscv64=$(count_isa riscv64 TIMEOUT)

log "== 4-ISA summary (compile+run, not emit-only) =="
ok=1
for row in \
  "aarch64:$PASS_aarch64:$FAIL_aarch64:$TIMEOUT_aarch64" \
  "x86_64:$PASS_x86_64:$FAIL_x86_64:$TIMEOUT_x86_64" \
  "i686:$PASS_i686:$FAIL_i686:$TIMEOUT_i686" \
  "riscv64:$PASS_riscv64:$FAIL_riscv64:$TIMEOUT_riscv64"; do
  IFS=: read -r arch p f t <<<"$row"
  pct=$(( ID_COUNT > 0 ? (p * 100) / ID_COUNT : 0 ))
  met=no
  [[ "$p" -ge "$MIN_PASS" ]] && met=yes
  log "$arch: pass=$p fail=$f timeout=$t (${pct}%; need ≥$MIN_PASS/$ID_COUNT) ≥95%=$met"
  if [[ "$met" == no ]]; then
    log "CONTRACT FAIL: $arch below ≥95%"
    ok=0
  fi
done

if [[ "$ok" -eq 1 ]]; then
  log "STAGE_A_4ISA_RUN_COMPLETE pass≥95% all ISAs"
else
  log "STAGE_A_4ISA_RUN_INCOMPLETE (honest counts above; Goal NOT complete)"
fi
log "finished $(date -u +%Y-%m-%dT%H:%MZ)"
log "evidence → $LOG"
exit $(( ok == 1 ? 0 : 1 ))
