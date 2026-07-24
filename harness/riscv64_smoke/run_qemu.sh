#!/usr/bin/env bash
# Assemble/link riscv64 asm with cross GCC and run under qemu-user (Docker).
# Host macOS typically lacks qemu-riscv64 + riscv64-linux-gnu-gcc.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ASM="${1:?usage: run_qemu.sh <file.s> [expected_ret]}"
EXPECT_RET="${2:-0}"
BASE="$(basename "$ASM" .s)"
OUTDIR="$(cd "$(dirname "$ASM")" && pwd)"
BIN="$OUTDIR/$BASE.riscv64"

if [[ ! -f "$ASM" ]]; then
  echo "missing asm: $ASM" >&2
  exit 1
fi

# Prefer a persistent image if present; else ephemeral ubuntu:24.04.
IMG="${GGCC_RISCV_DOCKER_IMAGE:-ubuntu:24.04}"

docker run --rm --platform linux/amd64 \
  -v "$OUTDIR:/work" \
  -w /work \
  "$IMG" \
  bash -lc "
    set -euo pipefail
    if ! command -v riscv64-linux-gnu-gcc >/dev/null 2>&1; then
      apt-get update -qq
      DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
        gcc-riscv64-linux-gnu qemu-user-static >/dev/null
    fi
    riscv64-linux-gnu-gcc -static -o '/work/${BASE}.riscv64' '/work/${BASE}.s'
    set +e
    qemu-riscv64-static '/work/${BASE}.riscv64'
    RET=\$?
    set -e
    echo \"qemu-riscv64 exit=\$RET\"
    if [[ \"\$RET\" -ne '${EXPECT_RET}' ]]; then
      echo \"FAIL: expected ret ${EXPECT_RET}, got \$RET\" >&2
      exit 1
    fi
    # Capture stdout for hello-style checks when EXPECT_RET=0 and file exists
    true
  "

echo "PASS: $BIN (ret=$EXPECT_RET)"
