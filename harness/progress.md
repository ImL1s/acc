# Progress (NO-DOWNGRADE)

## Stage A — PASS
- Evidence: `{SCRATCH}/stage_a.log`

## Stage B — PASS  
- Full single-exec: **202/220 ≈ 91.8%**
- 3 projects: tinyc, lua_smoke, miniz_smoke
- Evidence: `{SCRATCH}/stage_b.log`

## Stage C — PARTIAL (NOT complete)
| Gate | Status |
|------|--------|
| C5 double-run | PASS (`stage_c_rerun.log`) |
| C3 dual ISA 40/40 | PASS (`stage_c_multiarch.log`) |
| C4 clean-room | held |
| C2 SQLite/Redis | **BLOCKED** — amalgamation **frontend→codegen→.s→.o→link** works in Docker; **runtime incorrect** (version garbage / open segfault). Full SQLite tests not green. |
| C1 kernel 6.9 boot | **BLOCKED** — Docker/wrapper/tinyconfig scripts ready; no boot proof |

### C2 evidence trail (2026-07-20)
1. `ggcc --target-os linux -S sqlite3.c` → EXIT 0, ~12.4MB `.s`
2. Docker `cc -c sqlite3.s` → EXIT 0, ~4.8MB `.o`
3. Link `smain + sqlite3 + errno_def` → EXIT 0
4. Run: `smoke_ver` prints garbage; `smoke_open` SIGSEGV
5. Log: `{SCRATCH}/stage_c_projects.log` VERDICT BLOCKED

### Key fixes this session
- Bitwise compound assign; register/inline/restrict; hex `LL`
- PP stubs: POSIX types, errno/fcntl/mmap, `assert`/`NDEBUG`, `S_IS*`, `PTHREAD_MUTEX_INITIALIZER`, `__LINE__`/`__FILE__`
- Parser: call args via `parse_assign` (comma operator bug)
- Codegen: AAPCS64 stack args; large frame offsets (ADD/SUB ranges); global inits; extern-fn designators

### Next (hard)
1. Correctness on sqlite: real types for function returns/pointers; ABI for structs; global init of function pointers (aSyscall table)
2. Green smoke: `sqlite_smoke_ok` + libversion match
3. Full test suite or Redis
4. C1: `build_kernel.sh` first real make failure → language work

### Rule
**Do NOT claim complete until C1+C2 green.** Stage B is not complete.
