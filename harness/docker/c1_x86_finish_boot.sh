#!/bin/bash
# Finish x86 C1: ensure GOT LDS patch, link vmlinux, patch PHYS32 note, QEMU serial.
set -uo pipefail
cd /scratch/linux-x86-build
export ACC="${ACC:-/work/target-linux/release/acc}"
export ACC_ARCH=x86_64 ACC_TARGET_OS=linux
export ACC_PARSE_ALL_BODIES=1 ACC_SOFT_SKIP_BODIES=0 ACC_ALLOW_SOFT_SYSCC=0
export ACC_SOFT_FREESTANDING=0 ACC_KERNEL_FREESTANDING=1 ACC_USE_GOT=0
WRAP=/work/harness/docker/acc_cc_wrapper.sh
chmod +x "$WRAP" \
  /work/harness/docker/fix_x86_link_dups.sh \
  /work/harness/docker/fix_x86_pvh_phys32_note.sh \
  /work/harness/docker/install_x86_pvh_boot.sh

/work/harness/docker/install_x86_pvh_boot.sh /scratch/linux-x86-build || true
# Re-apply GOT neutralize if ensure wiped it (shouldn't)
if ! grep -q 'ggcc: allow soft GOT' arch/x86/kernel/vmlinux.lds.S; then
  echo "WARN: re-patch LDS GOT"; fi

cp -f /work/harness/docker/ggcc_pvh_head.S lib/ggcc_pvh_head.S
cp -f /work/harness/docker/ggcc_pvh_note.S lib/ggcc_pvh_note.S
cp -f /work/harness/docker/ggcc_pvh_enlighten.c lib/ggcc_pvh_enlighten.c
cp -f /work/harness/docker/ggcc_link_asm_x86_64.S lib/ggcc_link_asm_x86_64.S 2>/dev/null || true

rm -f lib/ggcc_pvh_head.o lib/ggcc_pvh_note.o lib/ggcc_pvh_enlighten.o \
  lib/built-in.a vmlinux.a vmlinux.o vmlinux arch/x86/kernel/vmlinux.lds
/work/harness/docker/fix_x86_link_dups.sh /scratch/linux-x86-build

echo "=== make vmlinux $(date -u +%Y-%m-%dT%H:%M:%SZ) ===" | tee /scratch/c1_x86_finish.log
make ARCH=x86 CC="$WRAP" HOSTCC=gcc HOSTCXX=g++ -j4 vmlinux >> /scratch/c1_x86_finish.log 2>&1
ec=$?
echo "MAKE_EC=$ec" | tee -a /scratch/c1_x86_finish.log
if [[ $ec -ne 0 || ! -f vmlinux ]]; then
  tail -60 /scratch/c1_x86_finish.log
  exit "$ec"
fi
ls -la vmlinux | tee -a /scratch/c1_x86_finish.log
/work/harness/docker/fix_x86_pvh_phys32_note.sh vmlinux | tee -a /scratch/c1_x86_finish.log
echo "==== notes after patch ====" | tee -a /scratch/c1_x86_finish.log
readelf -n vmlinux | head -20 | tee -a /scratch/c1_x86_finish.log

rm -f /scratch/qemu_boot_x86_64.log
timeout 90 qemu-system-x86_64 -m 512 \
  -kernel vmlinux \
  -initrd /work/harness/initrd/out/x86_64/initramfs.cpio \
  -append "console=ttyS0 earlyprintk=serial,ttyS0,115200" \
  -nographic -serial mon:stdio -no-reboot \
  > /scratch/qemu_boot_x86_64.log 2>&1 || true

python3 - <<'PY' | tee -a /scratch/c1_x86_finish.log
from pathlib import Path
b = Path("/scratch/qemu_boot_x86_64.log").read_bytes()
print(f"serial={len(b)} /#={b.count(b'/#')} BusyBox={b.count(b'BusyBox')} Linux={b.count(b'Linux version')} ggcc-pvh={b.count(b'ggcc-pvh')} SeaBIOS={b.count(b'SeaBIOS')}")
print("--- serial tail ---")
print(b.decode("utf-8", "replace")[-3000:])
has = b.count(b"/#") > 0 and (b.count(b"BusyBox") > 0 or b.count(b"Linux version") > 0)
# Strict C1: need literal /#
ok = b"/#" in b
Path("/scratch/c1_boot_marker_x86_64").write_text("PASS_BOOT\n" if ok else "FAIL_BOOT\n")
print("marker", "PASS_BOOT" if ok else "FAIL_BOOT")
PY
exit 0
