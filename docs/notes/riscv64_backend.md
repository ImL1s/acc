# RISC-V 64 backend (agent A12 scaffold → B04 minimal emitter)

**Branch / worktree:** `wt/riscv`  
**Emitter:** `src/codegen_riscv.rs` (promoted from `.stub`)  
**Status (B04):** **minimal subset works under `qemu-riscv64-static`** via Docker smoke — **not** wired into `driver.rs` / `main.rs` / `Target` (integrator owns merge).  
**Hard rule:** no CCC `src/` copy or read; clean-room from ggcc `codegen_x86_64.rs` patterns + public RISC-V ABI.

## What works (B04 evidence)

Hand-built AST → `emit_assembly` → `riscv64-linux-gnu-gcc -static` → `qemu-riscv64-static`:

| Smoke | Path | Result |
|-------|------|--------|
| return_code | `harness/riscv64_smoke/out/return_code.s` | exit **7** |
| arith (`10+20+12`) | `…/arith.s` | exit **42** |
| multi_fn (`add(20,22)-42`) | `…/multi_fn.s` | exit **0** |
| hello (`printf`) | `…/hello.s` | stdout `Hello, world!`, exit **0** |

Reproduce:

```bash
cd ggcc-wt-riscv
cargo test --manifest-path harness/riscv64_smoke/Cargo.toml
./harness/riscv64_smoke/smoke_all.sh   # emit + Docker qemu (apt once per run)
```

Or emit one + run:

```bash
cargo run --manifest-path harness/riscv64_smoke/Cargo.toml -- hello harness/riscv64_smoke/out/hello.s
./harness/riscv64_smoke/run_qemu.sh harness/riscv64_smoke/out/hello.s 0
```

**Host blocker (clear):** macOS host has `qemu-system-riscv64` but **no** `qemu-riscv64` user-mode and **no** `riscv64-linux-gnu-*`. Userspace proof uses Docker `ubuntu:24.04` + `gcc-riscv64-linux-gnu` + `qemu-user-static`.

## Supported subset (emitter)

- LP64 sizes; Linux ELF symbols (no `_` prefix)
- Prologue/epilogue: `ra`/`s0`, 16-byte SP align; locals below saved regs (`stack_size` starts at 16)
- `int` locals, assign, return; binary arith / compares; if / while / for (basic)
- Calls ≤8 args (`a0`–`a7`); `.rodata` strings via `lla` + `call`
- Nested functions with params

**Not yet:** full x86_64 parity (structs, float/lp64d hard-float emit, >8 args, switch/goto, frontend `-m riscv64` path).

## Goal (integrator)

Ship `Target::Riscv64` so Stage C can claim **4 ISAs**. Userspace: compile + `qemu-riscv64 ./a.out`. Kernel busybox later: `qemu-system-riscv64` (Phase C after E.2).

## Data model — LP64

Linux `riscv64-linux-gnu` / `lp64` (integer) and `lp64d` (hard-float double):

| Type | Size | Align |
|------|------|-------|
| `char` / `_Bool` | 1 | 1 |
| `short` | 2 | 2 |
| `int` | **4** | 4 |
| `long` | **8** | 8 |
| `long long` | 8 | 8 |
| pointer / `size_t` | **8** | 8 |
| `float` | 4 | 4 |
| `double` | 8 | 8 |

Contrast: i686 is **ILP32** (`long` = 4). Do not reuse i686 layout helpers blindly.

## Calling convention (LP64 / LP64D)

- **Args (int/ptr):** `a0`–`a7` (8). Excess on stack, 16-byte aligned frame.
- **Return:** `a0` (and `a1` for 128-bit / large struct split per ABI).
- **FP args/return (lp64d):** `fa0`–`fa7` / `fa0`. Prefer **lp64d** to match common Debian/Ubuntu sysroot and qemu user images.
- **Callee-saved:** `s0`–`s11`, `fs0`–`fs11`; `s0` = frame pointer when used.
- **Link / stack:** `ra`, `sp`; stack grows down; **16-byte** SP alignment at call sites.
- **Syscalls (Linux):** `a7` = nr, args in `a0`–`a5`, `ecall`; return in `a0` (error via `-errno` convention).

Asm dialect: GNU as / `riscv64-linux-gnu-as` (or Clang `--target=riscv64-linux-gnu`).

## Files owned by this worktree

| Path | Role |
|------|------|
| `src/codegen_riscv.rs` | Minimal LP64 emitter (`emit_assembly`) |
| `docs/notes/riscv64_backend.md` | This note |
| `harness/riscv64_smoke/` | Standalone crate + Docker qemu harness (no driver wire) |

Do **not** modify on `wt/riscv` until integrator PR: `src/driver.rs`, `src/main.rs`, `src/codegen.rs` Target enum, `harness/docker/ggcc_cc_wrapper.sh`.

## Integrator merge notes

Land order: **new `codegen_*.rs` first** → **single integrator PR** for Target / driver / main / wrapper.

### 1. Backend already promoted

```text
src/codegen_riscv.rs   # present on wt/riscv
```

Keep `pub fn emit_assembly(prog: &Program) -> Result<String, String>`.

### 2. Wire in `src/codegen.rs` (integrator only)

```rust
#[path = "codegen_riscv.rs"]
mod riscv;

// Target enum:
Riscv64,

// Target::parse:
"riscv64" | "riscv" | "rv64" => Some(Self::Riscv64),

// Target::as_str:
Self::Riscv64 => "riscv64",

// emit_assembly_for_os:
Target::Riscv64 => riscv::emit_assembly(prog),
```

### 3. `src/main.rs`

- Usage / `-m` help: add `riscv64`.
- Error strings: accept `riscv64` alongside `aarch64` / `x86_64` (and i686 if A11 landed).

### 4. `src/driver.rs`

- `arch` string for preprocess: `"riscv64"`.
- Assemble/link: Linux ELF — prefer `riscv64-linux-gnu-gcc` or env override; do **not** pass Darwin `-arch` for this target.
- Match X86_64 Linux path: `cc -o out file.s -lm` only when host/sysroot is already riscv64; otherwise document `GGCC_LINKER` / cross prefix.

### 5. Wrapper `harness/docker/ggcc_cc_wrapper.sh`

```text
riscv64|riscv|rv64gc|riscv64-*)  GGCC_M=riscv64
```

### 6. Harness

- Extend `harness/run_multiarch.sh` after emitter is wired.
- `STAGE_CONTRACTS.md`: riscv64 joins Stage A only with SCRATCH + qemu evidence (smoke above counts as early evidence).

### 7. Merge / conflict hygiene

- Parallel with A11 (`wt/i686`): no shared file writes until integrator.
- Rebase `wt/riscv` onto updated `main` before integrator PR.
- Single PR touching `Target` + driver + main + wrapper; backend body already on `wt/riscv`.

## Out of scope until integrator / later waves

- Editing hot locks: `codegen.rs`, `driver.rs`, `main.rs`.
- Builtin assembler/linker (A13–A15).
- Claiming C3/C1 riscv green without full oracle harness via `-m riscv64`.
