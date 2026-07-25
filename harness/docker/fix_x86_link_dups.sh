#!/usr/bin/env bash
# Weaken freestanding-keeper duplicates so vmlinux.o can link.
# Soft ACC_KERNEL_FREESTANDING may emit wait_for_initramfs into multiple TUs
# (init/main.o + kernel/umh.o). Prefer init/main.o; weaken the rest.
set -euo pipefail
KBUILD="$(cd "${1:-.}" && pwd)"
cd "$KBUILD"

OBJCOPY="${OBJCOPY:-objcopy}"
weaken() {
  local obj="$1" sym="$2"
  if [[ -f "$obj" ]] && nm --defined-only "$obj" 2>/dev/null | grep -q " T ${sym}$"; then
    "$OBJCOPY" --weaken-symbol="$sym" "$obj"
    echo "fix_x86_link_dups: weakened $sym in $obj"
  fi
}

weaken kernel/umh.o wait_for_initramfs

rm -f vmlinux.o .vmlinux.o.cmd vmlinux arch/x86/boot/bzImage 2>/dev/null || true
echo "fix_x86_link_dups: OK ($KBUILD)"
