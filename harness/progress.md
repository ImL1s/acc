# Progress (NO-DOWNGRADE)

## Stage A — PASS | Stage B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C5 | PASS |
| C3 | PASS |
| C4 | held |
| **C2** | **BLOCKED** (open works; exec crashes) |
| **C1** | **BLOCKED** |

### C2 smoke ladder
| Check | Result |
|-------|--------|
| amalgamation link | PASS |
| libversion | **PASS** `3.45.3` |
| initialize | **PASS** `rc=0` |
| `open(":memory:")` | **PASS** `rc=0` + valid db ptr |
| `exec("select 1")` | **FAIL** SIGSEGV |
| full tests / Redis | not started |

### Fixes this stretch
1. Compound-assign spill (`pColl += enc-1` clobber)
2. Int store width: struct fields `str w`, stack slots zero-extend 8-byte; `ldrsw` loads
3. Prior: va_list, memcpy struct assign, static string fields

### Next
1. Diagnose `sqlite3_exec` crash
2. Green create/insert/select smoke → full suite or Redis
3. C1 kernel boot

### blocked_reason
C2: open OK, SQL exec still crashes — not full-project green.  
C1: no boot proof.
