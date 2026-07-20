# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS |
| C5 double-run | PASS |
| C4 clean-room | held |
| **C2** | **BLOCKED** |
| **C1** | **BLOCKED** |

### C2 smoke ladder (Docker Linux, ggcc-produced .s only)
| Step | Result |
|------|--------|
| amalgamation → asm → link | PASS |
| libversion | **PASS** `3.45.3` / `3045003` |
| initialize | **PASS** |
| open `:memory:` | **PASS** |
| close | **PASS** |
| exec `""` | **PASS** |
| exec `";"` / `select 1` | **FAIL** SIGSEGV (`ExprDeleteNN` / parser) |

### Key fixes landed
- Real va_list (AAPCS64)
- Compound-assign register spill (`pColl += enc-1`)
- Aggregate/struct assign via memcpy (Hash copy)
- Int stack slot vs struct field store widths
- Static string fields in initializers
- **Multi-pass `collect_layouts`** (HashMap order no longer collapses nested unions; `sizeof(YYMINORTYPE)=16`)
- **Small struct/union ABI (≤16B)** in 1–2 GPRs: Token pass into `sqlite3Parser`, struct returns, local init from calls
- **`signed char` → Type::SChar** with `ldrsb x` (lemon `yyRuleInfoNRhs[]` negative RHS counts)

### Verified offsets (ggcc == gcc)
- `Parse.sLastToken` @ 288
- `sizeof(Expr)=72`, `offsetof(pLeft)=16`, `yyStackEntry=24`
- `yyRuleInfoNRhs` data bytes match source; loads use `ldrsb x`

### Next
1. Fail-driven: `exec(";")` still dies in `sqlite3ExprDeleteNN` after Token ABI + signed char look correct — dig reduce actions / Expr build
2. Green select/create → full SQLite tests or Redis
3. C1 kernel QEMU boot

### blocked_reason
C2: open/close/empty-exec green; any real SQL token still crashes in ExprDelete/parser — not full-project green.  
C1: no boot proof.
