# Builtin M5 — hosted link without system `cc`/`ld`

**Status:** IN PROGRESS — do **not** create `scratch/builtin_m5_marker` until docker smoke is green.

## Approach (chosen)

**Static musl (Option A)** — not dynamic `PT_INTERP`.

Link order mirrors `musl-gcc -static`:

`Scrt1.o crti.o <user.o> crtn.o --start-group libgcc.a libc.a --end-group`

System paths (aarch64 Linux docker):

- `/usr/lib/aarch64-linux-musl/` — crt + `libc.a`
- `/usr/lib/gcc/aarch64-linux-gnu/13/libgcc.a`

Override via `GGCC_MUSL_LIB_DIR`, `GGCC_LIBGCC_A`.

## Enable

```sh
cargo build --features builtin_linker --release

GGCC_BUILTIN_AS=1 GGCC_BUILTIN_LD=1 GGCC_BUILTIN_LD_STRICT=1 \
  ./target/release/ggcc -m aarch64 --target-os linux -o hello tests/builtin_m5_hello.c
```

| Knob | Effect |
|------|--------|
| `GGCC_BUILTIN_LD=1` | Use in-tree linker when any input has hosted undefined symbols |
| `GGCC_BUILTIN_LD_STRICT=1` | No silent fallback to system `cc`/`ld` (required for marker path) |
| `GGCC_DEBUG_M5=1` | Linker debug (GOT layout, key symbol VAs) |

## Marker path (when green)

`scratch/builtin_m5_marker` — same honesty shape as M2/M4.  
Harness: `harness/docker/run_builtin_m5.sh` (strace asserts no `cc`/`ld`, run prints `Hello, world!`).

## Requirements

1. Builtin link of ggcc hello (`printf("Hello, world!\n")`) without spawning system `cc`/`as`/`ld`.
2. Resolve libc via static musl archives.
3. Evidence: docker aarch64 run prints `Hello, world!\n`, exit 0.
4. Driver: `GGCC_BUILTIN_AS=1` + `GGCC_BUILTIN_LD=1` + strict mode for marker.

## Implementation notes (2026-07-24 evening)

- `src/linker/aarch64_hosted.rs` — archive-driven musl static link, GOT fill, crt section order (`.init` → `.text` → `.fini`), `.text.*` → `.text`.
- `src/linker/archive.rs` — Unix `ar` reader.
- `tests/builtin_m5_hello.c` — smoke source.
- Env aliases: `ACC_BUILTIN_*` and `GGCC_BUILTIN_*` both accepted.
- Harness: `harness/docker/run_builtin_m5.sh`.

### Fixes landed (still no marker)

| Fix | Result |
|-----|--------|
| `link_files` chmod 0755 | binary is executable |
| PT_LOAD `(p_vaddr - p_offset) % p_align == 0` | `execve` no longer `EINVAL` |
| `_DYNAMIC` → real DT_NULL `.dynamic` in RW (not `LOAD_BASE`) | Scrt1 ADRP lands off ELF header |
| `e_version = 1` | ELF header Version current |
| CRT | `Scrt1.o` (matches `musl-gcc -static`) |

### Still open

- Runtime **`SEGV_ACCERR` at `0x400148`** (branch in `_start`), exit 139.
- Same user `.o` linked with system `as` + `musl-gcc -static` prints `Hello, world!` — codegen/asm OK; builtin ELF layout/relocs still wrong.
- **Do not** stamp `scratch/builtin_m5_marker` until strict path Hello works.
