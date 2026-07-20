# Progress (NO-DOWNGRADE)

## Stage A — PASS
## Stage B — PASS (~202/220 + 3 projects)
## Stage C — PARTIAL — **NOT complete**

| Gate | Status |
|------|--------|
| C5 | PASS |
| C3 | PASS |
| C4 | held |
| **C2** | **BLOCKED** |
| **C1** | **BLOCKED** |

### C2 detail
| Step | Result |
|------|--------|
| `sqlite3.c` → frontend → `.s` | PASS (~12MB) |
| Docker assemble `.o` + link | PASS |
| `sqlite3_libversion` smoke | **PASS** `3.45.3` / `3045003` |
| `sqlite3_initialize` / `open` | **FAIL** SIGSEGV / pthread abort |
| Full SQLite tests / Redis | not run |

### Root causes fixed this stretch
1. **`va_arg` was null-deref** → real AAPCS64 reg-save + `__ggcc_va_start`/`__ggcc_va_arg`
2. **Struct `*p` assign** only stored 8 bytes → `memcpy` for aggregates
3. **Static `const char*` fields** emitted `.zero` → `.quad l_str_*`
4. Linux vs Darwin **printf varargs ABI**
5. Large **FP offsets**, call-arg comma, bitwise compound assign, etc.

### Still broken (open path)
- Crash in/around `sqlite3InsertBuiltinFuncs` / later pthread
- GDB: corrupt stack frames (SP discipline / frame size?)
- Evidence: `{SCRATCH}/stage_c_projects.log`, `open_gdb*.txt`, `init_test.txt`

### C1
Docker kernel scripts + `ggcc_cc_wrapper` ready; no bootable kernel.

### blocked_reason
C2: amalgamation links and libversion works, but initialize/open runtime incorrect — full project tests not green.  
C1: no QEMU boot proof.

**Do not claim complete. Do not rebrand Stage B as complete.**
