# Stage contracts (frozen — do not downgrade)

Completion requires **Stage A + B + C** all green under the **CCC full parity** bar
(`docs/plans/2026-07-23-ccc-full-parity.md`). Hello-only or “≥40 tests” is **not** complete.
Prior soft Stage-C stamps (`sqlite_reg`, SDS, `ggcc-init` only) are **not** Goal COMPLETE.

## Kernel
- Version: **Linux 6.9** (source under `third_party/linux-6.9` or fetched by `harness/docker/build_kernel.sh`)
- Verification: Linux Docker + QEMU boot (macOS host must not claim C1 without this)

## Stage B — exactly 3 real projects
Listed in `harness/real_projects.md`:
1. **miniz** (single-file zlib-like, or vendored tiny inflate/deflate test)
2. **lua** (or lua subset / official if feasible)
3. **sqlite** amalgamation smoke / official if feasible

Build each with `CC=$PWD/target/release/ggcc` (or Linux path in Docker).

## Stage C1 — busybox shell bar (frozen)
- Serial must show **busybox `/bin/sh`** (or a documented shell prompt from busybox userspace).
- **`ggcc-init:` alone is not C1 PASS** under CCC-strict contracts.

## Stage C2 — large projects (≥2) — CCC-strict (frozen)
1. **SQLite:** official **`testfixture`** + **`test/veryquick.test`** under ggcc.
   - **`sqlite_reg` PASS is not C2 PASS.**
2. **Redis:** built **`redis-server`** + live RESP **`PING` / `SET` / `GET`**.
   - **SDS PASS is not C2 PASS.**

(Or replacements only if documented here with reason — never smoke-only.)

## Multiarch oracle subset (Stage C3)
Same IDs must PASS on **aarch64** and **x86_64** backends:
`00001 00002 00003 00004 00005 00006 00007 00008 00009 00010 00012 00015 00021 00025 00030 00031 00034 00035 00036 00038`

(CCC full parity later expands to i686 + riscv64; soft 2-ISA green is not “4 ISA” complete.)

## Harness rules
- Pass = compile with **ggcc** → run → match expected (no skip-as-pass)
- Per-test timeout (compile/run)
- Task locks: `harness/current_tasks/`
- Progress: `harness/progress.md`
- Mutation + anti-bypass always required
