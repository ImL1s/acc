#!/usr/bin/env bash
# Install ggcc freestanding PVH boot path into an x86_64 KBUILD tree.
# enlighten.c is compiled with HOSTCC=gcc (not ggcc wrapper) per C1 policy carve-out
# for Xen/PVH glue that ggcc cannot compile yet.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KBUILD="$(cd "${1:-.}" && pwd)"
LIB="$KBUILD/lib"

mkdir -p "$LIB"

# sorttable + BUILDTIME_TABLE_SORT off (idempotent with ensure_x86_pvh_soft.sh)
if [[ -x "$ROOT/harness/docker/ensure_x86_pvh_soft.sh" ]]; then
  "$ROOT/harness/docker/ensure_x86_pvh_soft.sh" "$KBUILD" || true
fi

cp -f "$ROOT/harness/docker/ggcc_pvh_note.S" "$LIB/"
cp -f "$ROOT/harness/docker/ggcc_pvh_head.S" "$LIB/"
cp -f "$ROOT/harness/docker/ggcc_pvh_enlighten.c" "$LIB/"

# obj-y injection
python3 - "$LIB/Makefile" <<'PY'
from pathlib import Path
import re, sys
path = Path(sys.argv[1])
text = path.read_text() if path.is_file() else ""
for obj in ("ggcc_pvh_note.o", "ggcc_pvh_head.o", "ggcc_pvh_enlighten.o"):
    pat = re.compile(rf"^[ \t]*obj-y[ \t]*\+=[ \t]*{re.escape(obj)}[ \t]*\n", re.M)
    text = pat.sub("", text)
    if f"obj-y += {obj}" not in text:
        text = f"obj-y += {obj}\n" + text
path.write_text(text)
print(f"install_x86_pvh_boot: lib/Makefile obj-y += pvh note/head/enlighten")
PY

# Empty platform pvh enlighten (upstream xen_* breaks under ggcc link)
mkdir -p "$KBUILD/arch/x86/platform/pvh"
printf '%s\n' '# ggcc: skip upstream PVH enlighten (freestanding lib/ path)' >"$KBUILD/arch/x86/platform/pvh/Makefile"

CFG="$KBUILD/.config"
if [[ -f "$CFG" ]]; then
  for opt in MODULES UNWINDER_ORC STACK_VALIDATION BUILDTIME_TABLE_SORT; do
    sed -i "/^CONFIG_${opt}=y/d" "$CFG" 2>/dev/null || true
    sed -i "/^CONFIG_${opt}=m/d" "$CFG" 2>/dev/null || true
    grep -q "CONFIG_${opt} is not set" "$CFG" 2>/dev/null || \
      echo "# CONFIG_${opt} is not set" >>"$CFG"
  done
  sed -i '/^CONFIG_PVH=/d' "$CFG" 2>/dev/null || true
  grep -q '^CONFIG_PVH=y' "$CFG" 2>/dev/null || echo "CONFIG_PVH=y" >>"$CFG"
  grep -q '^CONFIG_64BIT=y' "$CFG" || echo "CONFIG_64BIT=y" >>"$CFG"
fi

echo "install_x86_pvh_boot: OK ($KBUILD)"
