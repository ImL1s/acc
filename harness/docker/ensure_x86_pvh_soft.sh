#!/usr/bin/env bash
# B07 soft-PVH packaging for x86 C1 (B01 runs this against linux-x86-build).
#
# Restores the linkable soft Xen PHYS32 ELF note path after a broken attempt to
# force-link real arch/x86/platform/pvh head/enlighten (xen_* undefs).
#
# Usage:
#   harness/docker/ensure_x86_pvh_soft.sh [KBUILD_DIR]
# Default KBUILD_DIR: $SCRATCH/linux-x86-build or ./scratch/linux-x86-build
#
# Idempotent. Does not run make or QEMU (B01 owns boot).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
NOTE_SRC="$ROOT/harness/docker/ggcc_pvh_note.S"

if [[ $# -ge 1 && -n "${1:-}" ]]; then
  KBUILD="$(cd "$1" && pwd)"
elif [[ -n "${SCRATCH:-}" && -d "$SCRATCH/linux-x86-build" ]]; then
  KBUILD="$(cd "$SCRATCH/linux-x86-build" && pwd)"
elif [[ -d "$ROOT/scratch/linux-x86-build" ]]; then
  KBUILD="$(cd "$ROOT/scratch/linux-x86-build" && pwd)"
else
  echo "ensure_x86_pvh_soft: KBUILD dir missing (pass path or set SCRATCH)" >&2
  exit 2
fi

if [[ ! -f "$NOTE_SRC" ]]; then
  echo "ensure_x86_pvh_soft: missing $NOTE_SRC" >&2
  exit 2
fi
if [[ ! -f "$KBUILD/Makefile" ]]; then
  echo "ensure_x86_pvh_soft: not a kernel tree: $KBUILD" >&2
  exit 2
fi

LIB="$KBUILD/lib"
MK="$LIB/Makefile"
LINK_SH="$KBUILD/scripts/link-vmlinux.sh"

mkdir -p "$LIB"
cp -f "$NOTE_SRC" "$LIB/ggcc_pvh_note.S"
echo "ensure_x86_pvh_soft: installed $LIB/ggcc_pvh_note.S"

# Drop broken real-PVH objects (if a prior attempt copied them into lib/).
rm -f "$LIB/ggcc_pvh_head.o" "$LIB/ggcc_pvh_enlighten.o" \
  "$LIB/ggcc_pvh_head.S" "$LIB/ggcc_pvh_enlighten.c" \
  "$LIB/ggcc_pvh_head.c" "$LIB/head.o" 2>/dev/null || true
# Also drop stale soft-note object so make rebuilds from the fresh .S
rm -f "$LIB/ggcc_pvh_note.o" 2>/dev/null || true

if [[ ! -f "$MK" ]]; then
  echo "ensure_x86_pvh_soft: missing $MK" >&2
  exit 2
fi

# Strip prior soft/real PVH obj-y lines, then inject soft note only.
python3 - "$MK" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text()
# Remove any ggcc_pvh_* obj-y lines (note/head/enlighten)
pat = re.compile(
    r"^[ \t]*obj-y[ \t]*\+=[ \t]*ggcc_pvh_(?:note|head|enlighten)\.o[ \t]*\n",
    re.M,
)
text2 = pat.sub("", text)
# Also strip accidental copies of platform pvh objects if someone added them
pat2 = re.compile(
    r"^[ \t]*obj-y[ \t]*\+=[ \t]*(?:head|enlighten)\.o[ \t]*#.*pvh.*\n",
    re.M | re.I,
)
text2 = pat2.sub("", text2)
if "obj-y += ggcc_pvh_note.o" not in text2:
    text2 = "obj-y += ggcc_pvh_note.o\n" + text2
path.write_text(text2)
print("ensure_x86_pvh_soft: lib/Makefile → obj-y += ggcc_pvh_note.o (head/enlighten stripped)")
PY

# sorttable segfaults on ggcc-built vmlinux; skip it (idempotent).
if [[ -f "$LINK_SH" ]]; then
  if grep -q 'ggcc: skip sorttable' "$LINK_SH" 2>/dev/null; then
    echo "ensure_x86_pvh_soft: sorttable already skipped in link-vmlinux.sh"
  elif grep -q 'is_enabled CONFIG_BUILDTIME_TABLE_SORT' "$LINK_SH" 2>/dev/null; then
    # Prefer false && guard so the SORTTAB block never runs.
    sed -i.bak \
      's/^if is_enabled CONFIG_BUILDTIME_TABLE_SORT; then$/if false \&\& is_enabled CONFIG_BUILDTIME_TABLE_SORT; then # ggcc: skip sorttable/' \
      "$LINK_SH"
    rm -f "$LINK_SH.bak"
    if grep -q 'ggcc: skip sorttable' "$LINK_SH"; then
      echo "ensure_x86_pvh_soft: patched scripts/link-vmlinux.sh (skip sorttable)"
    else
      echo "ensure_x86_pvh_soft: WARN could not patch sorttable guard" >&2
    fi
  else
    echo "ensure_x86_pvh_soft: WARN no BUILDTIME_TABLE_SORT block in link-vmlinux.sh" >&2
  fi
fi

# Also clear Kconfig bit so future regenerations stay quiet.
CFG="$KBUILD/.config"
if [[ -f "$CFG" ]]; then
  if grep -q '^CONFIG_BUILDTIME_TABLE_SORT=y' "$CFG" 2>/dev/null; then
    sed -i.bak '/^CONFIG_BUILDTIME_TABLE_SORT=/d' "$CFG"
    rm -f "$CFG.bak"
    grep -q 'CONFIG_BUILDTIME_TABLE_SORT is not set' "$CFG" 2>/dev/null \
      || echo '# CONFIG_BUILDTIME_TABLE_SORT is not set' >> "$CFG"
    echo "ensure_x86_pvh_soft: disabled CONFIG_BUILDTIME_TABLE_SORT in .config"
  else
    echo "ensure_x86_pvh_soft: CONFIG_BUILDTIME_TABLE_SORT already off/unset"
  fi
fi

echo "ensure_x86_pvh_soft: OK (KBUILD=$KBUILD)"
echo "ensure_x86_pvh_soft: next (B01): remake lib/ggcc_pvh_note.o + vmlinux, then QEMU -kernel vmlinux"
