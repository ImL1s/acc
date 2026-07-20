# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| **C3** multiarch | **PASS** 40/40 (held; re-run after language churn) |
| **C5** double-run | **PASS** 207/207 identical (held) |
| C4 clean-room | held |
| **C2** | SQLite amalgamation smoke PASS; no testfixture |
| **C1** | **PROGRESS** — ~165 kernel .o on amd64; no bzImage/QEMU |

### C1 Linux 6.9 (Docker `ggcc-linux-amd64`)

**Green (evidence):**
- prepare0 rebuildable (bounds/asm-offsets/devicetable-offsets)
- init: main.o, do_mounts.o, calibrate.o, noinitramfs.o
- vdso: vma.o
- mm: mempool.o, filemap.o
- lib: string.o, vsprintf.o, hexdump.o (+ many more)
- drivers/base: core.o + built-in.a
- ~165 non-tools `.o` objects mid-tree

**Language fixes landed this session (commits):**
- enum IntLit enumerators after PP (`1=1`)
- soft `->`/member on Int/Void/void*
- `__int128` + `__SIZEOF_INT128__` + arch predefs (`__x86_64__`)
- Func/Global symbol dedupe
- `__restrict__` pointer quals
- `__builtin_va_arg` / offsetof(`field[n]`)
- indirect calls >6 SysV args
- kernel `__user`/`__rcu`/… empty markers

**Still failing (sample):**
- fs/namei.o, fs/exec.o — expected type on huge PP lines
- kernel/sched/*.o — expected type
- arch/x86/entry/vdso/extable.o, events/*.o
- residual asm local labels (`1:`) on some TUs
- **no** bzImage / QEMU

### blocked_reason
C1: dozens more TUs after ~165 objects; no boot.  
C2: testfixture/Redis not run.  
**Goal NOT complete.**
