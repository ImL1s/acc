# Postgres initdb / 237 status (D-pg237)

**Date:** 2026-07-25  
**Status:** **BLOCKED** on ecpg link — not 237 PASS. Do not invent PASS summary.

**Canonical handoff:** [`docs/HANDOFF_CCC_STATUS_COMPLETE.md`](../HANDOFF_CCC_STATUS_COMPLETE.md)  
**Living:** [`harness/progress.md`](../../harness/progress.md)

## Ops

- Wrapper: `harness/docker/acc_cc_wrapper.sh`
- `ACC=/work/target-linux/release/ggcc` (+ `ACC_BIN`, `ACC_ARCH=x86_64`, `ACC_TARGET_OS=linux`)
- VPATH: `scratch/postgres-build-15.7` ← `third_party/stage_c/postgres/postgresql-15.7/`
- Soft owner: `src/codegen_x86_64.rs` (sole editor)
- `make check` as unprivileged `pgtest` (root → `initdb: cannot be run as root`)

## Landed (keep; do not re-litigate)

Quiet **initdb exit 0** and postmaster Unix listen path. Soft units / fixes include:

| Area | Unit / note |
|------|-------------|
| pgstat pending flush | restored `pgstat_report_stat` |
| SysV variadic `%al` | `tests/sysv_variadic_al.c` |
| Static addr in static init (`if_exists`) | `tests/static_local_addr_in_init.c` |
| `sockaddr_un.sun_path[108]` | `tests/sockaddr_un_sun_path.c` |
| ELF globals `.weak`/`.type`/`.size` | WAL / `PGLZ_strategy_default` |
| Aggregates / shr / bitwise / small return frame / struct+bool | see `tests/*` |

## Current blocker

```
undefined reference to `descriptor_type`
```

in **ecpg** (`descriptor.o`) — soft static variable mangling in data initializers. Same family as `__static_*` / `if_exists`; extend so referenced statics remain linkable.

## Next

1. Minimal soft unit reproducing `descriptor_type`-style static in initializer.
2. Fix mangling in `src/codegen_x86_64.rs` (+ parser/ast only if required).
3. Rebuild ecpg / `make check` as `pgtest` → exit 0.
4. Write **honest** `scratch/c2_postgres_237_summary.txt` only with real `regression.out`.
5. Then HANDOFF §3 verify → COMPLETE stamp.
