# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale) |
| C4 clean-room | held |
| **C2** | **BLOCKED** — CREATE → SQLITE_CORRUPT |
| **C1** | **BLOCKED** |

### C2 smoke (Docker Linux, ggcc .s only)
| Step | Result |
|------|--------|
| open / empty / `;` / `SELECT 1` | **PASS** |
| CREATE TABLE | **FAIL** rc=11 `database disk image is malformed` (no longer SEGV at start) |

### Root causes fixed this session (cont.)
1. SELECT 1: stack 9th+ arg receive
2. Bitfield packing; sizeof array bounds (Bitvec); 64-bit cmp; UInt loads
3. measure Switch/Case → VdbeExec frame
4. **offsetof in array dims** → NestedParse `saveBuf[136]` (was 0 → stack smash memset 0x120)
5. **va_list stack overflow copy** → NestedParse `#%d` formats to `#2` (was literal `#%d` / unrecognized token)

### CREATE diagnosis (open)
- NestedParse SQL now well-formed:  
  `UPDATE 'main'.sqlite_master SET type='table', name='t', … rootpage=#2 … rowid=#1`
- Still corrupt when applying schema / writing btree pages
- Next: put/get page image, OP_CreateBtree, master insert path

### blocked_reason
C2: CREATE malformed disk image. C1: no boot. Goal NOT complete.
