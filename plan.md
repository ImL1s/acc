# acc — Clean-room C compiler (Acceptance Plan)

## Goal
From-scratch clean-room C compiler `acc`.
Human-side method aligned with Anthropic CCC experiment; real capability, not toy demo.

## Hard constraints
1. **No** reading/copying/referencing `anthropics/claudes-c-compiler` `src/` or any compiler body.
2. Default path: project frontend → codegen → system `as`/`ld`/`cc` only on emitted `.s`/`.o`.
   **Forbidden:** feeding user `.c` to gcc/clang/ccc/tcc as the compiler.
3. **No** hardcode / fixture special-case / pre-baked binary as “compile result”.
4. **No** downgrading acceptance to “hello + ≥40 tests”.
5. **No** `update_goal(completed)` / “done” until **CCC-Status** gates green (see Completion definition; soft 2-ISA / soft Stage-C never counts).
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
- ≥ 3 real small projects with `CC=$PWD/target/release/acc` build + tests
  (fixed list in `harness/real_projects.md`: tinyc, lua_smoke, miniz_smoke)
- Evidence: `{SCRATCH}/stage_b.log`

## Acceptance — Stage C (CCC full parity)

> Soft Stage-C stamps (acc-init-only boot, `sqlite_reg`, SDS, soft 2-ISA green) are **not** Goal COMPLETE.
> Strict contracts: `harness/STAGE_CONTRACTS.md`, `harness/real_projects.md`.
> **Start here (goal + handoff):** [`docs/HANDOFF_CCC_STATUS_COMPLETE.md`](docs/HANDOFF_CCC_STATUS_COMPLETE.md).
> Full roadmap: [`docs/plans/2026-07-23-ccc-full-parity.md`](docs/plans/2026-07-23-ccc-full-parity.md).
> Finish sequencing (2026-07-24): [`docs/plans/2026-07-24-ccc-status-complete-finish.md`](docs/plans/2026-07-24-ccc-status-complete-finish.md) — **harness OK / CCC `src/` forbidden**.
> Cursor plan Status matrix: **CCC Full Finish** (`CCC Full Finish-e6f6e6a3`).

### Two finish labels (honest)

- **CCC-core** (interim note only — do not stop / do not set Goal COMPLETE):
  1. Fresh SQLite `testfixture`+`veryquick` **0 errors** + Redis RESP re-proof + `stage_c_projects.log` VERDICT PASS
  2. arm64 **and** x86_64 busybox shell in QEMU (soft `acc-init` PASS deleted)
  3. C3 contracts frozen to **raised** bar; ≥95% of 100 both ISAs (path to ≥90%/220 open or green)
  4. i686 + riscv64 backends merged (4 ISAs)
  5. Builtin **M2 + M4** on ≥1 ISA
  6. C4 + C5 re-PASS after last codegen thrash

- **CCC-Status** — **only this** may set `Goal: COMPLETE` in `harness/progress.md`:
  7. Core green
  8. Torture / ~99% track evidence (SCRATCH)
  9. Postgres **237** regression under acc with SCRATCH
  10. Builtin **M5** required (no Status deferral)
  11. Ledger megaprojects claimed only with SCRATCH
  12. `progress.md` Goal COMPLETE + README aligned

### Stage C gate detail

- **C1.** Compile+link **bootable Linux kernel 6.9**; QEMU serial must show **busybox `/bin/sh`** (or documented shell prompt) — **not** `acc-init:` alone. Status COMPLETE requires busybox on **arm64 and x86_64**.
  Host macOS → must verify via **Linux Docker/VM**.
  Evidence: `{SCRATCH}/stage_c_kernel.log`
- **C2.** ≥2 large projects under CCC-strict bar:
  - **SQLite:** official **`testfixture`** + **`test/veryquick.test`** with **0 errors** (NOT `sqlite_reg` PASS)
  - **Redis:** built **`redis-server`** + RESP **`PING`/`SET`/`GET`** (NOT SDS PASS)
  Evidence: `{SCRATCH}/stage_c_projects.log`
- **C3.** Raised multiarch contracts (not soft dual-ISA escape). Soft 2-ISA green is **not** COMPLETE; Status requires **4 ISAs** (x86_64, aarch64, i686, riscv64).
  Evidence: `{SCRATCH}/stage_c_multiarch.log`
- **C4.** Still clean-room; no external C compiler on user C.
- **C5.** Double-run: same suite twice → identical pass set.
  Evidence: `{SCRATCH}/stage_c_rerun.log`

## Completion definition (only)

**`Goal: COMPLETE` = CCC-Status gates met** (matrix above: Core + torture/~99% + Postgres 237 + M5 + ledger SCRATCH + docs aligned).

There is **no** 2-ISA soft COMPLETE escape. Soft Stage C (`sqlite_reg` / SDS / `acc-init`) must never be rebranded as Goal COMPLETE.
If blocked: explicit `blocked_reason` + last failure log.
See `docs/plans/2026-07-23-ccc-full-parity.md` and Cursor plan **CCC Full Finish**.

## Verification plan
1. `cargo build --release`
2. `bash harness/mutation_check.sh` → PASS
3. Stage A: suite 00001–00100 ≥95%; stage_a.log present
4. Stage B: full suite ≥198/220; three real project build.sh with CC=acc; stage_b.log
5. Stage C5: two full suite runs; pass sets equal; stage_c_rerun.log
6. Stage C3: raised multiarch bar green on aarch64+x86_64; then 4 ISAs for Status
7. Stage C2: SQLite `testfixture`+`veryquick` **0 errors** and Redis RESP under CC=acc; stage_c_projects.log (no sqlite_reg/SDS PASS)
8. Stage C1: Docker Linux kernel 6.9 + QEMU busybox `/bin/sh` on arm64 **and** x86_64
9. Builtin M2+M4 (core); M5 + Postgres 237 + torture/~99% SCRATCH (Status) before Goal COMPLETE
