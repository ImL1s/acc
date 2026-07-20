# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS (stale) |
| C4 clean-room | held |
| **C2** | **BLOCKED** (CREATE SEGV) |
| **C1** | **BLOCKED** |

### C2 smoke ladder (Docker Linux, ggcc .s only)
| Step | Result |
|------|--------|
| amalgamation link | PASS |
| libversion / init / open / close | PASS |
| exec `""` / `";"` | **PASS** |
| exec `SELECT 1` | **PASS** (stack-arg receive for 9th+ params) |
| CREATE/INSERT | **FAIL** — SEGV in `sqlite3BtreeCursor` during prepare |

### Root causes fixed this session
1. `.hword` for `unsigned short` static tables (lemon yy_*)
2. multipass struct layout; Token 16B ABI; `signed char` / `ldrsb x`
3. Bitfield packing (GCC aarch64 bit-offset / no-straddle) → `Column` 16B
4. **9th+ stack argument receive** (`ldr xN, [x29, #16+]`) → SelectNew pLimit; SELECT 1 green
5. **`sizeof` in array bounds** (`const_array_len`) → `Bitvec` 512B (was 16)
6. **64-bit integer compare** when operands are long (`cmp x` not `cmp w`) → GetUInt32("1") works
7. **`unsigned` types** (`UInt`/`ULong`/`UShort`) with zero-extend loads → mxPgno 0xfffffffe no longer -2; past SQLITE_FULL

### CREATE diagnosis (open)
- Was: `invalid rootpage` (GetUInt32 false-positive via 32-bit cmp of 4294967296)
- Then: `database or disk is full` (signed load of u32 mxPgno)
- Now: SEGV in `sqlite3BtreeCursor` during `CREATE TABLE` prepare (schema cursor)
- SELECT 1 still PASS; sizes of major structs match gcc (Column/Bitvec/WhereInfo/sqlite3)

### blocked_reason
C2: CREATE SEGV in btree cursor. C1: no boot. Goal NOT complete until Stage A+B+C green.
