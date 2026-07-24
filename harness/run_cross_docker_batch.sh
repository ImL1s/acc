#!/usr/bin/env bash
# Cross-arch only: i686 + riscv64 docker batch (assumes .s already in WORKDIR).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(cd "${1:-$ROOT/scratch/stage_a_4isa_work}" && pwd)"
ARCH="${2:-both}"
TO="${3:-8}"
ids_file="$WORK/ids.txt"
[[ -f "$ids_file" ]] || seq -f '%05g' 1 100 > "$ids_file"

run_i686() {
  docker run --rm --platform linux/amd64 -v "$WORK:/work" -w /work ubuntu:22.04 bash -lc "
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -qq -y gcc-multilib qemu-user coreutils >/dev/null
while read -r id; do
  [[ -f \"\${id}_i686.s\" ]] || { echo \"FAIL i686 \$id\"; continue; }
  if ! gcc -m32 -no-pie \"\${id}_i686.s\" -o \"\${id}_i686\" -lm 2>/dev/null; then echo \"FAIL i686 \$id\"; continue; fi
  rc=0; timeout $TO qemu-i386 \"./\${id}_i686\" >/dev/null 2>&1 || rc=\$?
  if [[ \$rc -eq 0 ]]; then echo \"PASS i686 \$id\"
  elif [[ \$rc -eq 124 ]]; then echo \"TIMEOUT i686 \$id\"
  else echo \"FAIL i686 \$id\"; fi
done < ids.txt
"
}

run_riscv() {
  docker run --rm --platform linux/amd64 -v "$WORK:/work" -w /work ubuntu:24.04 bash -lc "
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -qq -y gcc-riscv64-linux-gnu qemu-user-static coreutils >/dev/null
while read -r id; do
  [[ -f \"\${id}_riscv64.s\" ]] || { echo \"FAIL riscv64 \$id\"; continue; }
  if ! riscv64-linux-gnu-gcc -static \"\${id}_riscv64.s\" -o \"\${id}_riscv64\" -lm 2>/dev/null; then echo \"FAIL riscv64 \$id\"; continue; fi
  rc=0; timeout $TO qemu-riscv64-static \"./\${id}_riscv64\" >/dev/null 2>&1 || rc=\$?
  if [[ \$rc -eq 0 ]]; then echo \"PASS riscv64 \$id\"
  elif [[ \$rc -eq 124 ]]; then echo \"TIMEOUT riscv64 \$id\"
  else echo \"FAIL riscv64 \$id\"; fi
done < ids.txt
"
}

case "$ARCH" in
  i686) run_i686 ;;
  riscv64) run_riscv ;;
  both) run_i686; echo "---"; run_riscv ;;
  *) echo "usage: $0 [workdir] [i686|riscv64|both] [timeout]" >&2; exit 1 ;;
esac
