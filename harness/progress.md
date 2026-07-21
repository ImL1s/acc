# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS (held; re-run after language churn) |
| C5 double-run | PASS (held; re-run after language churn) |
| C4 clean-room | held (boot-stub i386 still open) |
| C2 | SQLite amalgamation smoke only; **no testfixture** |
| **C1** | **MAJOR PROGRESS** — **vmlinux linked (38MB)**; **no QEMU boot / bzImage** |

### C1 achievements
- ~518 kernel `.o` via ggcc wrapper in Docker amd64
- **vmlinux** + **System.map** produced (`LD vmlinux` make_ec=0)
- **0 undefined references** after soft link stubs
- **vdso64.so** linked
- **asm-offsets.h** with PTREGS_SIZE / TASK_threadsp / …
- **head64.o** exports `x86_64_start_kernel` (weak stub)

### Language / link fixes (this session)
- keep `common()` body for asm-offsets; soft-stub other non-main
- `is_extern` + file-scope extern tracking; register vars local
- enum constants as file-local (no .globl multi-def)
- soft stubs + non-static BSS as `.weak`
- do not reserve `emitted_syms` on prototypes (was swallowing stubs)
- soft-stub static empty bodies for fops/ktype
- link-time weak stubs for residual undefs; no-op objtool/sorttable

### Still red
1. **bzImage** — setup needs i386 `-m32` (ggcc is x86_64-only)
2. **QEMU boot** — ELF lacks PVH note; QEMU refuses uncompressed kernel
3. Soft-stubs ⇒ not a real bootable kernel yet even with bzImage

### blocked_reason
**C1:** vmlinux linked under ggcc; not yet QEMU-bootable (bzImage/PVH).  
**C2:** no SQLite testfixture / Redis.  
**C5:** re-run after language churn.  
**Goal NOT complete.**

Evidence: `{SCRATCH}/stage_c_kernel.log`, `vmlinux`, `System.map`, kernel make logs.
