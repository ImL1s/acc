# Stage contracts (frozen — do not downgrade)

Completion requires **Stage A + B + C** all green. Hello-only or “≥40 tests” is **not** complete.

## Kernel
- Version: **Linux 6.9** (source under `third_party/linux-6.9` or fetched by `harness/docker/build_kernel.sh`)
- Verification: Linux Docker + QEMU boot (macOS host must not claim C1 without this)

## Stage B — exactly 3 real projects
Listed in `harness/real_projects.md`:
1. **miniz** (single-file zlib-like, or vendored tiny inflate/deflate test)
2. **lua** (or lua subset / official if feasible)
3. **sqlite** amalgamation smoke / official if feasible

Build each with `CC=$PWD/target/release/ggcc` (or Linux path in Docker).

## Stage C2 — large projects (≥2)
1. **SQLite** full/regression tests where feasible
2. **Redis** basic tests  
   (or replacements only if documented here with reason)

## Multiarch oracle subset (Stage C3)
Same IDs must PASS on **aarch64** and **x86_64** backends:
`00001 00002 00003 00004 00005 00006 00007 00008 00009 00010 00012 00015 00021 00025 00030 00031 00034 00035 00036 00038`

## Harness rules
- Pass = compile with **ggcc** → run → match expected (no skip-as-pass)
- Per-test timeout (compile/run)
- Task locks: `harness/current_tasks/`
- Progress: `harness/progress.md`
- Mutation + anti-bypass always required
