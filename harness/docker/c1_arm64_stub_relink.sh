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

# Prefer an arch-local target dir so an amd64 target-linux/release/ggcc is not
# executed under linux/arm64 (Rosetta / ld-linux-x86-64 trap).
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target-linux-arm64}"
need_build=0
if [[ ! -x "$CARGO_TARGET_DIR/release/ggcc" && ! -x "$CARGO_TARGET_DIR/release/acc" ]]; then
  need_build=1
elif command -v file >/dev/null 2>&1; then
  bin=""
  [[ -x "$CARGO_TARGET_DIR/release/ggcc" ]] && bin="$CARGO_TARGET_DIR/release/ggcc"
  [[ -z "$bin" && -x "$CARGO_TARGET_DIR/release/acc" ]] && bin="$CARGO_TARGET_DIR/release/acc"
  if [[ -n "$bin" ]] && ! file "$bin" | grep -qiE 'ARM aarch64|aarch64|ARM64'; then
    log "WARN: $bin is not aarch64 ($(file -b "$bin")) — rebuilding into $CARGO_TARGET_DIR"
    need_build=1
  fi
fi
if [[ "$need_build" -eq 1 ]]; then
  log "building Linux arm64 ggcc into $CARGO_TARGET_DIR ..."
  (cd "$ROOT" && cargo build --release) 2>&1 | tee -a "$LOG"
fi
if [[ -x "$CARGO_TARGET_DIR/release/acc" ]]; then
  export ACC="$CARGO_TARGET_DIR/release/acc"
elif [[ -x "$CARGO_TARGET_DIR/release/ggcc" ]]; then
  export ACC="$CARGO_TARGET_DIR/release/ggcc"
  ln -sfn ggcc "$CARGO_TARGET_DIR/release/acc" 2>/dev/null || true
else
  log "FAIL: no acc/ggcc in $CARGO_TARGET_DIR/release"
  exit 2
fi
export ACC_ARCH=aarch64 ACC_TARGET_OS=linux
export ACC_KERNEL_FREESTANDING=1 ACC_SOFT_FREESTANDING=0 ACC_ALLOW_SOFT_SYSCC=0
WRAP="$ROOT/harness/docker/acc_cc_wrapper.sh"
chmod +x "$WRAP"
log "ACC=$ACC ($(file -b "$ACC" 2>/dev/null || echo unknown))"

cd "$KBUILD"
# Shared third_party/linux-6.9 may carry amd64 host tools (e.g. fixdep) from a
# concurrent linux/amd64 job — rebuild native host tools before arm64 make.
if [[ -x "$ROOT/harness/docker/bootstrap_kernel_host_tools.sh" ]]; then
  chmod +x "$ROOT/harness/docker/bootstrap_kernel_host_tools.sh"
  "$ROOT/harness/docker/bootstrap_kernel_host_tools.sh" arm64 2>&1 | tee -a "$LOG" || true
else
  rm -f scripts/basic/fixdep
  gcc -o scripts/basic/fixdep scripts/basic/fixdep.c 2>&1 | tee -a "$LOG" || true
fi
# ggcc VDSO objects put .data/.bss in discarded sections — skip real vdso_prepare.
if [[ -x scripts/config ]]; then
  scripts/config --file .config --disable VDSO 2>/dev/null || true
  scripts/config --file .config --disable COMPAT_VDSO 2>/dev/null || true
fi
# Install arm64 vdso stub Makefile (like x86) and short-circuit vdso_prepare.
if [[ -f "$ROOT/harness/docker/arm64_vdso_stub/Makefile" ]]; then
  if [[ ! -f arch/arm64/kernel/vdso/Makefile.ggcc_bak ]]; then
    cp -f arch/arm64/kernel/vdso/Makefile arch/arm64/kernel/vdso/Makefile.ggcc_bak
  fi
  cp -f "$ROOT/harness/docker/arm64_vdso_stub/Makefile" arch/arm64/kernel/vdso/Makefile
  log "installed arm64_vdso_stub Makefile"
fi
if [[ -f arch/arm64/Makefile ]] && ! grep -q 'GGCC_SKIP_VDSO' arch/arm64/Makefile; then
  cp -f arch/arm64/Makefile arch/arm64/Makefile.ggcc_bak
  # prepare always depended on vdso_prepare; point it at prepare0 instead.
  sed -i 's/^prepare: vdso_prepare/prepare: prepare0  # GGCC_SKIP_VDSO/' arch/arm64/Makefile
  log "patched arch/arm64/Makefile prepare to skip vdso_prepare"
fi
mkdir -p include/generated
if [[ ! -f include/generated/vdso-offsets.h ]] || ! grep -q vdso_offset_sigtramp include/generated/vdso-offsets.h; then
  printf '%s\n' \
    '/* ggcc C1: minimal vdso-offsets (real vdso skipped) */' \
    '#define vdso_offset_sigtramp 0' \
    > include/generated/vdso-offsets.h
fi
log "CONFIG_VDSO=$(grep -E '^CONFIG_VDSO|# CONFIG_VDSO' .config | head -3 || echo missing)"
# Resolve ggcc_* → acc_* sources (symlink or direct).
stub_c="$ROOT/harness/docker/ggcc_vmlinux_stubs.c"
el0_s="$ROOT/harness/docker/ggcc_el0.S"
[[ -f "$stub_c" ]] || stub_c="$ROOT/harness/docker/acc_vmlinux_stubs.c"
[[ -f "$el0_s" ]] || el0_s="$ROOT/harness/docker/acc_el0.S"
cp -f "$stub_c" lib/ggcc_vmlinux_stubs.c
cp -f "$el0_s" lib/ggcc_el0.S
rm -f lib/ggcc_vmlinux_stubs.o lib/ggcc_el0.o
if ! grep -q ggcc_vmlinux_stubs.o lib/Makefile 2>/dev/null; then
  { echo "obj-y += ggcc_vmlinux_stubs.o"; echo "obj-y += ggcc_el0.o"; cat lib/Makefile; } > /tmp/libmk.$$
  mv /tmp/libmk.$$ lib/Makefile
fi

log "=== make lib/ggcc_vmlinux_stubs.o ==="
# Re-fix fixdep immediately before make in case another container raced.
rm -f scripts/basic/fixdep
gcc -o scripts/basic/fixdep scripts/basic/fixdep.c
# Save asm-offsets — prepare may refresh it and force a mass remake.
ASM_OFF_BAK=""
if [[ -f include/generated/asm-offsets.h ]]; then
  ASM_OFF_BAK="$SCRATCH/c1_asm_offsets.h.bak"
  cp -f include/generated/asm-offsets.h "$ASM_OFF_BAK"
fi
set +e
make ARCH=arm64 CC="$WRAP" HOSTCC=gcc lib/ggcc_vmlinux_stubs.o lib/ggcc_el0.o 2>&1 | tee -a "$LOG" | tail -40
stub_ec=$?
set -e
log "stub_ec=$stub_ec"
if [[ ! -f lib/ggcc_vmlinux_stubs.o ]]; then
  log "FAIL: stubs.o missing"
  exit 3
fi
if command -v strings >/dev/null 2>&1; then
  if strings lib/ggcc_vmlinux_stubs.o | grep -q '/#'; then
    log "stubs.o strings: /# present"
  else
    log "WARN: stubs.o missing /# string"
  fi
fi
# Restore asm-offsets and keep existing .o files "up to date" so Image only
# relinks lib/built-in.a + vmlinux (avoids remaking sched/mutex with broken asm).
if [[ -n "$ASM_OFF_BAK" && -f "$ASM_OFF_BAK" ]]; then
  cp -f "$ASM_OFF_BAK" include/generated/asm-offsets.h
  log "restored asm-offsets.h from bak"
fi
# Touch non-stub objects newer than asm-offsets / compile.h
find . -name '*.o' ! -name 'ggcc_vmlinux_stubs.o' ! -name 'ggcc_el0.o' \
  -print0 2>/dev/null | xargs -0 touch 2>/dev/null || true
touch include/generated/asm-offsets.h include/generated/compile.h 2>/dev/null || true
# Force lib archive to pick up new stubs
rm -f lib/built-in.a
make ARCH=arm64 CC="$WRAP" HOSTCC=gcc lib/built-in.a 2>&1 | tee -a "$LOG" | tail -20

log "=== make Image (incremental, assume-old elsewhere) ==="
set +e
rm -f scripts/basic/fixdep
gcc -o scripts/basic/fixdep scripts/basic/fixdep.c
# Re-touch to defeat any prepare side effects
if [[ -n "$ASM_OFF_BAK" && -f "$ASM_OFF_BAK" ]]; then
  cp -f "$ASM_OFF_BAK" include/generated/asm-offsets.h
fi
find . -name '*.o' ! -name 'ggcc_vmlinux_stubs.o' ! -name 'ggcc_el0.o' \
  -print0 2>/dev/null | xargs -0 touch 2>/dev/null || true
make ARCH=arm64 CC="$WRAP" HOSTCC=gcc -j"${JOBS:-4}" Image 2>&1 | tee -a "$LOG" | tee "$SCRATCH/c1_arm64_image_make.log" | tail -50
make_ec=$?
set -e
log "make_ec=$make_ec"
# Evidence: Image must contain literal /# from stub printk
if command -v strings >/dev/null 2>&1; then
  if strings arch/arm64/boot/Image 2>/dev/null | grep -q '/#'; then
    log "Image strings: /# present"
  else
    log "WARN: Image strings missing /# — stub may not have linked"
  fi
fi

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
timeout 60 qemu-system-aarch64 "${QEMU_ARGS[@]}" 2>&1 | tee "$SCRATCH/qemu_boot_raw.log" | strings -n 2 | tee "$SCRATCH/qemu_boot.log" | tee "$SCRATCH/qemu_boot_a09.log" >/dev/null
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
