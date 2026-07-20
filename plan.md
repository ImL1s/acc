# ggcc — Clean-room C compiler (Acceptance Plan)

## Goal
From-scratch clean-room C compiler `ggcc` under `/Users/iml1s/Documents/mine/ggcc`.
Human-side method aligned with Anthropic CCC experiment; real capability, not toy demo.

## Hard constraints
1. **No** reading/copying/referencing `anthropics/claudes-c-compiler` `src/` or any compiler body.
2. Default path: project frontend → codegen → system `as`/`ld`/`cc` only on emitted `.s`/`.o`.
   **Forbidden:** feeding user `.c` to gcc/clang/ccc/tcc as the compiler.
3. **No** hardcode / fixture special-case / pre-baked binary as “compile result”.
4. **No** downgrading acceptance to “hello + ≥40 tests”.
5. **No** `update_goal(completed)` / “done” until Stage A+B+C all green.
6. Multi-agent / long-run OK; state in `harness/progress.md` + task locks.

## Human-side harness (required)
- agent loop + `current_tasks` lock + `progress.md`
- oracle: compile → run → diff expected
- public oracle: vendored c-testsuite + real project build scripts
- mutation check + anti-bypass always on

## Acceptance — Stage A (baseline, not complete)
- hello printf runs + mutation PASS + anti-bypass PASS
- c-testsuite single-exec **00001–00100** continuous pass rate **≥ 95%**
- Evidence: `{SCRATCH}/stage_a.log`

## Acceptance — Stage B (non-toy language, not complete)
- Language: multi-function, locals, pointers/arrays, struct/union, control flow, goto,
  typedef, globals, sizeof, basic preprocessor (`#define` simple, local `#include`)
- c-testsuite single-exec **full** pass rate **≥ 90%**
- ≥ 3 real small projects with `CC=$PWD/target/release/ggcc` build + tests
  (fixed list in `harness/real_projects.md`: tinyc, lua_smoke, miniz_smoke)
- Evidence: `{SCRATCH}/stage_b.log`

## Acceptance — Stage C (CCC-level; this is “complete”; all required)
- **C1.** Compile+link **bootable Linux kernel 6.9** on ≥1 arch.
  Host macOS → must verify via **Linux Docker/VM** (no “theoretically” hand-wave).
  Evidence: `{SCRATCH}/stage_c_kernel.log`
- **C2.** ≥2 large projects (fixed): **SQLite full test** and/or **Redis basic test**
  (or documented fixed list). Evidence: `{SCRATCH}/stage_c_projects.log`
- **C3.** ≥2 ISA backends (x86_64 + aarch64) pass same public oracle subset.
  Evidence: `{SCRATCH}/stage_c_multiarch.log`
- **C4.** Still clean-room; no external C compiler on user C.
- **C5.** Double-run: same suite twice → identical pass set.
  Evidence: `{SCRATCH}/stage_c_rerun.log`

## Completion definition (only)
Stage A + B + C all hard gates green **and** SCRATCH evidence complete.
If blocked on C: explicit `blocked_reason` + last failure log; **never** rebrand Stage B as complete.

## Verification plan
1. `cargo build --release`
2. `bash harness/mutation_check.sh` → PASS
3. Stage A: suite 00001–00100 ≥95%; stage_a.log present
4. Stage B: full suite ≥198/220; three real project build.sh with CC=ggcc; stage_b.log
5. Stage C5: two full suite runs; pass sets equal; stage_c_rerun.log
6. Stage C3: `bash harness/run_multiarch.sh`; stage_c_multiarch.log fail=0
7. Stage C2: SQLite and/or Redis under CC=ggcc; stage_c_projects.log
8. Stage C1: Docker Linux kernel 6.9 + QEMU boot; stage_c_kernel.log with boot proof
