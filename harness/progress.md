# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale — re-run pending) |
| C4 clean-room | held |
| **C2** | **IN PROGRESS** — CREATE/INSERT/SELECT (+ REAL values) green; float **text** printf still garbled |
| **C1** | **BLOCKED** |

### C2 smoke (Docker Linux, ggcc .s only)
| Step | Result |
|------|--------|
| open / `SELECT 1` | **PASS** |
| CREATE + INSERT + SELECT int | **PASS** |
| master cell vs gcc | **PASS** |
| REAL `column_double` (1.5, 0.5, 1e2) | **PASS** |
| REAL `column_text` / %g | **FAIL** — e.g. `1.5e+g70` (printf path) |
| full C2 suite / Redis | pending |

### Root causes fixed (this continue)
1. **x19 not preserved** → `nHdr += f()` lost → MakeRecord nHdr=2 → CORRUPT  
   - prolog/epilog save x19; spill x19 across compound-assign RHS
2. **u64→double bitcast** → AtoF significand wrong  
   - ucvtf for unsigned int→float casts/assigns
3. **typeof(-float) → Int** → call used scvtf on IEEE bits  
   - typeof Neg preserves Double; call prefers fmov when param is float

### Next
- Fix SQLite float→text (`%.16g` / appendf) garbled output
- Full C2 harness; C1 kernel; C5 re-run

### blocked_reason
C2 not complete (float printf; full suite). C1 no boot. Goal NOT complete.
