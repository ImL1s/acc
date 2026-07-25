# Builtin M5 — hosted link without system `cc`/`ld`

**Status:** PASS — `scratch/builtin_m5_marker` stamped (strict docker Hello).

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

## Marker path

`scratch/builtin_m5_marker` — same honesty shape as M2/M4.  
Harness: `harness/docker/run_builtin_m5.sh` (strace asserts no `cc`/`ld`/`as`, run prints `Hello, world!`).

## Requirements

1. Builtin link of ggcc hello (`printf("Hello, world!\n")`) without spawning system `cc`/`as`/`ld`.
2. Resolve libc via static musl archives.
3. Evidence: docker aarch64 run prints `Hello, world!\n`, exit 0.
4. Driver: `GGCC_BUILTIN_AS=1` + `GGCC_BUILTIN_LD=1` + strict mode for marker.

## Implementation notes (2026-07-24)

- `src/linker/aarch64_hosted.rs` — archive-driven musl static link, GOT fill, crt section order, `.text.*` / `.bss.*` canonicalization.
- `src/linker/archive.rs` — Unix `ar` reader.
- `tests/builtin_m5_hello.c` — smoke source.
- Env aliases: `ACC_BUILTIN_*` and `GGCC_BUILTIN_*` both accepted.
- Harness: `harness/docker/run_builtin_m5.sh`.

### Fixes landed

| Fix | Result |
|-----|--------|
| `link_files` chmod 0755 | binary is executable |
| PT_LOAD `(p_vaddr - p_offset) % p_align == 0` | `execve` no longer `EINVAL` |
| `_DYNAMIC` → real DT_NULL `.dynamic` in RW (not `LOAD_BASE`) | Scrt1 ADRP lands off ELF header |
| `e_version = 1` | ELF header Version current |
| CRT | `Scrt1.o` (matches `musl-gcc -static`) |
| **STT_SECTION empty-name resolve via `st_shndx`** | `.bss.main_tls` / `.bss.builtin_tls` no longer alias `.text`/`_start` |
| Canonicalize `.bss.*` → `.bss`; rank `.bss` last | RW `p_memsz > p_filesz` (real BSS); `__init_tls` writes writable TLS |

### Root cause (SEGV_ACCERR)

Musl `__init_tls` relocates against **STT_SECTION** symbols for `.bss.main_tls` / `.bss.builtin_tls`. Those symbols have **empty `st_name`**. The linker treated empty name as “current section”, so ADRP+ADD pointed at `.text` (`0x400130` = `_start`). `__init_tls` then `str` into RX → `SEGV_ACCERR`.

### Verify

```bash
bash harness/docker/run_builtin_m5.sh
test -f scratch/builtin_m5_marker
```
