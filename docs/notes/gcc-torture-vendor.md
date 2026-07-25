# Public FSF GCC c-torture execute tests (vendor pointer)

**Source:** sparse clone of [gcc.gnu.org/git/gcc.git](https://gcc.gnu.org/git/gcc.git)  
**Checkout:** `third_party/gcc-sparse` (gitignored sparse tree)  
**Symlink:** `third_party/gcc-torture` → `gcc-sparse/gcc/testsuite/gcc.c-torture/execute/`  
**Env:** `TORTURE_DIR` (default `$ROOT/third_party/gcc-torture`), `TORTURE_LIMIT`, `TORTURE_LOG`  
**License:** GPL-3.0+ (GCC testsuite)

Not from Claude CCC `src/` or CCC-private trees. Do **not** cite interim c-testsuite 13/13 (`scratch/torture_subset.log`) as torture ~99%.

## Refresh vendor (sparse)

```bash
cd third_party/gcc-sparse
git pull --depth 1
git sparse-checkout set gcc/testsuite/gcc.c-torture/execute
ln -sfn gcc-sparse/gcc/testsuite/gcc.c-torture/execute ../gcc-torture
```

As of 2026-07-24: **1690** `*.c` files; sparse HEAD `380bd5c`.

## Harness

Status track is **x86_64 / linux**. Upstream `ggcc` defaults to `-m aarch64`; the harness forces `-m x86_64 --target-os linux` (override with `GGCC_ARCH` / `GGCC_TARGET_OS`).

```bash
# Declared subset → scratch/torture_gcc_subset.log
GGCC_USE_DOCKER=1 TORTURE_LIMIT=100 bash harness/run_torture_subset.sh

# Expanded companion (keep primary log via TORTURE_LOG)
GGCC_USE_DOCKER=1 TORTURE_LIMIT=500 \
  TORTURE_LOG=$PWD/scratch/torture_gcc_subset_500.log \
  bash harness/run_torture_subset.sh
```

Requires Docker image `ggcc-linux` + `target-linux/release/ggcc` for Status parity.

## Honest SCRATCH rates (x86_64/linux, 2026-07-24T17:40Z)

| Set | Pass | Fail | Rate | Log |
|-----|------|------|------|-----|
| **100** (declared) | 77 | 23 | **77.0%** | `scratch/torture_gcc_subset.log` |
| 500 | 326 | 174 | **65.2%** | `scratch/torture_gcc_subset_500.log` |

**≥99% achieved: NO.** Soft codegen / parser / builtins dominate — see `scratch/torture_gcc_blocked_reason.md` and `scratch/torture_soft_queue_ids.txt` for `D-pg237` handoff. Do not thrash `src/codegen_x86_64.rs` from this domain.

A partial full-execute attempt was truncated mid-run (`scratch/torture_gcc_subset_full.log`); do not treat it as a complete 1690-ID rate.
