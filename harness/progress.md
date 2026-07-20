# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale — re-run pending) |
| C4 clean-room | held |
| **C2** | **IN PROGRESS** — CREATE/INSERT/SELECT + REAL text green |
| **C1** | **BLOCKED** |

### C2 smoke (Docker Linux, ggcc .s only)
| Step | Result |
|------|--------|
| open / `SELECT 1` | **PASS** |
| CREATE + INSERT + SELECT int | **PASS** |
| master cell vs gcc | **PASS** |
| REAL column_double + column_text | **PASS** (1.5, 0.5, 2.25, 3.0) |
| full C2 suite / Redis | pending |

### Root causes fixed (this continue)
1. x19 not preserved → MakeRecord nHdr → CORRUPT
2. u64→double bitcast / typeof(-float) → REAL Inf
3. **sibling Decl name reuse** → rr[2] overlapped exp → 1.5e+g70
4. **variadic float in dN vs GP va_list** → mprintf %g wrong

### Next
- Full C2 harness log (SQLite tests / Redis)
- C1 kernel QEMU; C5 re-run
- Known residual: multi-float args to **libc** printf (d1+) may still be wrong

### blocked_reason
C2 full suite not green yet. C1 no boot. Goal NOT complete.
