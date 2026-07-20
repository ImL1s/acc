# Progress (NO-DOWNGRADE)

## Stage A — PASS | Stage B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C5 | PASS |
| C3 | PASS |
| C4 | held |
| C2 | **BLOCKED** (major progress) |
| C1 | **BLOCKED** |

### C2 evidence
| Check | Result |
|-------|--------|
| amalgamation → `.s`→`.o`→link | PASS |
| `sqlite3_libversion` | **PASS** `3.45.3` / `3045003` |
| `sqlite3_initialize` | **PASS** `rc=0` |
| `sqlite3_open(":memory:")` | **FAIL** SIGSEGV (`createCollation` / `FindCollSeq`) |
| full SQLite tests / Redis | not started |

### Fixes landed
- Real `va_list` (AAPCS64 reg save)
- Struct assign via `memcpy`
- Static string fields in struct inits
- Int locals: full 8-byte stores (killed high-bit garbage)
- Linux printf ABI, frame offsets, etc.

### Next
1. Fix `createCollation` / `sqlite3FindCollSeq` crash on open
2. `sqlite_smoke_ok` then full tests or Redis
3. C1 kernel boot

### blocked_reason
C2: initialize works; open still crashes — full project tests not green.  
C1: no boot proof.

**Never claim complete without C1+C2 green.**
