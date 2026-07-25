#!/bin/bash
# One-shot: finish linking vmlinux in persistent x86 tree (inside ggcc-linux-amd64).
set -uo pipefail
cd /scratch/linux-x86-build
export ACC="${ACC:-/work/target-linux/release/acc}"
export ACC_ARCH=x86_64 ACC_TARGET_OS=linux
export ACC_PARSE_ALL_BODIES=1 ACC_SOFT_SKIP_BODIES=0 ACC_ALLOW_SOFT_SYSCC=0
export ACC_SOFT_FREESTANDING=0 ACC_KERNEL_FREESTANDING=1
WRAP=/work/harness/docker/acc_cc_wrapper.sh
chmod +x "$WRAP" /work/harness/docker/fix_x86_link_dups.sh
cp -f /work/harness/docker/ggcc_link_asm_x86_64.S lib/ggcc_link_asm_x86_64.S 2>/dev/null || true
cp -f /work/harness/docker/ggcc_link_stubs_x86_64.c lib/ggcc_link_stubs_x86_64.c 2>/dev/null || true
if [[ -f lib/ggcc_link_asm_x86_64.S ]] && ! grep -q ggcc_link_asm_x86_64.o lib/Makefile; then
  { echo "obj-y += ggcc_link_asm_x86_64.o"; cat lib/Makefile; } > /tmp/libmk.$$
  mv /tmp/libmk.$$ lib/Makefile
fi
/work/harness/docker/fix_x86_link_dups.sh /scratch/linux-x86-build
echo "=== start make vmlinux $(date -u +%Y-%m-%dT%H:%M:%SZ) ===" | tee /scratch/c1_x86_vmlinux_make.log
make ARCH=x86 CC="$WRAP" HOSTCC=gcc HOSTCXX=g++ -j4 vmlinux >> /scratch/c1_x86_vmlinux_make.log 2>&1
ec=$?
echo "MAKE_EC=$ec" | tee -a /scratch/c1_x86_vmlinux_make.log
ls -la vmlinux >> /scratch/c1_x86_vmlinux_make.log 2>&1 || true
if [[ $ec -ne 0 ]] && grep -q "Unexpected GOT" /scratch/c1_x86_vmlinux_make.log; then
  echo "=== GOT scan ===" | tee -a /scratch/c1_x86_vmlinux_make.log
  find . -name "*.o" ! -path "./scripts/*" ! -path "./tools/*" -print0 2>/dev/null \
    | xargs -0 -n1 sh -c 'readelf -r "$1" 2>/dev/null | grep -q GOT && echo GOT_IN $1' _ \
    2>/dev/null | head -50 | tee -a /scratch/c1_x86_vmlinux_make.log
fi
tail -80 /scratch/c1_x86_vmlinux_make.log
exit $ec
