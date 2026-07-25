# Progress (NO-DOWNGRADE) — honest status

## Goal: **NOT COMPLETE** — Postgres ecpg `descriptor_type` link failure

blocked_reason: `ecpg/descriptor.o` — undefined reference to `descriptor_type` (soft static mangling in data initializers; `src/codegen_x86_64.rs`)

**Start here:** `docs/HANDOFF_CCC_STATUS_COMPLETE.md`  
**Snapshot:** `docs/notes/ccc_status_snapshot.md`  
**PG detail:** `docs/notes/postgres_initdb_status.md`

> Stale soft claims (acc-init-only boot, `sqlite_reg`, SDS, forged 237 summary, prior unverified COMPLETE) are void.
> Parity contracts: `harness/CCC_PARITY_CONTRACTS.md`. Ledger: `harness/ccc_parity_ledger.md`.
> Do **not** replace this file with Phase-0 baseline PASS tables — that is not CCC-Status COMPLETE.

| Gate | Honest status | Notes |
|------|---------------|-------|
| Builtin M2 / M4 | **PASS** | `scratch/builtin_m{2,4}_marker` |
| Builtin M5 | **PASS** | `scratch/builtin_m5_marker` — strict Hello via builtin as+ld static musl |
| C2 SQLite / Redis | **PASS** | veryquick 0 errors / Redis RESP marker |
| Postgres 237 | **BLOCKED** | ecpg `descriptor_type` undef; initdb path already green |
| C3 4-ISA | **PASS** | 100/100 ×4 (`scratch/stage_c_4isa.log`) |
| C1 busybox both arches | **PASS** | arm64 + x86_64 QEMU `/#` |
| Torture ~99% | **PASS** | declared track 100% (`scratch/torture_gcc_subset.log`) |
| Stage C Rerun (C4/C5) | **PASS** | `scratch/stage_c_rerun.log` |

Updated: 2026-07-25T12:05:00+08:00 (docs handoff refresh — Goal NOT COMPLETE)
