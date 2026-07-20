# Progress (NO-DOWNGRADE)

## Stage A — PASS
- Evidence: stage_a.log

## Stage B — PASS  
- Full single-exec: **202/220 ≈ 91.8%** (was 198; language work for Stage C improved suite)
- 3 projects: tinyc, lua_smoke, miniz_smoke
- Evidence: stage_b.log, suite_full_count.txt

## Stage C — PARTIAL
| Gate | Status |
|------|--------|
| C5 double-run | PASS |
| C3 dual ISA 40/40 | PASS |
| C4 clean-room | held |
| C2 SQLite/Redis | BLOCKED (parse progress deep into sqlite3.c; no .o yet) |
| C1 kernel 6.9 boot | BLOCKED (Linux ELF + Docker ready; full kernel not compiled) |

### Language fixes this session (SQLite-driven)
- PP: shared macros across includes, #define body comment strip, \\newline splice, multi-line macro calls, `\'` in macro args, expand caps
- Parser: void* params, trailing `const`, `long double`, bitfields, `a[(n+1)]`, `void (*f(T))(void)`, comma operator
- va_start/va_arg/va_end stubs

### Next failing SQLite site
- Compile-options string table / further constructs after ~line 16k preprocessed

### Rule
Do NOT claim complete until C1+C2 green.
