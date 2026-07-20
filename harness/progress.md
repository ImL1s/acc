# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale — re-run after C2) |
| C4 clean-room | held |
| **C2** | **BLOCKED** |
| **C1** | **BLOCKED** |

### C2 smoke ladder (Docker Linux, ggcc-produced .s only)
| Step | Result |
|------|--------|
| amalgamation → asm → link | PASS |
| libversion | **PASS** `3.45.3` |
| initialize | **PASS** |
| open `:memory:` | **PASS** |
| close | **PASS** |
| exec `""` | **PASS** |
| exec `";"` | **PASS** `rc=0` |
| exec `SELECT 1` | **FAIL** `rc=20` SQLITE_MISMATCH (datatype mismatch) — no SIGSEGV |
| CREATE/INSERT | **FAIL** schema/rootpage / later SEGV |

### Key fixes landed (this session + prior)
- Multi-pass `collect_layouts` (YYMINORTYPE size)
- AAPCS64 small struct ABI (Token ≤16B in 2 GPRs)
- `signed char` → `Type::SChar` + `ldrsb x`
- **`.hword` for 2-byte static data** (lemon `yy_action`/`yy_lookahead` were `.long` → wrong shift/reduce → ExprDelete crash)

### Evidence
- `{SCRATCH}/stage_c_projects.log` updated with post-hword ladder
- GDB: after hword, only 2 `sqlite3Parser` calls (SEMI+EOF); no parser SEGV

### Next
1. Fix VDBE path so `SELECT 1` returns rows (MEM_Int / affinity / MustBeInt)
2. CREATE TABLE + INSERT + SELECT round-trip
3. Full SQLite tests or Redis; C1 kernel; C5 re-run

### blocked_reason
C2: SQL parser no longer crashes; `SELECT 1` still SQLITE_MISMATCH — not full-project green.  
C1: no boot proof.
