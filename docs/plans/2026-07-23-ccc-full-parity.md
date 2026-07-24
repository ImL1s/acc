# CCC Full Parity Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> **Parallel:** Use dispatching-parallel-agents — one agent per independent domain (never two agents editing `src/codegen.rs` at once).

**Goal:** Raise clean-room `ggcc` from the current soft Stage-C stamp to **CCC-level capability and architecture**: official SQLite/Redis suites, busybox-shell Linux boots on multiple ISAs, fuller multiarch oracles, then builtin assembler/linker/DWARF and i686/RISC-V — **without copying** `anthropics/claudes-c-compiler` `src/`.

**Architecture:** Keep ggcc frontend→codegen as the only C compiler. Reuse CCC **non-`src/`** process artifacts (sparse-checkout: `BUILDING_LINUX.txt`, `current_tasks/`, `ideas/`, README Status ledger shape). System `as`/`ld` remain until Phase E gates flip. Kernel freestanding shrinks only when real bodies boot. Progress honesty: Goal stays **NOT complete** until Phase B (strict C2) + Phase C (busybox C1) green; later phases are required for “全部 CCC” claim.

**Tech Stack:** Rust ggcc, Docker Linux, QEMU, Linux 6.9, busybox, SQLite + Tcl testfixture, Redis 7.x, c-testsuite, optional CCC sparse tree under `third_party/ccc-harness-ref/` (no `src/`).

---

## Hard constraints (never violate)

1. **No** read/copy/reference of CCC `src/` (or any compiler body).
2. User `.c` → ggcc only. System `cc`/`as`/`ld` only on ggcc-emitted `.s`/`.o` until builtin toolchain replaces them deliberately.
3. **No** fixture/hardcode PASS. **No** `sqlite_reg` / SDS as C2 PASS after Phase A.
4. Multi-agent OK; locks in `harness/current_tasks/`; status in `harness/progress.md`.
5. Prefer language/codegen fixes over soft freestanding stubs.

## CCC bar we are chasing (public Status — capability + architecture)

| Area | CCC claim | ggcc today | Target |
|------|-----------|------------|--------|
| C2 SQLite | Official test suite | `sqlite_reg` 38/0 | `testfixture` + `veryquick` |
| C2 Redis | Test suite / server | SDS only | `redis-server` RESP PING/SET/GET |
| C1 Linux | Busybox `/bin/sh` multi-ISA | arm64 linked EL1 payload | arm64 busybox shell → x86_64 → riscv |
| Oracles | ~99% / torture | A≥95%/100, B≥90%/220 | Raise multiarch + suite % stepwise |
| ISAs | 4 (x86-64, i686, aarch64, riscv64) | 2 | Add i686 + riscv64 |
| Toolchain | Builtin as/ld/DWARF | System `cc` on `.s` | Builtin milestones M1–M5 |
| Extras | Postgres 237, FFmpeg, … | none | Phase F after core |

## Parallel agent map (dispatching-parallel-agents)

Independent domains — **run in parallel only when file sets do not overlap**:

| Domain ID | Scope | Primary files | Blocks on |
|-----------|--------|---------------|-----------|
| `D-honesty` | Demote COMPLETE; tighten contracts | `progress.md`, `STAGE_CONTRACTS.md`, `real_projects.md`, `run_c2_projects.sh` | — |
| `D-wrapper` | Link order `-lm` trailing | `ggcc_cc_wrapper.sh` | — |
| `D-sqlite` | testfixture / tcl / zipfile | wrapper includes, `codegen` only if needed, C2 script | `D-wrapper` for link |
| `D-redis` | libm + `callReply*` + RESP | `codegen.rs` emission, wrapper, C2 script | `D-wrapper`; **serialize** vs `D-sqlite` if both touch `codegen.rs` |
| `D-c1-busybox` | EL0 + busybox initrd | `codegen.rs` freestanding, `build_kernel.sh`, `harness/initrd/` | Prefer after C2 or separate worktree |
| `D-c3` | Expand multiarch IDs | `run_multiarch.sh`, `STAGE_CONTRACTS.md`, x86 backend | — |
| `D-isa` | i686 / riscv backends | new `codegen_*.rs`, `driver.rs` | After C3 expansion green on 2 ISAs |
| `D-builtin` | as / ld / DWARF | new modules + `driver.rs` | After language gates stable |

**Rule:** If two domains need `src/codegen.rs`, queue them; do not parallel-edit.

## CCC harness reuse (allowed)

```bash
# One-time sparse clone WITHOUT src/
git clone --filter=blob:none --sparse \
  https://github.com/anthropics/claudes-c-compiler.git \
  third_party/ccc-harness-ref
cd third_party/ccc-harness-ref
git sparse-checkout set BUILDING_LINUX.txt current_tasks ideas include README.md DESIGN_DOC.md projects
# NEVER checkout src/
```

Use as **reference only** for busybox QEMU recipe and project PASS ledger format. Mirror ledger into `harness/ccc_parity_ledger.md`.

---

## Phase 0 — Honesty reset (do first, same day)

### Task 0.1: Demote Goal COMPLETE

**Files:**
- Modify: `harness/progress.md`
- Modify: `README.md` (Stage C status line)

**Step 1:** Set progress header to:

```markdown
## Goal: **NOT complete** — CCC full parity in progress

blocked_reason: Raised bar to CCC Status; C2 still sqlite_reg/SDS; C1 not busybox shell; no builtin as/ld; 2/4 ISAs
```

**Step 2:** Document that prior COMPLETE was ggcc Stage-C soft bar only.

**Step 3:** Commit

```bash
git add harness/progress.md README.md
git commit -m "docs: demote COMPLETE; raise bar to CCC full parity"
```

### Task 0.2: Freeze strict contracts

**Files:**
- Modify: `harness/STAGE_CONTRACTS.md`
- Modify: `harness/real_projects.md`
- Modify: `plan.md` Stage C section (clarify CCC-full vs old soft bar)

**Step 1:** Replace C2 wording with:

- SQLite: official `testfixture` + `test/veryquick.test` under ggcc (no `sqlite_reg` PASS).
- Redis: built `redis-server` + RESP `PING`/`SET`/`GET` (no SDS PASS).
- C1: serial must show busybox `/bin/sh` (or documented shell prompt), not only `ggcc-init:`.

**Step 2:** Commit with message `docs: freeze CCC-strict Stage C contracts`.

### Task 0.3: Kill C2 smoke fallbacks in harness

**Files:**
- Modify: `harness/docker/run_c2_projects.sh`

**Step 1:** Remove `sq_ok` from `sqlite_reg`; remove `rd_ok` from `PASS_REDIS_SDS*`.

**Step 2:** Require:

- `testfixture` exists and `veryquick` log contains suite summary.
- `c2_redis_marker` == `PASS_REDIS_DEFAULT_LATENCY` from live RESP.

**Step 3:** Run script expecting **FAIL** (red before green).

**Step 4:** Commit `test: require testfixture+RESP for C2 PASS`.

---

## Phase A — Shared wrapper fixes (unblocks SQLite + Redis)

### Task A.1: Trailing `-lm` on all link paths

**Files:**
- Modify: `harness/docker/ggcc_cc_wrapper.sh` (no-`.c` and single-`.c` link; ~332–450)

**Step 1:** Write a tiny Docker link repro: one `.o` calling `floor` + archive needing `-lm`; expect fail with current order.

**Step 2:** Change link line to: **objects/archives first**, then `${passthru_sys}`, then force trailing `-lm -ldl -lpthread`.

**Step 3:** Re-run Redis `make` link; expect libm undefs gone (callReply may remain).

**Step 4:** Commit `fix(wrapper): put -lm after archives on all link paths`.

### Task A.2: Quoted include + `-I` for system headers

**Files:**
- Modify: `src/preprocess.rs` and/or driver include search (as needed)
- Modify: `harness/docker/ggcc_cc_wrapper.sh` if `-isystem` not forwarded

**Step 1:** Minimal `echo '#include "tcl.h"'` compile with `-I/usr/include/tcl8.6` under wrapper → must succeed.

**Step 2:** Fix search path for `"..."` includes to honor `-I` (and optionally `-isystem`).

**Step 3:** Commit `fix: honor -I for quoted includes (tcl.h)`.

---

## Phase B — Strict C2 (parallel domains after A)

### Task B.1 Domain `D-sqlite`: Build testfixture

**Files:**
- Modify: `harness/docker/run_c2_projects.sh`
- Possibly: `harness/c2/stubs.c` (last resort only)
- Codegen only if emit/link of `zipfile*.c` fails

**Step 1:** In Docker: `make testfixture CC=ggcc_cc_wrapper` with tcl-dev installed.

**Step 2:** On first compile/link error, fix **language** (prefer) or include/link; do not stub away suite.

**Step 3:** Run `./testfixture test/veryquick.test` → write `scratch/c2_sqlite_veryquick.log`.

**Step 4:** Gate: log must show real pass/fail counts; `nfail=0` for veryquick (or document CCC-comparable known skips only in ledger, never silent).

**Step 5:** Commit when green `feat(c2): SQLite testfixture veryquick under ggcc`.

### Task B.2 Domain `D-redis`: Emit callReply + RESP

**Files:**
- Modify: `src/codegen.rs` / parser if symbols dropped
- Modify: `harness/docker/run_c2_projects.sh`

**Step 1:** `nm` on `call_reply.o` under ggcc — confirm missing `callReplyCreate`, `freeCallReplyInternal`, `callReplyAttribute`, `callReplyGetAttribute`.

**Step 2:** Write failing unit/oracle that compiles a reduced `call_reply` snippet exporting those names.

**Step 3:** Fix codegen/sema so bodies emit (static/inline/visibility bugs are common).

**Step 4:** Full `make` redis-server; start server; `PING`→`PONG`, `SET foo bar`→`+OK`, `GET foo`→`bar`.

**Step 5:** Stamp `PASS_REDIS_DEFAULT_LATENCY` only from that path.

**Step 6:** Commit `feat(c2): Redis RESP basic under ggcc`.

### Task B.3: C2 SCRATCH + progress

**Files:**
- Overwrite: `scratch/stage_c_projects.log`
- Modify: `harness/progress.md` (C2 PASS only; Goal still NOT complete until Phase C)

**Step 1:** Re-run full C2 harness; confirm no SDS/`sqlite_reg` PASS language.

**Step 2:** Update progress C2 row; keep Goal NOT complete.

---

## Phase C — C1 busybox shell (CCC BUILDING_LINUX shape)

### Task C.1: Busybox initrd recipe

**Files:**
- Create/Modify: `harness/initrd/build_busybox_initrd.sh`
- Modify: `harness/docker/build_kernel.sh` QEMU `-initrd`

**Step 1:** Build static busybox; cpio with `/init` → `setsid cttyhack /bin/sh` (mirror CCC `BUILDING_LINUX.txt` flow from sparse ref).

**Step 2:** QEMU aarch64 with existing Image + new initrd; capture serial.

**Step 3:** Until EL0 works, expect hang — that is the next task.

### Task C.2: Shrink freestanding enough for EL0 exec

**Files:**
- Modify: `src/codegen.rs` freestanding keepers (`rest_init`, `run_init_process`, VFS/binfmt-related no-ops ~4312+)
- Modify: `harness/docker/KERNEL_STATUS.md`, `BUILDING_LINUX.txt`

**Step 1:** Stop calling linked `ggcc_real_init_payload` as pid1; restore path toward real `kernel_init` / `run_init_process` on initrd `/init`.

**Step 2:** Re-enable real bodies incrementally; keep early paging keepers only while proven necessary.

**Step 3:** PASS when serial shows shell prompt (`/#` or busybox banner) **and** `Linux version`.

**Step 4:** Update `PASS_BOOT` grep in `build_kernel.sh` accordingly.

**Step 5:** Commit `feat(c1): arm64 busybox /bin/sh boot under ggcc`.

### Task C.3: x86_64 kernel boot (after arm64 busybox)

**Files:**
- Modify: `src/codegen_x86_64.rs` (keepers / printk as needed)
- Modify: `harness/docker/build_kernel.sh` (`KERNEL_ARCH=x86_64`)

**Step 1:** Same busybox bar on x86_64 QEMU.

**Step 2:** Evidence in `scratch/stage_c_kernel_x86_64.log`.

---

## Phase D — Multiarch oracles (C3 raise)

### Task D.1: Expand C3 20 → Stage A (100) both ISAs

**Files:**
- Modify: `harness/STAGE_CONTRACTS.md`
- Modify: `harness/run_multiarch.sh`

**Step 1:** IDS = 00001–00100; require ≥95% both aarch64 and x86_64 with real run.

**Step 2:** Fix x86_64 failures first (smaller backend).

**Step 3:** Commit when `scratch/stage_c_multiarch.log` green.

### Task D.2: Expand toward Stage B (≥90% of 220 both ISAs)

Same files; raise bar; fix fail-driven.

### Task D.3: (Later) GCC torture / 99% track

Add `harness/run_torture_subset.sh` only after D.2; track in ledger — do not block Phase E start if C2+C1 busybox already green.

---

## Phase E — New ISAs + builtin toolchain (architecture parity)

### Task E.1: `Target::I686` + `codegen_i686.rs`

**Files:**
- Create: `src/codegen_i686.rs`
- Modify: `src/driver.rs`, `src/main.rs`, `codegen` dispatch
- Modify: `ggcc_cc_wrapper.sh` — **stop** mapping `i386` → x86_64

**Step 1:** Hello + c-testsuite subset under `qemu-i386`.

**Step 2:** Commit when subset green.

### Task E.2: `Target::Riscv64` + `codegen_riscv.rs`

Same pattern with `qemu-riscv64`.

### Task E.3: Builtin assembler M1–M2

**Files:**
- Create: `src/assembler/` (start aarch64 or x86_64 only)
- Modify: `src/driver.rs` — feature flag `builtin_assembler`

**Milestones:**
- **M1:** `.s` → ELF `.o` for C3 subset one ISA.
- **M2:** Link executable + libc without system `cc` for that subset.
- Keep system fallback until M2 green.

### Task E.4: Builtin linker M2–M3 + second ISA

**Files:**
- Create: `src/linker/`

### Task E.5: DWARF M4

**Files:**
- Create: `src/dwarf/` — line tables enough for basic debug oracles.

### Task E.6: Kernel objects via builtin path M5

Only after M2–M3; optional for “全部” marketing but required before claiming CCC Status toolchain sentence.

---

## Phase F — CCC project ledger expansion (after B+C+E.2)

Track in `harness/ccc_parity_ledger.md` (format like CCC `ideas/new_projects.txt`):

| Project | Bar | Order |
|---------|-----|-------|
| zlib / lua / QuickJS | build + tests | early |
| PostgreSQL | 237 regression | after language maturity |
| FFmpeg FATE | optional mega | last |
| musl / TCC / DOOM | ledger | as capacity allows |

One project per agent when independent; never claim PASS without SCRATCH log.

---

## Completion definition (only — “全部”)

Goal COMPLETE **only when all** are true:

1. Phase 0 honesty applied (no soft C2).
2. Phase B: SQLite `testfixture`+`veryquick` + Redis RESP evidence in SCRATCH.
3. Phase C: arm64 **and** x86_64 busybox-shell boots; riscv when E.2 exists.
4. Phase D: multiarch ≥ Stage A (100) both existing ISAs; path to B documented green or green.
5. Phase E: i686 + riscv64 backends ship; builtin as/ld at least **M2** on ≥1 ISA (system fallback allowed for kernel until M5).
6. C4 anti-bypass + mutation still PASS; C5 double-run identity.
7. `harness/progress.md` Goal COMPLETE + `ccc_parity_ledger.md` matches SCRATCH.

Until then: `Goal: NOT complete` + `blocked_reason`.

---

## Suggested first execution week (subagent-driven)

1. Human/agent: Tasks 0.1–0.3 + A.1–A.2 (serial).
2. Parallel: `D-sqlite` (B.1) ∥ `D-redis` (B.2) **if** codegen conflicts → Redis waits.
3. Then `D-c1-busybox` (C.1–C.2).
4. Parallel: `D-c3` (D.1) while C1 iterates (no shared files).

## Verification commands (recurring)

```bash
export SCRATCH="$PWD/scratch"
cargo build --release
bash scripts/anti_bypass_audit.sh
bash harness/mutation_check.sh
# C2 strict
bash harness/docker/run_c2_projects.sh   # must FAIL until B done; PASS only on testfixture+RESP
# C1
KERNEL_ARCH=arm64 JOBS=4 bash harness/docker/build_kernel.sh
# C3
bash harness/run_multiarch.sh
```

## Out of scope for copying

- Any file under CCC `src/`
- Bit-for-bit CCC IR/pass names
- Claiming COMPLETE on SDS/`sqlite_reg`/EL1 payload again
---
