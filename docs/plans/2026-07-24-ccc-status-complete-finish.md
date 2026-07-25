# CCC-Status COMPLETE Finish Implementation Plan

> **Canonical goal + handoff (read first):** [`docs/HANDOFF_CCC_STATUS_COMPLETE.md`](../HANDOFF_CCC_STATUS_COMPLETE.md)
>
> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> **Parallel:** Use dispatching-parallel-agents — one agent per domain below; **never** two agents editing the same soft-codegen file at once.

**Goal:** Drive clean-room `ggcc` to honest **`Goal: COMPLETE`** under CCC-Status gates (Postgres 237, M5, torture/~99%, C1 serial SCRATCH, ledger/docs aligned) — **without copying Claude CCC compiler `src/`**.

**Status (2026-07-25):** Goal still **NOT COMPLETE**. Most gates PASS (M5, C1 dual-arch, torture, C2/C3, C4/C5). **Sole open Status blocker:** Postgres ecpg `undefined reference to descriptor_type`. See living `harness/progress.md`.

**Architecture:** Keep ggcc as the only C compiler for user `.c`. Finish remaining Status extras as **independent workstreams** that share only honesty artifacts (`progress.md`, ledger). Soft x86_64 codegen has a **single owner** for the Postgres initdb path. CCC **harness / methodology** may be referenced; CCC **compiler body** may not.

**Tech Stack:** Rust ggcc, Docker (`ggcc-linux`), QEMU, Linux 6.9 + busybox, SQLite testfixture, Redis, Postgres 15.7 VPATH build, optional vendored **public** GCC torture sources (not CCC).

---

## Hard constraints (never violate)

1. **Harness OK / compiler body forbidden**
   - **Allowed:** CCC *human-side* methodology — oracle shape, busybox QEMU recipes, ledger format, `BUILDING_LINUX.txt`-class process notes, sparse-checkout of **non-`src/`** trees used as reference only.
   - **Forbidden:** Reading, copying, paraphrasing, or deriving from `anthropics/claudes-c-compiler` **`src/`** (or any CCC compiler implementation). No “port their codegen” / “mirror their AST”.
2. User `.c` → **ggcc only**. System `as`/`ld`/`cc` only on ggcc-emitted `.s`/`.o` until M5 deliberately replaces them for the marker path.
3. Soft Stage-C bars (`ggcc-init`, `sqlite_reg`, SDS, 2-ISA-only) **never** count as COMPLETE.
4. **No** fixture/hardcode PASS. Markers only with SCRATCH evidence.
5. **No** `Goal: COMPLETE` stamp until **every** Status checklist row is green with on-disk SCRATCH.
6. Prefer language/codegen fixes over permanent Postgres workarounds; temporary GGCC_* skips must be listed and removed when soft is fixed.

---

## As-of 2026-07-24 evening SCRATCH (do not re-litigate)

| Gate | Honest status | Evidence / note |
|------|---------------|-----------------|
| C2 SQLite + Redis | **PASS** (keep re-proving) | `scratch/c2_veryquick_summary.txt`, `scratch/c2_redis_marker` |
| C3 4-ISA | **PASS** (Docker healthy) | `scratch/stage_c_4isa.log` → `STAGE_A_4ISA_RUN_COMPLETE` (100/100 ×4) |
| Builtin M2 | **PASS** | `scratch/builtin_m2_marker` |
| Builtin M4 | **PASS** | `scratch/builtin_m4_marker` + `docs/notes/builtin_linker_m4.md` |
| Builtin M5 | **FAIL** | Marker **MISSING**. `execve` OK; runtime `SEGV_ACCERR@0x400148`. See `docs/notes/builtin_m5_requirements.md` + `docs/notes/ccc_status_snapshot.md` |
| Postgres 237 | **BLOCKED** | Linked; initdb child 139 after pristine genam restore. **0/237**. See `docs/notes/postgres_initdb_status.md` |
| C1 serial logs | **GAP** | Dual-arch busybox Status not closed |
| Torture/~99% | **FAIL** | ~50% last known — **not** Status ~99% |
| Ledger / docs | **Synced this evening** | Goal still **NOT COMPLETE** in `harness/progress.md` |

Soft fixes already landed (keep): CompoundAssign `%rbx` spill (`len += *sp++` / pglz); `Expr::AddrOfLabel` for `&&label` / `ExecInterpExpr` dispatch; 12-byte struct-by-value spill.

---

## Parallel agent map (dispatching-parallel-agents)

Run **Wave 1 in parallel** (no shared soft-codegen file). Queue Wave 2 domains that need `codegen_x86_64.rs` behind **one** owner.

| Domain ID | Scope | May edit | Must NOT edit | Wave |
|-----------|--------|----------|---------------|------|
| `D-pg237` | initdb → `make check` 237 | `src/codegen_x86_64.rs`, `src/parser.rs`/`ast.rs` if needed, `third_party/stage_c/postgres/**` markers/workarounds, `scratch/c2_initdb*.log` | C1/M5/torture harnesses | 1 (solo soft owner) |
| `D-c1-serial` | Refresh QEMU serial BusyBox `/#` logs | `harness/docker/build_kernel.sh`, `harness/initrd/**`, `scratch/qemu_boot*.log` | `codegen_*.rs`, Postgres | 1 |
| `D-m5` | Hosted builtin link + marker | `src/linker/**`, `src/driver.rs`, `docs/notes/builtin_m5_requirements.md`, `scratch/builtin_m5_*` | `codegen_x86_64.rs`, Postgres | 1 |
| `D-torture` | Real GCC torture track + SCRATCH | `harness/run_torture_subset.sh` (or new), vendor **public** torture under `third_party/` (not CCC), `scratch/*torture*` | soft codegen, Postgres | 1 |
| `D-honesty` | Ledger + README + progress alignment | `harness/ccc_parity_ledger.md`, `harness/progress.md`, `README.md`, `scratch/c2_gate_matrix.md` refresh | soft codegen (unless only docs) | 1 **after** others land evidence, or parallel read-only then write |
| `D-c5-rerun` | Double-run identical suite | `harness/stage_c_rerun.sh` (create if missing), `scratch/stage_c_rerun.log` | Postgres | 2 (after C3 harness stable) |
| `D-ci` | CI reflects Status gates (not fake green) | `.github/workflows/ci.yml` | Do not weaken gates | 2 |

**Rule:** If two domains need `src/codegen_x86_64.rs` or `src/parser.rs`, **serialize**. Prefer `D-pg237` as the sole soft owner until initdb green.

**CCC reference (allowed harness only):**

```bash
# Optional sparse clone — NEVER checkout src/
git clone --filter=blob:none --sparse \
  https://github.com/anthropics/claudes-c-compiler.git \
  third_party/ccc-harness-ref
cd third_party/ccc-harness-ref
git sparse-checkout set BUILDING_LINUX.txt current_tasks ideas include README.md DESIGN_DOC.md projects
# VERIFY: no src/ in the tree
test ! -d src
```

Use only for process recipes / ledger shape. Implement all compiler behavior in **ggcc** from first principles + public specs/tests.

---

## Wave 1 — Domain `D-pg237` (soft owner)

### Task P1: Reproduce initdb314 crash site without new theories

**Files:**
- Read: `scratch/c2_initdb314.log`
- Read: `third_party/stage_c/postgres/postgresql-15.7/src/backend/utils/activity/pgstat*.c` (existing GGCC markers)
- Test: none yet

**Step 1:** Extract last 40 non-SCC `GGCC_*` lines before `SEGV_simple` and the last `GGCC_Q:`.

**Step 2:** Confirm whether crash is still inside `pgstat_create_relation` after `boot_skip`, or a later unmarked site.

**Step 3:** Write one paragraph to `scratch/c2_pg_next_blocker.md` (create).

**Step 4:** Commit (when executing)

```bash
git add scratch/c2_pg_next_blocker.md
git commit -m "docs: pin postgres initdb next SEGV site after Q250"
```

### Task P2: Minimal failing soft unit for suspected bug class

**Files:**
- Create: `tests/<name>.c` (exact name once bug class known — e.g. struct return, shmem pointer, bool ABI)
- Modify: only if needed `src/codegen_x86_64.rs`

**Step 1:** Write the smallest C program that reproduces the soft miscompile (prefer host Linux docker + soft `.o` + gcc harness, same pattern as pglz/`computed_goto_dispatch.c`).

**Step 2:** Run under soft; expect FAIL/SEGV.

```bash
docker run --rm --platform linux/amd64 -v "$PWD":/work -w /work ggcc-linux \
  bash -lc 'export GGCC=/work/target-linux/release/ggcc GGCC_ARCH=x86_64 GGCC_TARGET_OS=linux
  /work/harness/docker/ggcc_cc_wrapper.sh -O0 -g -o /tmp/t /work/tests/<name>.c && /tmp/t'
```

Expected: non-zero or SEGV **before** fix.

**Step 3:** Implement minimal soft fix.

**Step 4:** Re-run unit → PASS (`OK` / exit 0).

**Step 5:** Commit

```bash
git add tests/<name>.c src/codegen_x86_64.rs
git commit -m "fix(x86_64): <one-line why for postgres initdb>"
```

### Task P3: Rebuild postgres + initdb smoke

**Files:**
- Build tree: `scratch/postgres-build-15.7/` (VPATH from `third_party/stage_c/postgres/postgresql-15.7/`)
- Log: `scratch/c2_initdb315.log` (or next free number)

**Step 1:** Rebuild soft: `CARGO_TARGET_DIR=target-linux cargo build --release` inside `ggcc-linux`.

**Step 2:** Delete stale `.o` / `objfiles.txt` for touched backend units; **never** leave shadowing `scratch/postgres-build-15.7/src/**/*.c`.

**Step 3:** Relink `postgres`; `initdb -D /tmp/pgdata --no-sync -n` as user `pgtest`.

**Step 4:** Success criteria: initdb exits 0 **or** new SEGV site documented with markers (then loop P2–P3). Strip temporary `write(2,…)` markers once past the site.

**Step 5:** Commit only soft/PG delta that is intentional (markers optional).

### Task P4: `make check` toward 237

**Files:**
- Create/update: `scratch/c2_postgres_237_summary.txt`
- Log: `scratch/c2_postgres_make_check.log`

**Step 1:** After initdb green, run Postgres regression under the same soft-built tree (document exact `make check` / `installcheck` command used in docker).

**Step 2:** Parse `All N tests passed` / count; write summary `PASS: 237/237` or `FAIL: k/237` with failing test names.

**Step 3:** Do **not** stamp COMPLETE from this task alone.

**Step 4:** Commit evidence paths into ledger later via `D-honesty`.

---

## Wave 1 — Domain `D-c1-serial`

### Task C1.1: Refresh arm64 serial BusyBox log

**Files:**
- Modify/run: `harness/docker/build_kernel.sh`
- Create: `scratch/qemu_boot_a09.log` (and/or `scratch/qemu_boot.log`)

**Step 1:**

```bash
KERNEL_ARCH=arm64 bash harness/docker/build_kernel.sh 2>&1 | tee scratch/c1_kernel_arm64_build.log
```

**Step 2:** Confirm serial contains BusyBox shell prompt (`/#` or documented `/bin/sh`). Soft `ggcc-init:` alone = **FAIL**.

**Step 3:** Ensure `scratch/c1_boot_marker` still honest.

**Step 4:** Commit evidence (log files + marker if refreshed).

### Task C1.2: Refresh x86_64 serial BusyBox log

**Files:**
- Create: `scratch/qemu_boot_x86_64.log`

**Step 1:**

```bash
KERNEL_ARCH=x86_64 bash harness/docker/build_kernel.sh 2>&1 | tee scratch/c1_kernel_x86_64_build.log
```

**Step 2:** Confirm `/#` (or documented prompt) in `scratch/qemu_boot_x86_64.log`.

**Step 3:** Commit.

---

## Wave 1 — Domain `D-m5`

### Task M5.1: Failing hosted-link smoke (no system cc/ld)

**Files:**
- Create: `tests/builtin_m5_hello.c` with `printf("Hello, world!\n");`
- Create: `harness/docker/run_builtin_m5.sh` (or extend existing M4 runner)
- Read: `docs/notes/builtin_m5_requirements.md`, `docs/notes/builtin_linker_m4.md`

**Step 1:** Script must fail today if it asserts no `cc`/`ld` spawn and expects hello output.

**Step 2:** Run script; expect FAIL (unresolved libc / fallback).

### Task M5.2: Implement minimal hosted link path

**Files:**
- Modify: `src/linker/**`
- Modify: `src/driver.rs` (`GGCC_BUILTIN_LD=1` hosted path; **no silent fallback** when stamping marker)

**Step 1:** Choose one approach and document in `docs/notes/builtin_linker_m4.md` § M5:
- **A:** static musl `.a` resolved by builtin ld, or
- **B:** dynamic `PT_INTERP` + glibc/musl shared objects

**Step 2:** Implement only what is needed for hello `printf` on **aarch64 Linux** (YAGNI other ISAs).

**Step 3:** Re-run smoke → prints `Hello, world!\n`, exit 0, `link=builtin`.

**Step 4:** Write `scratch/builtin_m5_marker` (+ run log). **Do not** invent the marker early.

**Step 5:** Commit

```bash
git add src/linker src/driver.rs docs/notes/builtin_linker_m4.md \
  scratch/builtin_m5_marker scratch/builtin_m5_run.log harness/docker/run_builtin_m5.sh
git commit -m "feat: builtin hosted link M5 hello without system cc/ld"
```

---

## Wave 1 — Domain `D-torture`

### Task T1: Vendor public GCC torture (not CCC)

**Files:**
- Create: `third_party/gcc-torture/` **or** document `TORTURE_DIR` pointing at a public checkout
- Modify: `harness/run_torture_subset.sh`

**Step 1:** Obtain **public** GCC `c-torture` / `gcc.c-torture/execute` sources (FSF GCC tree). **Do not** take tests from CCC `src/` or CCC-private trees.

**Step 2:** Script compiles+runs a documented ID list under ggcc (start small: 50–100), tees `scratch/torture_gcc_subset.log`.

**Step 3:** Summary line: `pass=N fail=M total=T rate=R%`. Track toward ~99% in ledger as **in progress** until rate met.

**Step 4:** Commit harness + log (not huge vendor blobs if policy forbids — then submodule + log only).

### Task T2: Raise subset until honest ~99% claim or blocked_reason

**Files:**
- Update: `scratch/torture_gcc_subset.log`
- Update: ledger row only when rate is real

**Step 1:** Expand ID set; fix soft bugs discovered (**queue soft fixes through `D-pg237` owner** if they touch `codegen_x86_64.rs`; otherwise ISA-local owners).

**Step 2:** Stop when ≥99% on the declared set **or** write `blocked_reason` with failing IDs — never fake.

---

## Wave 1/2 — Domain `D-honesty`

### Task H1: Refresh gate matrix from disk

**Files:**
- Modify: `scratch/c2_gate_matrix.md` (or regenerate)
- Modify: `harness/ccc_parity_ledger.md`

**Step 1:** Re-scan SCRATCH; mark PASS only with existing files.

**Step 2:** Postgres row: `PARTIAL` until 237; C3 PASS if `STAGE_A_4ISA_RUN_COMPLETE` present; M5 TODO until marker; C1 PASS only when serial logs exist.

### Task H2: Align README + progress (still NOT COMPLETE until all green)

**Files:**
- Modify: `harness/progress.md`
- Modify: `README.md`

**Step 1:** Update tables to match SCRATCH (C3 green, M4 green, M5/PG/torture/C1-serial honest).

**Step 2:** Keep `## Goal: **NOT complete**` until Task Z1.

**Step 3:** Commit docs-only.

---

## Wave 2 — Domain `D-c5-rerun` + `D-ci`

### Task R1: Double-run harness

**Files:**
- Create: `harness/stage_c_rerun.sh` if missing
- Create: `scratch/stage_c_rerun.log`

**Step 1:** Run Stage A 00001–00100 twice; require identical pass ID sets.

**Step 2:** Tee evidence; fail if drift.

### Task CI1: CI must not claim COMPLETE

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1:** Ensure CI runs mutation + a cheap oracle subset; optional artifact upload of markers.

**Step 2:** Badge stays “in progress” until Goal COMPLETE; do not add a lying green COMPLETE job.

---

## Final gate — stamp COMPLETE (only when all green)

### Task Z1: COMPLETE checklist

**Files:**
- Modify: `harness/progress.md`
- Modify: `README.md` badge/status

**Step 1:** Verify on disk:

```bash
test -f scratch/stage_c_4isa.log && grep -q STAGE_A_4ISA_RUN_COMPLETE scratch/stage_c_4isa.log
test -f scratch/builtin_m4_marker && test -f scratch/builtin_m5_marker
test -f scratch/qemu_boot_a09.log && test -f scratch/qemu_boot_x86_64.log
grep -q '/#' scratch/qemu_boot_a09.log scratch/qemu_boot_x86_64.log
test -f scratch/c2_postgres_237_summary.txt && grep -q '237/237' scratch/c2_postgres_237_summary.txt
test -f scratch/torture_gcc_subset.log  # and rate ≥99% per declared contract
test -f scratch/stage_c_rerun.log
```

**Step 2:** Only then set `## Goal: **COMPLETE**` and refresh CCC Status badge.

**Step 3:** Commit

```bash
git add harness/progress.md README.md harness/ccc_parity_ledger.md
git commit -m "docs: stamp CCC-Status COMPLETE with SCRATCH evidence"
```

---

## Suggested parallel dispatch (copy-paste)

```text
Agent D-pg237  → Tasks P1–P4 (sole codegen_x86_64 owner)
Agent D-c1     → Tasks C1.1–C1.2
Agent D-m5     → Tasks M5.1–M5.2
Agent D-torture→ Tasks T1–T2 (no soft codegen)
# After evidence lands:
Agent D-honesty→ Tasks H1–H2
# After language stable:
Agent D-c5/CI  → Tasks R1, CI1
# Only when checklist green:
Agent Z        → Task Z1
```

---

## Out of scope (YAGNI for this plan)

- FFmpeg / other megaprojects beyond Status ledger honesty
- Copying CCC DWARF/as/ld **implementations**
- Claiming COMPLETE from soft Stage-C or partial initdb
- Parallel edits to `codegen_x86_64.rs`

---

## References (ggcc-owned)

- `plan.md` — Completion definition
- `harness/STAGE_CONTRACTS.md` — frozen gates
- `docs/plans/2026-07-23-ccc-full-parity.md` — earlier roadmap (superseded for *finish* sequencing by this doc)
- `docs/notes/builtin_m5_requirements.md`, `docs/notes/builtin_linker_m4.md`
- `scratch/c2_initdb314.log` — current PG high-water
- `scratch/stage_c_4isa.log` — C3 done
