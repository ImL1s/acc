# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| **C3** multiarch | **PASS** 40/40 (held; re-run after language churn) |
| **C5** double-run | **PASS** 207/207 identical (held; re-run after language churn) |
| C4 clean-room | held (wrapper refuses gcc on .c) |
| **C2** | **PROGRESS** — SQLite amalgamation smoke PASS; no testfixture |
| **C1** | **PROGRESS** — kbuild prepare headers green on x86 path; real kernel .c in progress |

### C1 Linux 6.9

**Done (x86 path, aarch64 Docker host emitting x86_64 asm — headers via sed):**
- `scripts/mod/devicetable-offsets.h`
- `include/generated/bounds.h`
- `include/generated/asm-offsets.h`
- Static-inline codegen skip (except main) for compile-time speed

**Architecture note (2026-07-21):**
- Default Docker image on this Mac is **aarch64**. Emitting x86_64 `.s` then assembling with aarch64 `as` fails (`unknown mnemonic movq`).
- Built `ggcc-linux-amd64` (`--platform linux/amd64`) for matching x86_64 as/ld.
- Accidentally wiped `scripts/basic/` while cleaning host tools; restore from tarball in progress.

**Latest language fixes (commits 179edeb, 4629c61):**
- PP join across `#ifdef` inside `struct_group(...)` args
- Soft unterminated macro args; enum `__BPF_ENUM_FN` residue
- Peel glued `__u8name` bitfields; file-scope asm; asm goto labels
- x86_64 stack args beyond 6 registers
- `__label__` / `&&label` / GNU `?:`

**Still blocked:**
- Full `init/main.o` + rest of kernel → **no bzImage / QEMU**
- Host tool ELF mismatch when switching ARCH=arm64 ↔ x86 on shared tree
- Some offsetof soft-zeros in generated headers

### C2 SQLite
- amalgamation smoke PASS; testfixture / Redis **not run**

### blocked_reason
C1: past prepare headers; assembling real kernel objects needs arch-matched Docker (amd64 image ready); tree needs `scripts/basic` restore; then continue fail-drive on `init/main.o` and vdso.  
C2: full project suites not run.  
**Goal NOT complete.**
