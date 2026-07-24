# i686 backend (A11 scaffold → B03 minimal emit)

**Branch / worktree:** `wt/i686`  
**Status (B03):** **minimal `emit_assembly` works** — `oracles/hello` → AT&T `.s` → `gcc -m32 -no-pie` → **`qemu-i386` prints `Hello, world!`**.  
**Not wired:** `Target::I686` / `driver` / `main` / wrapper remap (integrator).

## Paths

| Path | Role |
|------|------|
| `src/codegen_i686.rs` | ILP32 SysV cdecl emitter (`pub fn emit_assembly`) |
| `src/codegen.rs` | `pub mod i686` only (no `Target::I686` dispatch) |
| `docs/notes/i686_backend.md` | This note |
| `scratch/hello_i686.s` | Emitted asm (from unit test) |
| `scratch/verify_i686_hello.sh` | Docker: `gcc -m32 -no-pie` + `qemu-i386` |

**Not touched (integrator later):** `src/driver.rs`, `src/main.rs`, `Target` enum / `emit_assembly_for_os` arm, `harness/docker/ggcc_cc_wrapper.sh`.

## What works (B03 evidence)

1. **Unit tests:** `cargo test --bin ggcc i686::` → 4/4 pass (`hello_emits_printf_and_string`, `write_hello_oracle_asm`, …).
2. **Emit:** `oracles/hello/main.c` → `scratch/hello_i686.s` (`.globl main`, `call printf`, `.asciz "Hello, world!\n"`).
3. **Run:** `bash scratch/verify_i686_hello.sh` (or Docker snippet below) → `PASS_I686_HELLO` / `stdout=[Hello, world!]` / `ec=0`.

```bash
# from worktree root
bash scratch/verify_i686_hello.sh
```

Manual Docker (host is arm64 macOS — no local `qemu-i386` / `gcc -m32`):

```bash
docker run --rm --platform linux/amd64 \
  -v "$PWD/scratch:/work" -w /work ubuntu:22.04 bash -c '
    apt-get update -qq && apt-get install -qq -y gcc-multilib qemu-user
    gcc -m32 -no-pie hello_i686.s -o hello_i686
    qemu-i386 ./hello_i686
  '
```

## ABI — ILP32 System V (i386)

Primary target: **Linux ELF** (`qemu-i386` / `gcc -m32`). Darwin i386 out of scope.

| Item | Choice |
|------|--------|
| Data model | **ILP32** — `sizeof(int) = sizeof(long) = sizeof(void*) = 4` |
| Calling convention | **cdecl** — all args on stack (RTL push); caller cleans |
| Return | `%eax` |
| Frame | `%ebp`; save `%ebx` at `-4(%ebp)` |
| Stack align | **16-byte** SP at `call` (pad so total push ≡ 12 mod 16 after prologue) |
| Addressing | **Absolute** (`movl $l_str_N, %eax`) — requires **`-no-pie`** link |
| Asm | AT&T for `gcc -m32 -c` |

## Host / image blockers (documented)

| Environment | Blocker | Workaround |
|-------------|---------|------------|
| macOS arm64 host | No `gcc -m32`, no user-mode `qemu-i386` (only `qemu-system-i386`) | Docker `ubuntu:22.04` + `gcc-multilib` + `qemu-user` |
| `ggcc-linux-amd64:latest` | No multilib (`Scrt1.o` / `-lgcc` for `-m32`); no `qemu-i386` | Use `ubuntu:22.04` for smoke (or extend image later) |
| CLI `-m i686` | **Not wired** — integrator must add `Target::I686` + driver `-m32` | Smoke via `codegen::i686::emit_assembly` + verify script |

## Coverage of this minimal emitter

**Supported (enough for hello + simple control flow):** functions, locals, return, calls (stack ABI), strings, ints, unary/binary/assign, if/while/do/for/break/continue/goto, sizeof, cast, index/member (basic), soft stubs for empty reachable statics.

**Not yet (will `Err` or soft-zero):** full varargs ABI beyond libc `printf`, x87/SSE FP, PIC/`@GOTOFF`, switch/case tables, rich global init lists, freestanding memops, Darwin Mach-O.

## Required wrapper change (do **not** apply until CLI wired)

Today (`harness/docker/ggcc_cc_wrapper.sh`):

```sh
case "$ARCH" in
  x86_64|x86|i386) GGCC_M=x86_64 ;;
  ...
esac
```

**Required when backend is live via CLI:**

```sh
case "$ARCH" in
  x86_64|x86) GGCC_M=x86_64 ;;
  i386|i686)  GGCC_M=i686 ;;
  arm64|aarch64) GGCC_M=aarch64 ;;
  *) GGCC_M=x86_64 ;;
esac
```

## Merge instructions for integrator

### 1. Land files from `wt/i686`

- `src/codegen_i686.rs`
- `docs/notes/i686_backend.md`
- optional: `scratch/verify_i686_hello.sh`

### 2. Wire dispatch (`src/codegen.rs`)

`pub mod i686` is already present on this branch. Add:

```rust
pub enum Target {
    #[default]
    Aarch64,
    X86_64,
    I686,   // NEW
}

// parse: "i686" | "i386" => Some(Self::I686)
// as_str: I686 => "i686"

// emit_assembly_for_os:
//   Target::I686 => i686::emit_assembly(prog),
```

### 3. CLI (`src/main.rs`) — integrator only

- `-m i686` / `i386`; usage strings.

### 4. Driver (`src/driver.rs`) — integrator only

- Map `Target::I686` → assemble/link with `cc -m32 -no-pie` (Linux). Skip Darwin `-arch` for i686.

### 5. Wrapper remap — after CLI path green

### 6. Next gates

1. c-testsuite subset under `qemu-i386`.
2. Then commit / merge to main.

```bash
cd /path/to/ggcc
git fetch . wt/i686
git merge wt/i686
```

## Out of scope for B03

- Editing `driver.rs` / `main.rs` / wrapper remap
- Commit on `main`
- CCC `src/`
- Full ISA parity with x86_64 backend
