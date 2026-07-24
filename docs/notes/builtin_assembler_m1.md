# Builtin assembler (M1 scaffold → M2 ELF encode)

**Status:** M2 subset landed — ELF `.o` emission for a ggcc-shaped aarch64/linux slice.  
**M4:** freestanding builtin `ld` core — see `docs/notes/builtin_linker_m4.md` + `scratch/builtin_m4_marker`.  
**Not claimed:** M5 (hosted link without system `cc`).  
**ISA:** aarch64 Linux ELF only. Darwin / x86_64 deferred.

## Enable the feature

```sh
# build with module compiled in (default path still uses system cc on .s)
cargo build --features builtin_assembler

# opt into the builtin path at runtime (falls back to system cc on error)
GGCC_BUILTIN_AS=1 cargo run --features builtin_assembler -- -m aarch64 --target-os linux -o out input.c
```

Without `--features builtin_assembler`, `driver.rs` is unchanged (system `cc` on `.s` only).

| Knob | Effect |
|------|--------|
| Cargo feature `builtin_assembler` | Compiles `src/assembler/`; wires optional driver hook |
| Env `GGCC_BUILTIN_AS=1` | Attempts builtin `.s`→`.o` before system `cc`; on `Err`, falls back |
| Default (no feature / no env) | System assembler/linker only |

## API (`src/assembler/`)

| Item | Role |
|------|------|
| `parse_assembly(src, target, os)` | Parse tiny ggcc `.s` subset → `AsmUnit` |
| `assemble_to_object(...)` | Parse + emit relocatable ELF64 `.o` (M2 subset) |
| `assemble_file(..., obj_path)` | Write `.o` path |
| `env_opt_in()` | `GGCC_BUILTIN_AS` check |
| `aarch64/` | Parse + encode + ELF writer |

`AsmLine`: `Empty` | `Directive` | `Label` | `Instr`.

## M2 (current)

- [x] Parse ggcc aarch64 Linux `.s` for C3 subset (hello-shaped main, prologue/epilogue).
- [x] Emit relocatable ELF `.o` (`ET_REL`, `.text`/`.rodata`, symtab, `.rela.text` for `bl` / `adrp` / `:lo12:`).
- [x] Driver: with feature + `GGCC_BUILTIN_AS=1`, can produce `.o` without system `as`; **link still uses system `cc`**.
- [x] Default build (no feature) identical to pre-scaffold behavior.
- [x] Tests: `cargo test --features builtin_assembler` (incl. docker link/run smoke for main + hello fragment).

### Encoded today (honest subset)

`ret`, `nop`, `mov`, `movz`, `add`, `sub`, `stp`, `ldp`, `str`, `ldr` (incl. pre-index / unscaled), `adrp`, `add … :lo12:`, `bl`, local `b`; common data directives (`.asciz`, `.zero`, …). Many allow-listed mnemonics are **parse-only**.

Evidence: `scratch/builtin_m2_marker`.

## M3+ (not here — do not claim)

- Broad mnemonic coverage (SIMD, all branches, etc.).
- Full ggcc kernel / `.equ` register ladder / all weak stubs in one pass without fallback.

## M4/M5

- **M4 (landed):** Builtin linker module (`src/linker/`) — freestanding aarch64 `ET_EXEC`. Evidence: `scratch/builtin_m4_marker`.
- **M5 (open):** Link hosted executable + libc **without** system `cc` / system `ld`. See `docs/notes/builtin_linker_m4.md`.

## Feature-flag plan

```toml
[features]
default = []
builtin_assembler = []   # empty; gates cfg only
builtin_linker = ["builtin_assembler"]
```

Driver pseudocode (wired behind `cfg`):

```text
write .s
if cfg(builtin_assembler) && GGCC_BUILTIN_AS:
    match assemble_file(.s → .o):
        Ok → link .o with system cc   # M2: still external linker
        Err → fall back to system cc on .s
else:
    system cc on .s
```

## Files

| Path | Role |
|------|------|
| `src/assembler/mod.rs` | Public API + tests |
| `src/assembler/aarch64/` | Parse, encode, ELF emit |
| `docs/notes/builtin_assembler_m1.md` | This note |
| `scratch/builtin_m2_marker` | M2 link/run evidence stamp |
| `scratch/builtin_m4_marker` | M4 freestanding builtin ld evidence |
| `Cargo.toml` | `features.builtin_assembler` / `builtin_linker` |
| `src/main.rs` | `#[cfg(feature)] mod assembler` / `mod linker` |
| `src/driver.rs` | Feature-gated opt-in + system fallback |
| `src/linker/` | M4 freestanding ELF linker |

**Not touched:** freestanding/kernel codegen, wrapper, Stage C harnesses.
