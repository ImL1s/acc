# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| **C3** multiarch | **PASS** 40/40 (held; re-run after language churn) |
| **C5** double-run | **PASS** 207/207 identical (held; re-run after language churn) |
| C4 clean-room | held (wrapper refuses gcc on .c) |
| **C2** | **PROGRESS** — SQLite amalgamation smoke PASS; no testfixture |
| **C1** | **PROGRESS** — kbuild prepare headers green; real kernel .c failing |

### C1 Linux 6.9 (Docker, SCRATCH evidence)

**prepare / kbuild headers (fresh):**
- `scripts/mod/empty.o` OK
- `scripts/mod/devicetable-offsets.h` OK
- `include/generated/bounds.h` OK (`NR_PAGEFLAGS`, `MAX_NR_ZONES`, …)
- `include/generated/asm-offsets.h` OK (PT_*, CPUINFO_*, …) — some soft-zero offsets remain
- `usr/built-in.a` AR OK
- `scripts/checksyscalls.sh` CALL OK

**Blocked on real kernel C (post-prepare):**
- `init/main.c` / `init/do_mounts.c`: `ERROR: unterminated macro args` (preprocessor)
- `arch/x86/entry/vdso/vma.c`: `ERROR: -> on non-pointer Int` (codegen typeof/layout)
- **no** `bzImage` / QEMU boot

### C2 SQLite
- amalgamation API smoke PASS; official testfixture / Redis **not run**

### blocked_reason
C1: past prepare headers; first real kernel TUs fail (PP macro args + pointer typeof).  
C2: full SQLite testfixture / Redis not executed.  
**Goal NOT complete.**
