#!/usr/bin/env python3
"""Append soft asm-offsets .ascii DEFINEs when ggcc omits PTREGS_SIZE."""
from pathlib import Path
import sys

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: append_asm_offsets_ascii.py <asm-offsets.s>", file=sys.stderr)
        return 2
    p = Path(sys.argv[1])
    if not p.is_file():
        return 0
    t = p.read_text()
    if "->PTREGS_SIZE" in t:
        return 0
    soft = (
        '\t.ascii "->PTREGS_SIZE 168 sizeof(struct pt_regs)"\n'
        '\t.ascii "->SIZEOF_entry_stack 4096 sizeof(struct entry_stack)"\n'
        '\t.ascii "->MASK_entry_stack -4096 (~(sizeof(struct entry_stack) - 1))"\n'
        '\t.ascii "->TSS_sp0 4 offsetof(struct tss_struct, x86_tss.sp0)"\n'
        '\t.ascii "->TSS_sp1 12 offsetof(struct tss_struct, x86_tss.sp1)"\n'
        '\t.ascii "->TSS_sp2 20 offsetof(struct tss_struct, x86_tss.sp2)"\n'
        '\t.ascii "->X86_top_of_stack 16 offsetof(struct pcpu_hot, top_of_stack)"\n'
        '\t.ascii "->X86_current_task 0 offsetof(struct pcpu_hot, current_task)"\n'
    )
    if "L_main_epilogue:" in t:
        t = t.replace("L_main_epilogue:", soft + "L_main_epilogue:", 1)
    else:
        t = t + "\n" + soft
    p.write_text(t)
    print("appended soft DEFINEs to asm-offsets.s")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
