# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale) |
| C4 clean-room | held |
| **C2** | **BLOCKED** |
| **C1** | **BLOCKED** |

### C2 smoke ladder (Docker Linux, ggcc .s only)
| Step | Result |
|------|--------|
| amalgamation link | PASS |
| libversion / init / open / close | PASS |
| exec `""` / `";"` | **PASS** |
| exec `SELECT 1` | **FAIL** rc=20 SQLITE_MISMATCH |
| CREATE/INSERT | FAIL (schema) |

### Root causes fixed this session
1. `.hword` for `unsigned short` static tables (lemon yy_*)
2. multipass struct layout; Token 16B ABI; `signed char` / `ldrsb x`
3. **Bitfield packing** (layout + access)

### SELECT 1 diagnosis (open)
- prepare succeeds; EXPLAIN shows **wrong bytecode**:
  `Null; MustBeInt; IfNot; Integer; ResultRow; DecrJumpZero`  
  vs correct `Integer; ResultRow; Halt`
- Looks like limit-register path runs despite `SelectNew` getting `pLimit=NULL`
- Next: why `p->pLimit` is non-NULL by `computeLimitRegisters`

### blocked_reason
C2: parser no longer crashes; SELECT 1 still wrong VDBE (MISMATCH). C1: no boot.
