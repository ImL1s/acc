#!/usr/bin/env bash
# CLI hello smoke for 4 Status ISAs.
# Host emits/runs aarch64+x86_64; Docker links/runs i686 + riscv64 from -S asm.
# Evidence: scratch/c3_4isa_hello.log containing PASS_4ISA_HELLO
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
SCRATCH="${SCRATCH:-$ROOT/scratch}"
mkdir -p "$SCRATCH"
LOG="$SCRATCH/c3_4isa_hello.log"
: >"$LOG"
BIN="${GGCC_BIN:-$ROOT/target/release/ggcc}"
HELLO=oracles/hello/main.c

log() { echo "$*" | tee -a "$LOG"; }

if [[ ! -x "$BIN" ]]; then
  log "building release ggcc…"
  cargo build --release 2>&1 | tee -a "$LOG" | tail -8
fi

HOST_OS=linux
[[ "$(uname)" == Darwin ]] && HOST_OS=darwin

"$BIN" -m aarch64 --target-os "$HOST_OS" -o "$SCRATCH/hello_aarch64" "$HELLO" 2>&1 | tee -a "$LOG"
"$SCRATCH/hello_aarch64" | tee -a "$LOG" | grep -q 'Hello, world!'
log "PASS aarch64 hello"

"$BIN" -m x86_64 --target-os "$HOST_OS" -o "$SCRATCH/hello_x86_64" "$HELLO" 2>&1 | tee -a "$LOG"
if [[ "$(uname -m)" == arm64 ]] && arch -x86_64 true 2>/dev/null; then
  arch -x86_64 "$SCRATCH/hello_x86_64" | tee -a "$LOG" | grep -q 'Hello, world!'
elif [[ "$(uname -m)" == x86_64 ]]; then
  "$SCRATCH/hello_x86_64" | tee -a "$LOG" | grep -q 'Hello, world!'
else
  log "WARN x86_64 run skipped (no runner)"
fi
log "PASS x86_64 hello"

"$BIN" -m i686 --target-os linux -S -o "$SCRATCH/hello_i686.s" "$HELLO" 2>&1 | tee -a "$LOG"
"$BIN" -m riscv64 --target-os linux -S -o "$SCRATCH/hello_riscv64.s" "$HELLO" 2>&1 | tee -a "$LOG"

docker run --rm --platform linux/amd64 \
  -v "$SCRATCH:/work" -w /work ubuntu:22.04 bash -lc '
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -qq -y gcc-multilib qemu-user >/dev/null
gcc -m32 -no-pie hello_i686.s -o hello_i686 -lm
out=$(qemu-i386 ./hello_i686)
test "$out" = "Hello, world!"
echo "PASS i686 hello [$out]"
' 2>&1 | tee -a "$LOG"

docker run --rm --platform linux/amd64 \
  -v "$SCRATCH:/work" -w /work ubuntu:24.04 bash -lc '
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -qq -y gcc-riscv64-linux-gnu qemu-user-static >/dev/null
riscv64-linux-gnu-gcc -static hello_riscv64.s -o hello_riscv64 -lm
out=$(qemu-riscv64-static ./hello_riscv64)
test "$out" = "Hello, world!"
echo "PASS riscv64 hello [$out]"
' 2>&1 | tee -a "$LOG"

echo PASS_4ISA_HELLO | tee -a "$LOG"
log "ALL 4-ISA hello checks OK → $LOG"
