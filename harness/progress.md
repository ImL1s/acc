# Progress (NO-DOWNGRADE)

## Stage A — PASS
## Stage B — PASS (~202/220 + 3 projects)
## Stage C — PARTIAL — **NOT complete**

| Gate | Status |
|------|--------|
| C5 | PASS |
| C3 | PASS |
| C4 | held |
| C2 | **BLOCKED** — amalgamation → `.s`→`.o`→link; **libversion PASS** (`3.45.3`); va_list real; **sqlite3_open still SIGSEGV** |
| C1 | **BLOCKED** — Docker/kernel scripts only |

### Recent (2026-07-20 night)
- **Root cause of early open crash:** `va_arg` was `(*(T*)0)` — fixed with AAPCS64 reg-save + `__ggcc_va_start`/`__ggcc_va_arg`
- **Struct assign:** `*ptr` → memcpy for aggregates (sqlite3_config MALLOC/MUTEX)
- **Still failing:** open path SIGSEGV (likely more ABI/global/layout issues after init)

### Evidence
- `{SCRATCH}/stage_c_projects.log` — VERDICT BLOCKED + partial smokes
- git: `feat: real va_list...`

### Next
1. GDB open crash after va_list fix → next null/fnptr/layout bug
2. Green `sqlite_smoke_ok` then full tests / Redis
3. C1 kernel compile under wrapper

**Do not claim done until C1+C2 green.**
