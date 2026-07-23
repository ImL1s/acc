# ggcc Design Document

Architecture of **ggcc** (clean-room C compiler). For build/usage/status see
[README.md](README.md) and [harness/progress.md](harness/progress.md).

This document describes **this** tree only. It is not derived from CCC `src/`.

---

## Table of Contents

1. [Goals and non-goals](#goals-and-non-goals)
2. [High-level pipeline](#high-level-pipeline)
3. [Source tree](#source-tree)
4. [Frontend](#frontend)
5. [Code generation](#code-generation)
6. [Driver and toolchain boundary](#driver-and-toolchain-boundary)
7. [Harness (human-side method)](#harness-human-side-method)
8. [Stage gates](#stage-gates)
9. [Known residual design debt](#known-residual-design-debt)

---

## Goals and non-goals

**Goals**

- Real (subset) C: multi-function, locals, pointers/arrays, struct/union,
  control flow, goto, typedef, globals, sizeof, basic `#define` / `#include`
- Public-oracle driven growth (c-testsuite + fixed real projects)
- Dual ISA: AArch64 + x86_64 backends sharing one frontend
- Clean-room: no CCC / Anthropic compiler sources as reference implementation
- Stage C experiments: Linux 6.9 arm64 via Docker; large projects (SQLite/Redis)

**Non-goals (today)**

- Full ISO C / every GNU extension
- In-tree assembler, linker, or DWARF writer (system tools assemble/link `.s`)
- Bit-for-bit CCC parity or marketing LOC claims
- Treating Stage A/B alone as “complete”

---

## High-level pipeline

```
    C source (.c)
         |
         v
    +-----------+
    | preprocess|   macros, #if, local #include, soft libc/kernel prefixes
    +-----------+
         |
         v
    +-----------+
    |   lexer   |   tokens (incl. sticky GNU attrs, sections)
    +-----------+
         |
         v
    +-----------+
    |  parser   |   recursive descent → AST (soft recovery on huge TUs)
    +-----------+
         |
         v
    +------------------+
    | codegen_aarch64  |   or codegen_x86_64  → textual assembly
    +------------------+
         |
         v
    system as / cc / ld   (assemble + link ONLY emitted .s)
         |
         v
    executable (Mach-O on Darwin, ELF on Linux)
```

Optional freestanding helpers for early kernel bring-up live in codegen and
are **gated** (`GGCC_SOFT_FREESTANDING`); PASS claims require real C bodies for
mid-boot functions.

---

## Source tree

```
src/
  main.rs           CLI (-S -E -m --target-os -I -D …)
  driver.rs         read → PP → parse → emit → system assemble/link
  preprocess.rs     minimal C preprocessor + soft headers
  lexer.rs / token.rs
  parser.rs / ast.rs
  codegen.rs        AArch64 (Darwin + Linux dialects)
  codegen_x86_64.rs x86_64 System V / Darwin

harness/
  run_oracle.sh / run_ctestsuite.sh / run_multiarch.sh
  mutation_check.sh
  docker/           Linux kernel CC wrapper + build scripts
  progress.md       honest gate status
  STAGE_CONTRACTS.md / real_projects.md

oracles/            small fixtures
third_party/        c-testsuite + real project wrappers + stage_c sources
```

---

## Frontend

### Preprocessor

- Object- and function-like macros, `__VA_ARGS__`, basic `#if` / `#ifdef`
- Local `#include "..."` / extra `-I` paths
- Soft prefixes for freestanding / incomplete system headers (Linux types,
  `va_list`, etc.) — not a substitute for real libc headers on host smoke tests

### Parser

- Recursive descent with struct/union layouts, enums, typedefs
- Soft top-level recovery so one bad decl does not abort a huge kernel TU
- Important fixed patterns: bare cast `(unsigned)`, `T *(name)(params)`, etc.

### AST

- Functions, globals, statements, expressions sufficient for Stage B language
  and large-project smoke/regression work

---

## Code generation

### AArch64

- Darwin: `@PAGE` / `@PAGEOFF`, Mach-O sections
- Linux: `:lo12:`, ELF-oriented sections; optional freestanding early helpers
- AAPCS64-ish register use; large aggregates by-ref + memcpy; va_arg VR cursor

### x86_64

- System V / Darwin dialect for multiarch oracle subset

### Reachability

- Static functions emitted when reachable from roots (non-static / main /
  global initializers) to avoid kernel-header noise

---

## Driver and toolchain boundary

**Allowed**

- Emit `.s` from ggcc
- Invoke system `cc`/`as`/`ld` on that `.s` (and objects derived from it)

**Forbidden on PASS path**

- Passing user/kernel `.c` to gcc/clang/ccc/tcc as the C compiler
- Hardcoding fixture basenames or prebuilt binaries as “compile results”
- Soft SYSCC fallback (`GGCC_ALLOW_SOFT_SYSCC=1` rejected by wrapper)

Kernel builds use `harness/docker/ggcc_cc_wrapper.sh` as `CC=`.

---

## Harness (human-side method)

Aligned with CCC-style *process*, not CCC code:

| Piece | Role |
|-------|------|
| Oracle runner | compile → run → diff expected |
| Public suite | vendored `c-testsuite` single-exec |
| Real projects | fixed list under `third_party/real/` |
| Mutation check | greeting string change must change stdout |
| Anti-bypass | no external C compiler on user `.c`; freestanding gate probe |
| Task locks | `harness/current_tasks/` |
| Progress | `harness/progress.md` (no silent downgrade) |

---

## Stage gates

See `harness/STAGE_CONTRACTS.md`.

| Stage | Meaning |
|-------|---------|
| A | hello + mutation + anti-bypass; c-testsuite 00001–00100 ≥ 95% |
| B | language surface; full single-exec ≥ 90%; 3 real projects |
| C | C1 kernel boot, C2 ≥2 large projects, C3 dual ISA, C4 clean-room, C5 double-run |

**Complete** only when A+B+C all green with SCRATCH evidence.

---

## Known residual design debt

- Early kernel path still has some always-on freestanding helpers (e.g. early
  printk / idmap); mid-boot soft body-skip is opt-in only
- Parser soft recovery can drop bodies under complex headers — track with
  failing public tests
- No full SSA IR / optimizer pipeline (direct AST → asm)
- SQLite full suite not 100% green (residual suite errors)
- Host Darwin vs Linux ELF: dual target-os modes

Update this file when the pipeline shape changes; keep progress.md for gate
truth.
