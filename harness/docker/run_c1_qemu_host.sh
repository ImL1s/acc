#!/usr/bin/env bash
# Host-side QEMU serial capture (no Docker). arm64 only on macOS with qemu-system-aarch64.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="${SCRATCH:-$ROOT/scratch}"
KERNEL="${KERNEL_IMAGE:-$ROOT/third_party/linux-6.9/arch/arm64/boot/Image}"
INITRD="${INITRD_PATH:-$ROOT/harness/initrd/out/arm64/initramfs.cpio}"
LOG="$SCRATCH/qemu_boot.log"
A09="$SCRATCH/qemu_boot_a09.log"

[[ -f "$KERNEL" ]] || { echo "missing kernel: $KERNEL" >&2; exit 2; }
mkdir -p "$SCRATCH"

ARGS=(-M virt -cpu cortex-a57 -m 512 -kernel "$KERNEL" -nographic
  -append "console=ttyAMA0 earlycon=pl011,0x9000000")
[[ -f "$INITRD" ]] && ARGS+=(-initrd "$INITRD")

echo "QEMU arm64 -> $LOG (strings -n 2 filtered)"
set +e
timeout 60 qemu-system-aarch64 "${ARGS[@]}" 2>&1 | tee "$SCRATCH/qemu_boot_raw.log" | strings -n 2 | tee "$LOG" | tee "$A09" >/dev/null
qec=${PIPESTATUS[0]}
set -e
echo "qemu_ec=$qec"
grep -E 'Linux version|BusyBox|/bin/sh|/#' "$LOG" | head -20 || true
grep -q '/#' "$LOG" && echo "Z1: /# present" || echo "Z1: /# absent"
