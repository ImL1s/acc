# Progress (NO-DOWNGRADE)

## Stage A — PASS
- Evidence: stage_a.log

## Stage B — PASS  
- Full single-exec: **202/220 ≈ 91.8%** baseline
- 3 projects: tinyc, lua_smoke, miniz_smoke
- Evidence: stage_b.log, suite_full_count.txt

## Stage C — PARTIAL
| Gate | Status |
|------|--------|
| C5 double-run | PASS |
| C3 dual ISA 40/40 | PASS |
| C4 clean-room | held |
| C2 SQLite/Redis | **IN PROGRESS** — full `sqlite3.c` → Linux aarch64 `.s` **OK** (~12.4MB asm); link/run smoke + full tests still open |
| C1 kernel 6.9 boot | BLOCKED (Docker/wrapper scripts ready; language/CLI gap for kernel TUs) |

### Milestone 2026-07-20 session
**Full amalgamation `third_party/stage_c/sqlite/sqlite3.c` compiles to assembly with:**
```
./target/release/ggcc --target-os linux -S -o $SCRATCH/sqlite3.s third_party/stage_c/sqlite/sqlite3.c
# EXIT 0, ~12455116 bytes
```

### Language / PP / codegen fixes (SQLite-driven this session)
- Compound assign: `|= &= ^= <<= >>=`
- Storage: `register` / `inline` / `restrict` / `auto`
- Hex int suffixes `LL`/`ULL` after hex digits
- POSIX stubs: `uid_t`/`gid_t`/`stat`/`tm`/`flock`/pthread constants/errno map/mmap/fcntl/sysconf
- PP: `NDEBUG` + `assert(x)` no-op; dynamic `__LINE__`/`__FILE__`
- Parser: call args use `parse_assign` (comma was eating multi-arg calls)
- Codegen: >8 integer args (AAPCS64 stack); undeclared id as extern fn designator;
  index/`->` soft fallbacks for incomplete typing; global `&` / compound init;
  bare InitList as static blob

### Next for C2
1. Docker assemble+link `smain.s` + `sqlite3.s` (system `cc` only on `.s`)
2. Run `sqlite_smoke_ok` / `sqlite3_libversion_number`
3. Full SQLite test suite or Redis as second large project
4. Evidence: `{SCRATCH}/stage_c_projects.log` VERDICT PASS

### Next for C1
- Run `harness/docker/build_kernel.sh` for real last_failure under ggcc CC wrapper
- Expand language for kernel (GNU attributes, asm, `-I`/`-D`)

### Rule
Do NOT claim complete until C1+C2 green with SCRATCH evidence.
Do NOT rebrand Stage B as complete.
