# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale — re-run pending) |
| C4 clean-room | held |
| **C2** | **IN PROGRESS** — CREATE/INSERT/SELECT int green; REAL→Inf |
| **C1** | **BLOCKED** |

### C2 smoke (Docker Linux, ggcc .s only)
| Step | Result |
|------|--------|
| open / empty / `;` / `SELECT 1` | **PASS** |
| CREATE TABLE + INSERT + SELECT int | **PASS** |
| master cell vs gcc (`1f 01 06 17 … table…`) | **PASS** |
| REAL column value | **FAIL** — reads as `Inf` (float/Vdbe path) |
| full C2 suite / Redis | pending |

### Root causes fixed this session
1. SELECT 1: stack 9th+ arg; bitfields; sizeof/offsetof; 64-bit cmp; UInt
2. measure Switch/Case → VdbeExec frame
3. offsetof array dims → NestedParse saveBuf
4. va_list stack overflow copy → NestedParse `#%d` → `#2`
5. **x19 not preserved across calls** → `nHdr += f()` lost → MakeRecord nHdr=2 → CORRUPT
   - fix: prolog/epilog save x19; spill x19 across compound-assign RHS

### Next
- Fix REAL/`Inf` (OP_Real / double constant embedding / Mem.u.r)
- Full C2 harness log; C1 kernel; C5 re-run

### blocked_reason
C2 not complete (REAL broken; full suite not green). C1 no boot. Goal NOT complete.
