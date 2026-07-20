# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| **C3** multiarch | **PASS** 40/40 (held; re-run after language churn) |
| **C5** double-run | **PASS** 207/207 identical (held) |
| C4 clean-room | held |
| **C2** | SQLite amalgamation smoke PASS; no testfixture |
| **C1** | **PROGRESS** — prepare + many TUs green on amd64; no bzImage yet |

### C1 Linux 6.9 (Docker `--platform linux/amd64`, image `ggcc-linux-amd64`)

**Green (recent):**
- prepare0 (bounds/asm-offsets/devicetable-offsets) rebuildable
- `init/main.o`, `init/do_mounts.o`, `arch/x86/entry/vdso/vma.o`
- `mm/mempool.o`, `drivers/base/core.o`
- enum IntLit enumerators (`SOCK_STREAM` → `1=1` after PP)
- soft `->` / member on incomplete types (Int/Void/void*)
- `__int128` / `__SIZEOF_INT128__` + arch macros (`__x86_64__` for -m x86_64)
- Func/Global symbol dedupe (no double `.globl`)
- array designators: enum index + GNU `[lo ... hi]`

**Still failing (fail-drive):**
- various TUs: `expected type`, lvalue, too many indirect args, etc.
- **no** `bzImage` / QEMU boot yet

**Platform:** Host Docker default is aarch64; use `ggcc-linux-amd64` for x86_64 kernel.

### blocked_reason
C1: more kernel TUs after do_mounts/vdso; no bootable image.  
C2: testfixture/Redis not run.  
**Goal NOT complete.**
