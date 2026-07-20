# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| **C3** multiarch | **PASS** 40/40 (held; re-run after language churn if needed) |
| **C5** double-run | **PASS** 207/207 identical (held; re-run after language churn) |
| C4 clean-room | held (wrapper refuses gcc on .c) |
| **C2** | **PROGRESS** — SQLite amalgamation API smoke PASS; no testfixture |
| **C1** | **PROGRESS** — past first kbuild .c; still blocked before boot |

### C1 Linux 6.9 (Docker, evidence in SCRATCH)

**Working (fresh Docker + wrapper + ggcc):**
- Kconfig probes (cc-version / as-version) OK
- `scripts/mod/empty.o` OK
- `scripts/mod/devicetable-offsets.s` OK → `scripts/mod/devicetable-offsets.h` generated
- `kernel/bounds.s` OK → `include/generated/bounds.h` generated  
  (e.g. `NR_PAGEFLAGS 22`, `MAX_NR_ZONES 2`; `SPINLOCK_SIZE` still wrong/0 — sizeof layout gap)

**Blocked on:**
- `arch/x86/kernel/asm-offsets.c` — still fail-driving through ~97k-line PP of arch/headers  
  last class of errors: GNU C (asm goto, nested designators, bitfields, statement-exprs, …)  
  **no** `bzImage` / QEMU boot yet

**Language fixes landed this pass (non-exhaustive):**
- GNU type sugar: `__signed__` / `__unsigned__` / `__const__` / `__volatile__`
- `-include` + CWD include resolution; wrapper forwards `-include` and `-Wp,-MMD`
- Macro blue-paint (no recursive `inline` explosion)
- `typeof` / `_Generic` / `__builtin_va_list` / `__builtin_types_compatible_p` / `__builtin_expect` / `__builtin_constant_p`
- Statement expressions `({…})`, empty asm `""`, named asm operands, asm goto/inline, resilient ALTERNATIVE skip
- Enum values from prior enumerators; case ranges; trailing commas; nested designators
- Soft fallbacks: non-const `"i"` asm, incomplete offsetof, undefined locals in header inlines

### C2 SQLite
- amalgamation compile + link + run smoke: CREATE/INSERT/SELECT/… **PASS**
- official testfixture / Redis: **not run**

### blocked_reason
C1: full tinyconfig build + QEMU boot not achieved; stuck mid `asm-offsets` language surface.  
C2: full SQLite testfixture / Redis suite not executed.  
**Goal NOT complete.**
