# Stage C Completion Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Drive clean-room `ggcc` from Stage A+B green through Stage C hard gates (C1–C5) without downgrade.

**Architecture:** Fail-driven language growth: public/c-testsuite + SQLite amalgamation + kernel force parser/codegen fixes. Frontend→codegen only; system as/ld only on emitted .s/.o. Dual backends (aarch64 + x86_64). Harness locks + progress.md.

**Tech Stack:** Rust, aarch64 Darwin/Linux ELF, x86_64, Docker Linux, QEMU, vendored c-testsuite, SQLite amalgamation 3.45.3, Linux 6.9.

---

## Acceptance (frozen — no shrink)

See repo root `plan.md` and goal plan. Stage C complete only when:

| Gate | Evidence |
|------|----------|
| C1 kernel 6.9 boot | `{SCRATCH}/stage_c_kernel.log` |
| C2 ≥2 large projects | `{SCRATCH}/stage_c_projects.log` |
| C3 dual ISA subset | `{SCRATCH}/stage_c_multiarch.log` |
| C4 clean-room | anti-bypass + mutation |
| C5 double-run | `{SCRATCH}/stage_c_rerun.log` |

**Never** call done on Stage B alone.

---

## Current baseline (2026-07-20)

- Stage A PASS, Stage B PASS (~202/220), C3 PASS, C5 PASS, C4 held.
- C2 BLOCKED: full `sqlite3.c` parse still failing (last: compound `|=` etc.).
- C1 BLOCKED: Docker/ELF stubs only; no full kernel build+boot.

---

### Task 1: Compound bitwise assigns (`|= &= ^= <<= >>=`)

**Files:**
- Modify: `src/token.rs`, `src/lexer.rs`, `src/parser.rs`, `src/codegen.rs`, `src/codegen_x86_64.rs`
- Test: `examples/` or inline smoke under scratch

**Step 1: Mini program**

```c
int main(void) {
  int x = 1;
  x |= 2;
  x &= 3;
  x ^= 1;
  x <<= 1;
  x >>= 1;
  return x;
}
```

**Step 2:** `cargo build --release && ./target/release/ggcc -o /tmp/t t.c && /tmp/t; echo $?`

**Step 3:** Recompile `third_party/stage_c/sqlite/sqlite3.c --target-os linux -S`

**Step 4:** Commit `feat: bitwise compound assign ops`

---

### Task 2: Fail-driven SQLite parse → asm

**Files:** parser/preprocess/codegen as failures dictate; lock `harness/current_tasks/sqlite.lock`

**Loop:**
1. `./target/release/ggcc --target-os linux -S -o {SCRATCH}/sqlite3.s third_party/stage_c/sqlite/sqlite3.c 2>{SCRATCH}/sqlite_err.txt`
2. On error: dump PP context (`GGCC_DUMP_PP`), minimal repro, fix one root cause
3. Until `.s` size > 0 and exit 0
4. Docker link smain + sqlite3.o; run open/exec smoke
5. Append to `{SCRATCH}/stage_c_projects.log`

**Do not** feed sqlite3.c to gcc/clang as compiler.

---

### Task 3: C2 second large project (Redis basic tests)

**Files:** `third_party/stage_c/redis/` (fetch if needed), `harness/real_projects.md` C2 list

**Steps:** After SQLite smoke green, vendor Redis fixed tag; `CC=ggcc` build minimal; run basic tests in Docker Linux if needed; log.

---

### Task 4: C1 Linux 6.9 boot

**Files:** `harness/docker/*`, scripts under `scripts/kernel_boot.sh`

**Steps:**
1. Linux Docker with cross/native tools
2. Kernel 6.9 source; `CC=ggcc` (or wrap that only runs ggcc for C)
3. Config minimal; build bzImage/vmlinux for x86_64 or aarch64
4. QEMU boot; capture dmesg/serial
5. `{SCRATCH}/stage_c_kernel.log` must show boot success string

---

### Task 5: Re-verify C3/C5 after language work

**Commands:**
```bash
bash harness/run_multiarch.sh | tee {SCRATCH}/stage_c_multiarch.log
# double suite
bash harness/run_ctestsuite.sh | tee {SCRATCH}/run1.txt
bash harness/run_ctestsuite.sh | tee {SCRATCH}/run2.txt
# diff pass sets → stage_c_rerun.log
```

---

### Task 6: Harness hygiene

- Update `harness/progress.md` every session
- Task locks for parallel agents
- Mutation + anti-bypass still PASS

---

## Parallel worktree map (dispatching-parallel-agents)

| Worktree | Branch | Owner scope |
|----------|--------|-------------|
| wt-sqlite | feat/sqlite-c2 | Tasks 1–2 only (frontend/codegen for sqlite errors) |
| wt-kernel | feat/kernel-c1 | Task 4 harness/docker scripts + minimal kernel compile experiments |
| wt-suite | feat/suite-harden | Remaining c-testsuite fails + C5 re-run scripts |

Integrate to main after each worktree: merge non-conflicting; re-run sqlite compile after suite merges.

---

## Constraints recap

1. No CCC src reference
2. No external C compiler on user .c
3. No fixture hardcode
4. No acceptance downgrade
5. No done until C1–C5 green
