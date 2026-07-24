# Public FSF GCC c-torture execute tests (vendor pointer)

**Source:** sparse clone of [gcc.gnu.org/git/gcc.git](https://gcc.gnu.org/git/gcc.git)  
**Symlink:** `third_party/gcc-torture` → `gcc-sparse/gcc/testsuite/gcc.c-torture/execute/`  
**License:** GPL-3.0+ (GCC testsuite)

Not from Claude CCC `src/` or CCC-private trees.

## Refresh vendor (sparse)

```bash
cd third_party/gcc-sparse
git pull --depth 1
git sparse-checkout set gcc/testsuite/gcc.c-torture/execute
```

## Harness

```bash
TORTURE_LIMIT=100 bash harness/run_torture_subset.sh
```

Log: `scratch/torture_gcc_subset.log`
