# Project Status Tracking

## Goal: **NOT COMPLETE** — Postgres 237 initdb SEGV under remediation

# Format: project_name: TODO/PARTIAL/PASS (notes)
# Mirror of CCC ideas/new_projects.txt for ggcc CCC-strict parity.
# Never claim PASS without SCRATCH evidence. Soft bars (sqlite_reg/SDS/ggcc-init) do not count.
# SCRATCH root: scratch/ (export SCRATCH=$PWD/scratch)
# Synced: 2026-07-26 — Goal: **NOT COMPLETE** — Postgres 237 initdb SEGV under remediation.

sqlite: PASS (strict veryquick evidence on disk: scratch/c2_veryquick_summary.txt → "0 errors out of 317930"; scratch/c2_sqlite_veryquick.ec=0; scratch/c2_veryquick_run_meta.txt suite_completed=yes)
redis: PASS RESP (scratch/c2_redis_marker = PASS_REDIS_DEFAULT_LATENCY)
postgres: BLOCKED (strict 237 regression evidence on disk: scratch/c2_postgres_237_summary.txt; initdb SEGV exit 139 under remediation)
zlib: TODO (no scratch/c2_zlib.log)
lua: TODO (no scratch/c2_lua.log)
QuickJS: TODO (no scratch/c2_quickjs.log)
busybox: PASS (dual-arch arm64 and x86_64 QEMU serial show /# shell prompt: scratch/qemu_boot_a09.log, scratch/qemu_boot_x86_64.log)
FFmpeg: TODO (no scratch/c2_ffmpeg_fate.log)

# Status extras (tracked for honesty)
builtin_m4: PASS core (scratch/builtin_m4_marker = builtin_linker M4=ok; freestanding aarch64 ET_EXEC via src/linker; no system cc/ld)
builtin_m5: PASS (scratch/builtin_m5_marker = builtin_linker M5=ok; strict Hello via builtin as+ld static musl)
torture_gcc_subset: 77.0% pass rate (77/100 passed, 23 failed; raw log: scratch/torture_gcc_subset.log)


