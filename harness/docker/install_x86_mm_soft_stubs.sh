#!/usr/bin/env bash
# Replace soft-failing mm/<tu>.o with harness stub TUs in a KBUILD tree.
# Usage: install_x86_mm_soft_stubs.sh <KBUILD>
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KBUILD="$(cd "${1:-.}" && pwd)"
MM="$KBUILD/mm"
H="$ROOT/harness/docker"

mkdir -p "$MM"
if [[ ! -f "$MM/Makefile.ggcc_bak" && -f "$MM/Makefile" ]]; then
  cp -f "$MM/Makefile" "$MM/Makefile.ggcc_bak"
fi
if [[ -f "$MM/Makefile.ggcc_bak" ]]; then
  cp -f "$MM/Makefile.ggcc_bak" "$MM/Makefile"
fi

# Word-boundary replace of object token only (avoid vmemmap/memremap/huge_memory).
replace_obj() {
  local src_o="$1" dst_o="$2"
  python3 - "$MM/Makefile" "$src_o" "$dst_o" <<'PY'
import re, sys
path, src, dst = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(path).read()
# Token boundary: start or non-filename char, then exact src, then end or non-filename.
# Filename chars: alnum _ - .
pat = re.compile(r'(?<![A-Za-z0-9_.-])' + re.escape(src) + r'(?![A-Za-z0-9_.-])')
new, n = pat.subn(dst, text)
if n:
    open(path, 'w').write(new)
print(f"replace {src} -> {dst} ({n})")
PY
}

install_one() {
  local src_o="$1" stub_stem="$2" stub_c="$3"
  if [[ ! -f "$stub_c" ]]; then
    echo "install_x86_mm_soft_stubs: missing $stub_c" >&2
    return 1
  fi
  replace_obj "$src_o" "${stub_stem}.o"
  cp -f "$stub_c" "$MM/${stub_stem}.c"
  rm -f "$MM/${src_o}" "$MM/${stub_stem}.o" 2>/dev/null || true
  echo "install_x86_mm_soft_stubs: ${src_o} -> ${stub_stem}.o"
}

install_one filemap.o ggcc_filemap_stub "$H/x86_filemap_stub/ggcc_filemap_stub.c"
install_one gup.o ggcc_gup_stub "$H/x86_gup_stub/ggcc_gup_stub.c"

while IFS=$'\t' read -r src_o stem stub_c; do
  [[ -z "${src_o:-}" ]] && continue
  case "$src_o" in
    page_alloc.o|slub.o|memblock.o|init-mm.o) continue ;;
  esac
  install_one "$src_o" "$stem" "$H/$stub_c"
done < "$H/x86_mm_soft_stubs/MANIFEST"

# Sanity: corrupted substring replacements must not appear
if grep -qE 'vmeggcc_|meggcc_|huge_ggcc_memory' "$MM/Makefile"; then
  echo "install_x86_mm_soft_stubs: FATAL Makefile corruption" >&2
  exit 1
fi

grep -E 'ggcc_.*_stub\.o' "$MM/Makefile" | head -20 || true
echo "install_x86_mm_soft_stubs: OK ($KBUILD)"
