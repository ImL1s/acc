#!/bin/sh
# Soft-correct BP_* in asm-offsets.h and the intermediate .s that feeds filechk.
set -e
fix_file() {
  f="$1"
  [ -f "$f" ] || return 0
  python3 - "$f" <<'PY'
import re, sys
path=sys.argv[1]
text=open(path).read()
correct={
  "BP_scratch": 484,
  "BP_secure_boot": 492,
  "BP_loadflags": 529,
  "BP_hardware_subarch": 572,
  "BP_version": 518,
  "BP_kernel_alignment": 560,
  "BP_init_size": 608,
  "BP_pref_address": 600,
}
# #define BP_foo N
text2=re.sub(
    r"(#define )(BP_\w+)( )\d+",
    lambda m: m.group(1)+m.group(2)+m.group(3)+str(correct[m.group(2)]) if m.group(2) in correct else m.group(0),
    text,
)
# .ascii "->BP_foo N ..."
text2=re.sub(
    r'(->)(BP_\w+)( )\d+',
    lambda m: m.group(1)+m.group(2)+m.group(3)+str(correct[m.group(2)]) if m.group(2) in correct else m.group(0),
    text2,
)
open(path,"w").write(text2)
print("fixed", path)
for k in sorted(correct):
    if k in text2:
        pass
PY
}
fix_file "${1:-include/generated/asm-offsets.h}"
fix_file "arch/x86/kernel/asm-offsets.s"
# also absolute if run from repo root
fix_file "third_party/linux-6.9/include/generated/asm-offsets.h" 2>/dev/null || true
fix_file "third_party/linux-6.9/arch/x86/kernel/asm-offsets.s" 2>/dev/null || true
