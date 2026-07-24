# Builtin linker (M4 core → M5 hosted)

**Status:** M4 core landed — freestanding aarch64 Linux static `ET_EXEC` from builtin-assembler `.o` **without** system `cc`/`ld`.  
**Not claimed:** M5 (hosted link with libc/`printf` without system `cc`).

## Enable

```sh
cargo build --features builtin_linker   # implies builtin_assembler

# Assemble + try builtin link (falls back to system cc on unresolved/unsupported)
GGCC_BUILTIN_AS=1 GGCC_BUILTIN_LD=1 \
  cargo run --features builtin_linker -- -m aarch64 --target-os linux -o out input.c
```

| Knob | Effect |
|------|--------|
| Cargo feature `builtin_linker` | Compiles `src/linker/`; depends on `builtin_assembler` |
| Env `GGCC_BUILTIN_LD=1` | After a successful builtin `.o`, attempt freestanding builtin link; on `Err`, fall back to system `cc` |
| Default | System assemble/link only |

## M4 (current)

- [x] Parse ELF64 LE `ET_REL` (aarch64).
- [x] Merge alloc sections; resolve globals; reject unresolved (honest — no fake libc).
- [x] Apply reloc subset: `CALL26`, `JUMP26`, `ADR_PREL_HI21`, `ADD_ABS_LO12_NC`, `ABS64/32`.
- [x] Inject `_start` → `bl main`; `movz x8,#93`; `svc #0`.
- [x] Emit static `ET_EXEC` + single `PT_LOAD`.
- [x] Docker smoke: run without installing/using `gcc`/`ld` in the container.

Evidence: `scratch/builtin_m4_marker` (+ `scratch/builtin_m4_prog`, `scratch/builtin_m4_run.log`).

## M5 (requirements — do not stamp `builtin_m5_marker` until green)

Target: link a **hosted** ggcc hello (`printf`) to a runnable aarch64 Linux binary **without spawning system `cc`/`ld`**.

Minimum bar:

1. Resolve libc symbols (`printf`, …) — static musl **or** dynamic (`PT_INTERP` + `.dynsym`/GOT/PLT).
2. Provide or link crt (`_start`/ABI that matches glibc or musl).
3. Driver path: `GGCC_BUILTIN_AS=1` + `GGCC_BUILTIN_LD=1` produces hello stdout `Hello, world!\n` with **zero** system linker process.
4. Marker file: `scratch/builtin_m5_marker` with docker evidence analogous to M2/M4.

Until then: M4 freestanding-only; hosted programs still use system `cc` on `.o`/`.s`.

### M5 progress (static musl — in tree, not stamped)

- [x] Failing smoke harness: `harness/docker/run_builtin_m5.sh` + `tests/builtin_m5_hello.c`
- [x] `src/linker/aarch64_hosted.rs` — Scrt1/crti/crtn + archive `libgcc.a`/`libc.a`, GOT, hosted relocs
- [x] `src/linker/archive.rs` — `ar` member reader
- [x] Driver strict path: `GGCC_BUILTIN_LD_STRICT=1` / `ACC_BUILTIN_LD_STRICT=1`
- [x] chmod 0755, PT_LOAD congruence, real `_DYNAMIC`, `e_version=1` → `execve` succeeds
- [ ] Runtime Hello (still `SEGV_ACCERR@0x400148` in `_start`)
- [ ] Green docker run → `scratch/builtin_m5_marker`

See `docs/notes/builtin_m5_requirements.md` for env knobs and open issues.

## Files

| Path | Role |
|------|------|
| `src/linker/mod.rs` | Public API + tests |
| `src/linker/elf_read.rs` | `ET_REL` reader |
| `src/linker/aarch64.rs` | M4 freestanding layout, relocs, `ET_EXEC` emit |
| `src/linker/aarch64_hosted.rs` | M5 static musl hosted link |
| `src/linker/archive.rs` | Unix `ar` reader for `.a` members |
| `docs/notes/builtin_m5_requirements.md` | M5 approach + status |
| `docs/notes/builtin_linker_m4.md` | This note |
| `scratch/builtin_m4_marker` | M4 evidence stamp |
| `Cargo.toml` | `features.builtin_linker` |
