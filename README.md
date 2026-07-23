# ggcc — Grok's C Compiler (clean-room)

A C compiler written from scratch in Rust. Frontend (preprocess / lex / parse)
and code generation for **AArch64** and **x86_64** are implemented in-tree.
System `as` / `ld` / `cc` are used **only** to assemble and link assembly that
**this** compiler emitted — never to compile user `.c`.

> **Clean-room:** this repository does **not** contain, vendor, or derive from
> [anthropics/claudes-c-compiler](https://github.com/anthropics/claudes-c-compiler)
> `src/`. Process and acceptance (public oracles, real projects, Linux boot
> experiments) are inspired by the CCC experiment's *human-side method*, not
> by reading CCC's compiler implementation.

Status is tracked honestly in [`harness/progress.md`](harness/progress.md).
Stage A/B are largely green; Stage C (kernel + large projects) is **in
progress** — do not treat this as a production compiler.

## Prerequisites

- **Rust** (stable, 2021 edition) — [rustup](https://rustup.rs/)
- Host **macOS (arm64)** or **Linux** with a working C toolchain used only as
  assembler/linker (`cc` / `clang` / `gcc` for `.s` → binary)
- Optional Stage C: **Docker** (Linux aarch64 image) + `qemu-system-aarch64`

## Building

```bash
cargo build --release
```

Binary: `target/release/ggcc`.

## Quick start

```bash
cat > hello.c << 'EOF'
#include <stdio.h>
int main(void) {
    printf("Hello from ggcc!\n");
    return 0;
}
EOF

./target/release/ggcc -o hello hello.c
./hello
```

### Flags

```bash
ggcc -o out input.c              # compile + assemble/link via system cc
ggcc -S -o out.s input.c         # emit assembly only
ggcc -E input.c                  # preprocess only
ggcc -m aarch64 | -m x86_64      # ISA (default aarch64 on Apple Silicon)
ggcc --target-os darwin | linux  # asm dialect (default: host)
ggcc -I dir -DNAME[=val]         # includes / defines
```

Unknown GCC-style flags are ignored so `CC=ggcc` can drive many Makefiles.

## Oracles and harness (CCC-style human side)

```bash
# In-repo fixtures
./harness/run_oracle.sh

# Vendored public c-testsuite (single-exec)
./harness/run_ctestsuite.sh

# Dual-ISA subset (Stage C3)
./harness/run_multiarch.sh

# Mutation + anti-bypass (must stay green)
./harness/mutation_check.sh
./scripts/anti_bypass_audit.sh
```

Agent workflow: [`AGENT_PROMPT.md`](AGENT_PROMPT.md), task locks under
`harness/current_tasks/`, contracts in `harness/STAGE_CONTRACTS.md`.

## Real projects (Stage B / C2)

Frozen Stage B list: [`harness/real_projects.md`](harness/real_projects.md).

```bash
# Examples (after cargo build --release)
CC=$PWD/target/release/ggcc third_party/real/miniz/build.sh test
CC=$PWD/target/release/ggcc third_party/real/lua/build.sh test
CC=$PWD/target/release/ggcc third_party/real/sqlite/build.sh test
```

Stage C2 aims at **SQLite full/regression** and **Redis basic** (not smoke alone).

## Linux kernel (Stage C1)

See [`BUILDING_LINUX.txt`](BUILDING_LINUX.txt). Summary:

- Kernel tree is **not** vendored (too large); fetch Linux **6.9** separately
- Use Docker + `harness/docker/ggcc_cc_wrapper.sh` so kernel `.c` never goes to system CC
- Soft system-CC and mid-boot soft freestanding body-skip are **off** on PASS path
- Current honest status: partial boot evidence; see `harness/progress.md`

## Design

Architecture notes: [`DESIGN_DOC.md`](DESIGN_DOC.md).

Pipeline (honest):

```
  .c  →  preprocess  →  lex  →  parse (AST)
      →  codegen (aarch64 | x86_64 asm)
      →  system as/cc (assemble + link only)
      →  executable
```

Unlike CCC, ggcc does **not** ship a full in-tree assembler/linker/ELF writer;
it emits textual assembly and relies on the host toolchain for the last step.

## Layout

```
src/                 compiler (clean-room)
oracles/             in-repo fixtures (stdout / exit)
third_party/
  c-testsuite/       public single-exec suite
  real/              Stage B project wrappers
  stage_c/           large-project sources (sqlite amalgamation, …)
harness/             oracle runners, docker kernel scripts, progress
scripts/             anti-bypass audit, helpers
docs/                plans / notes
tests/               small C regression snippets
```

## License

MIT — see [`LICENSE`](LICENSE).

## Disclaimer

This is an experimental compiler and harness. It has not been validated for
production use. Claims about Stage C completeness must match
`harness/progress.md` and SCRATCH evidence, not marketing text.
