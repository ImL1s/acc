# CCC-Status snapshot (2026-07-25 noon)

**Start here:** [`docs/HANDOFF_CCC_STATUS_COMPLETE.md`](../HANDOFF_CCC_STATUS_COMPLETE.md)  
**Living stamp:** [`harness/progress.md`](../../harness/progress.md)

**Goal: NOT COMPLETE.** Soft Stage-C bars do not count. Do not stamp COMPLETE until every Status row is green with on-disk SCRATCH.

## Gate matrix (honest)

| Gate | Status | Evidence / blocker |
|------|--------|--------------------|
| C2 SQLite + Redis | **PASS** | `scratch/c2_veryquick_summary.txt`, `scratch/c2_redis_marker` |
| C3 4-ISA | **PASS** | `scratch/stage_c_4isa.log` → `STAGE_A_4ISA_RUN_COMPLETE` |
| Builtin M2 / M4 / M5 | **PASS** | `scratch/builtin_m{2,4,5}_marker` |
| C1 busybox both arches | **PASS** | `scratch/qemu_boot_a09.log`, `scratch/qemu_boot_x86_64.log` (`/#`) |
| Torture ~99% | **PASS** | `scratch/torture_gcc_subset.log` (100% on declared track) |
| Stage C Rerun (C4/C5) | **PASS** | `scratch/stage_c_rerun.log` |
| **Postgres 237** | **BLOCKED** | `ecpg/descriptor.o`: `undefined reference to descriptor_type` (soft static mangling). Soft: `src/codegen_x86_64.rs`. |
| Ledger / docs | **THIS SNAPSHOT** | Align to SCRATCH; no COMPLETE stamp |

## Postgres (short)

- Quiet **initdb exit 0** already landed (do not re-debug pending-ref / socket-path sizeof / `%al` unless regress).
- `make check` must run as **pgtest** (not root).
- After soft fix: rebuild common + backend, relink `postgres`, then `make check` → honest `scratch/c2_postgres_237_summary.txt` only if exit 0.

## Next (to COMPLETE)

1. Soft-fix ecpg `descriptor_type` link error.
2. Green `make check` + honest 237 summary (+ keep `regression.out`).
3. Re-run HANDOFF §3 verify script; stamp COMPLETE only then.
