# Postgres initdb status (D-pg237)

**Date:** 2026-07-24 (evening)  
**Status:** BLOCKED — not 237 PASS. Do not invent `scratch/c2_postgres_237_summary.txt` PASS.

## Ops

- Host disk recovered (~100Gi+ free after iOS sim prune + Docker VM restart from ENOSPC).
- Wrapper: `harness/docker/acc_cc_wrapper.sh` with `ACC=/work/target-linux/release/ggcc` (or `acc`).
- Fetch sources: `bash harness/docker/fetch_postgres.sh` (do not vendor the 15.7 tree in git).

## initdb323

- Restored **pristine** `genam.c` / `catcache.c` / `lsyscache.c` / `syscache.c` from Postgres REL_15_7.
- Prior `#if 0 /* GGCC_MARKER_STRIP */` **broke braces** and dropped `systable_getnext` from soft emit — soft ggcc **does not implement `#if 0`**.
- `systable_getnext` symbol restored; postgres relinked.
- initdb still **exit 1 / child 139** (`GGCC_SEGV_simple`). Stderr still large from markers in other TUs.
- Soft owner remains `src/codegen_x86_64.rs` until initdb green.

## Next

1. Delete remaining `GGCC_*` `write(2)` diagnostics **line-by-line** (no `#if 0`); keep only SEGV handler if needed.
2. Quiet initdb `timeout 900` → then `make check` → write honest `scratch/c2_postgres_237_summary.txt`.
