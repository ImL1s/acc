# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale) |
| C4 clean-room | held |
| **C2** | **BLOCKED** (CREATE SEGV past cursor open) |
| **C1** | **BLOCKED** |

### C2 smoke ladder (Docker Linux, ggcc .s only)
| Step | Result |
|------|--------|
| amalgamation link | PASS |
| open / empty / `;` | **PASS** |
| `SELECT 1` | **PASS** |
| CREATE TABLE | **SEGV** (after valid BtreeCursor; was invalid rootpage → FULL → p=0x8) |

### Root causes fixed (session)
1. 9th+ stack arg receive → SELECT 1
2. Bitfield packing (Column=16)
3. sizeof in array bounds (Bitvec=512)
4. 64-bit cmp for long (GetUInt32)
5. UInt/ULong zero-extend (mxPgno)
6. **measure_stmt visits Switch/Case** → VdbeExec frame 3504B (was 400B); Btree* no longer 0x8

### Next
CREATE SEGV in memset after first BtreeCursor with valid pBt. Continue fail-drive.

### blocked_reason
C2 incomplete; C1 no boot. Goal NOT complete.
