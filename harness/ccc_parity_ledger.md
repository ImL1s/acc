# Project Status Tracking
# Format: project_name: TODO/PARTIAL/PASS (notes)
# Mirror of CCC ideas/new_projects.txt for ggcc CCC-strict parity.
# Never claim PASS without SCRATCH evidence. Soft bars (sqlite_reg/SDS/ggcc-init) do not count.
# SCRATCH root: scratch/ (export SCRATCH=$PWD/scratch)
# Synced: 2026-07-24 evening — Goal NOT COMPLETE (see docs/notes/ccc_status_snapshot.md).

sqlite: PASS (strict veryquick evidence on disk: scratch/c2_veryquick_summary.txt → "0 errors out of 317930"; scratch/c2_sqlite_veryquick.ec=0; scratch/c2_veryquick_run_meta.txt suite_completed=yes. Full scratch/c2_sqlite_veryquick.log not present — do not cite missing path. soft sqlite_reg does not count)
redis: PASS RESP (scratch/c2_redis_marker = PASS_REDIS_DEFAULT_LATENCY; soft SDS does not count)
postgres: PARTIAL (postgres linked; initdb still SEGV child 139 as of initdb323; no scratch/c2_postgres_237_summary.txt PASS; make check / 237 .out bar not PASS — docs/notes/postgres_initdb_status.md)
zlib: TODO (no scratch/c2_zlib.log)
lua: TODO (no scratch/c2_lua.log)
QuickJS: TODO (no scratch/c2_quickjs.log)
busybox: PARTIAL (arm64 historically PASS_BOOT + serial; x86_64 dual-arch Status SCRATCH incomplete — C1 both-arches not met for COMPLETE; ggcc-init-only soft boots do not count)
FFmpeg: TODO (no scratch/c2_ffmpeg_fate.log)

# Status extras (not CCC project rows; tracked for honesty)
builtin_m4: PASS core (scratch/builtin_m4_marker = builtin_linker M4=ok; freestanding aarch64 ET_EXEC via src/linker; no system cc/ld)
builtin_m5: FAIL (no scratch/builtin_m5_marker; execve OK after PT_LOAD/_DYNAMIC/e_version; runtime SEGV_ACCERR@0x400148 — docs/notes/builtin_m5_requirements.md)
torture_99pct: FAIL (scratch/torture_gcc_subset.log ≈ 845/1690 = 50%; NOT ~99%; subset smoke does not count)
