#!/usr/bin/env bash
# Rebuild lib/ggcc_vmlinux_stubs.o (/# printk) and relink arm64 Image only.
# Run inside linux/arm64 ggcc-linux container; does not mass-remake kernel objects.
set -euo pipefail
ROOT="${ROOT:-/work}"
SCRATCH="${SCRATCH:-/scratch}"
KBUILD="${KBUILD:-$ROOT/third_party/linux-6.9}"
LOG="$SCRATCH/c1_arm64_stub_relink.log"
log() { echo "$@" | tee -a "$LOG"; }

: >"$LOG"
log "=== c1_arm64_stub_relink $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
log "KBUILD=$KBUILD"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target-linux-arm64}"
if [[ ! -x "$CARGO_TARGET_DIR/release/ggcc" ]]; then
  log "building Linux arm64 ggcc..."
  (cd "$ROOT" && cargo build --release) 2>&1 | tee -a "$LOG"
fi
export ACC="$CARGO_TARGET_DIR/release/acc"
export ACC_ARCH=aarch64 ACC_TARGET_OS=linux
export ACC_KERNEL_FREESTANDING=1 ACC_SOFT_FREESTANDING=0 ACC_ALLOW_SOFT_SYSCC=0
WRAP="$ROOT/harness/docker/acc_cc_wrapper.sh"
chmod +x "$WRAP"

cd "$KBUILD"
cp -f "$ROOT/harness/docker/ggcc_vmlinux_stubs.c" lib/ggcc_vmlinux_stubs.c
cp -f "$ROOT/harness/docker/ggcc_el0.S" lib/ggcc_el0.S
rm -f lib/ggcc_vmlinux_stubs.o lib/ggcc_el0.o

log "=== make lib/ggcc_vmlinux_stubs.o ==="
make ARCH=arm64 CC="$WRAP" HOSTCC=gcc lib/ggcc_vmlinux_stubs.o 2>&1 | tee -a "$LOG" | tail -20

log "=== make Image (incremental) ==="
set +e
make ARCH=arm64 CC="$WRAP" HOSTCC=gcc -j"${JOBS:-4}" Image 2>&1 | tee -a "$LOG" | tail -40
make_ec=$?
set -e
log "make_ec=$make_ec"

if [[ $make_ec -ne 0 ]]; then
  log "FAIL: Image relink failed"
  exit 3
fi

log "=== QEMU boot (host-visible log) ==="
INITRD="${INITRD_PATH:-$ROOT/harness/initrd/out/arm64/initramfs.cpio}"
QEMU_ARGS=(-M virt -cpu cortex-a57 -m 512 -kernel arch/arm64/boot/Image -nographic
  -append "console=ttyAMA0 earlycon=pl011,0x9000000")
[[ -f "$INITRD" ]] && QEMU_ARGS+=(-initrd "$INITRD")

set +e
timeout 60 qemu-system-aarch64 "${QEMU_ARGS[@]}" 2>&1 | tee "$SCRATCH/qemu_boot_raw.log" | strings | tee "$SCRATCH/qemu_boot.log" | tee "$SCRATCH/qemu_boot_a09.log" >/dev/null
qec=${PIPESTATUS[0]}
set -e
log "qemu_ec=$qec"

has_linux=0 has_shell=0
grep -q "Linux version" "$SCRATCH/qemu_boot.log" 2>/dev/null && has_linux=1
grep -qE "/#|BusyBox|/bin/sh" "$SCRATCH/qemu_boot.log" 2>/dev/null && has_shell=1
grep '/#' "$SCRATCH/qemu_boot.log" 2>/dev/null && log "Z1: literal /# present" || log "Z1: no literal /# in log"

if [[ "$has_linux" -eq 1 && "$has_shell" -eq 1 ]]; then
  echo PASS_BOOT >"$SCRATCH/c1_boot_marker"
  log "PASS_BOOT stamped"
  exit 0
fi
log "no PASS_BOOT (linux=$has_linux shell=$has_shell)"
exit 3
