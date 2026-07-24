# CCC-Status snapshot (2026-07-24 evening)

**Goal: NOT COMPLETE.** Soft Stage-C bars do not count. Do not stamp `Goal: COMPLETE` in `harness/progress.md` until every Status row below is green with on-disk SCRATCH.

## Gate matrix (honest)

| Gate | Status | Evidence / blocker |
|------|--------|--------------------|
| C2 SQLite + Redis | **PASS** (re-prove as needed) | `scratch/c2_veryquick_summary.txt`, `scratch/c2_redis_marker` |
| C3 4-ISA | **PASS** (when Docker up) | `scratch/stage_c_4isa.log` → `STAGE_A_4ISA_RUN_COMPLETE` |
| Builtin M2 / M4 | **PASS** | `scratch/builtin_m2_marker`, `scratch/builtin_m4_marker` |
| Builtin M5 | **FAIL** | No `scratch/builtin_m5_marker`. `execve` OK after PT_LOAD / `_DYNAMIC` / `e_version` fixes; runtime `SEGV_ACCERR@0x400148` in `_start`. Same `.o` + `musl-gcc -static` prints Hello. |
| Postgres 237 | **BLOCKED** | Linked; initdb still child 139 (`GGCC_SEGV_simple`). Restored pristine `genam`/`catcache`/`lsyscache`/`syscache` after unsafe `#if 0` strip (soft does not implement `#if 0`). **0/237**. See `docs/notes/postgres_initdb_status.md`. |
| C1 busybox both arches | **GAP** | arm64 serial historically OK; x86_64 serial SCRATCH incomplete / `c1_boot_marker` not claimed for Status both-arches |
| Torture ~99% | **FAIL** | ~50% last known (`scratch/torture_gcc_subset.log`); subset smoke ≠ Status bar |
| Ledger / docs | **THIS SNAPSHOT** | Align `progress.md` + ledger to SCRATCH; no COMPLETE stamp |

## Ops note

Host disk recovered after ENOSPC (Docker VM restart + prune). Wrapper: `harness/docker/acc_cc_wrapper.sh` with Linux release binary often named `ggcc` under `target-linux/release/`.

## Next (to COMPLETE)

1. **M5:** Fix hosted ELF layout vs working musl-gcc ET_EXEC → Hello under strict → stamp `scratch/builtin_m5_marker`.
2. **PG:** Line-delete leftover `GGCC_*` `write(2)` markers (never `#if 0`) → quiet initdb → `make check` 237 → `scratch/c2_postgres_237_summary.txt`.
3. **C1 x86** serial BusyBox + torture queue + ledger honesty → only then stamp COMPLETE.
