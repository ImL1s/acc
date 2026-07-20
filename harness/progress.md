# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| **C3** multiarch | **PASS** 40/40 (held; re-run after language churn) |
| **C5** double-run | **PASS** 207/207 identical (held) |
| C4 clean-room | held |
| **C2** | SQLite amalgamation smoke PASS; no testfixture |
| **C1** | **PROGRESS** — prepare green + `init/main.o` on amd64; not booting yet |

### C1 Linux 6.9 (Docker `--platform linux/amd64`, image `ggcc-linux-amd64`)

**Green:**
- `prepare0` full pass (bounds.h, asm-offsets.h, devicetable-offsets.h, checksyscalls)
- `init/main.o` **built** (283KB ELF)
- `usr/built-in.a` AR
- ~19 `.o` files in tree mid-build

**Current failures (fail-drive):**
- `arch/x86/entry/vdso/vma.c` — `ERROR: -> on non-pointer Int` (typeof/layout)
- `init/do_mounts.c` — `ERROR: enum enumerator name expected`
- **no** `bzImage` / QEMU

**Platform:** Host Docker default is aarch64; use `ggcc-linux-amd64` for x86_64 kernel asm/as match.

### blocked_reason
C1: real kernel objects after prepare; vdso typeof + do_mounts enum still fail.  
C2: testfixture/Redis not run.  
**Goal NOT complete.**
