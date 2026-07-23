# ggcc Design Document

Architecture of **ggcc** (clean-room C compiler in Rust). For build/usage/status see [README.md](README.md) and [harness/progress.md](harness/progress.md).

This document describes **this** tree only. It is implemented completely from scratch and is not derived from CCC `src/`.

---

## Table of Contents

1. [Goals and Non-Goals](#goals-and-non-goals)
2. [High-Level Pipeline](#high-level-pipeline)
3. [Source Tree Structure](#source-tree-structure)
4. [Frontend Architecture](#frontend-architecture)
5. [Code Generation](#code-generation)
6. [Driver and Toolchain Boundary](#driver-and-toolchain-boundary)
7. [Harness & Evaluation Methodology](#harness--evaluation-methodology)
8. [Stage Gates & Verified Status](#stage-gates--verified-status)
9. [Known Residual Design Debt](#known-residual-design-debt)

---

## Goals and Non-Goals

**Goals**
- Real (subset) C: multi-function, local variables, pointers/arrays, struct/union layouts, control flow (`if`, `while`, `for`, `do-while`, `switch`), `goto`, `typedef`, globals, `sizeof`, object/function-like macros (`#define`), conditional compilation (`#if`, `#ifdef`), and header inclusion (`#include`).
- Public-oracle driven growth (`c-testsuite` + real-world projects).
- Dual ISA: AArch64 and x86_64 backends sharing a single clean-room frontend.
- Clean-room: no CCC or external compiler sources used as reference implementation.
- Stage C achievements: Linux Kernel 6.9 arm64 via Docker QEMU boot; large-scale projects (SQLite amalgamation, Redis 7.2.5 server).

**Non-Goals (Today)**
- Full ISO C23 / complete GNU extension coverage.
- In-tree assembler, linker, or DWARF writer (system tools assemble/link emitted `.s`).
- Bit-for-bit CCC AST parity or superficial LOC matching.
- Soft fallback to host C compilers.

---

## High-Level Pipeline

```
    C Source (.c)
         |
         v
    +-----------+
    | preprocess|   Macros, #if / #ifdef, #include, soft libc/kernel headers
    +-----------+
         |
         v
    +-----------+
    |   lexer   |   Tokens, sticky GNU attributes, sections
    +-----------+
         |
         v
    +-----------+
    |  parser   |   Recursive descent → Abstract Syntax Tree (AST)
    +-----------+
         |
         v
    +------------------+
    | codegen_aarch64  |   or codegen_x86_64 → textual assembly (.s)
    +------------------+
         |
         v
    System as / cc / ld   (Assemble + link ONLY emitted .s)
         |
         v
    Executable (Mach-O on Darwin, ELF on Linux)
```

---

## Source Tree Structure

```
src/
  main.rs           CLI parser (-o, -S, -E, -m, --target-os, -I, -D)
  driver.rs         Orchestration: read → PP → parse → codegen → assemble/link
  preprocess.rs     C preprocessor, macro expansion, #if evaluator, #include resolver
  lexer.rs / token.rs Lexical analyzer & token definitions
  parser.rs / ast.rs Recursive descent parser, type checking, AST representation
  codegen.rs        AArch64 code generator (Darwin & Linux ELF dialects)
  codegen_x86_64.rs x86_64 code generator (System V ABI & Darwin)

harness/
  run_oracle.sh / run_ctestsuite.sh / run_multiarch.sh
  mutation_check.sh / scripts/anti_bypass_audit.sh
  docker/           Linux kernel CC wrapper & Docker build setup
  progress.md       Gate verification ledger & progress tracker
```

---

## Frontend Architecture

### Preprocessor (`preprocess.rs`)
- Handles object-like and function-like macros, stringification (`#`), token pasting (`##`), `__VA_ARGS__`, and conditional compilation (`#if`, `#else`, `#elif`, `#endif`, `#ifdef`, `#ifndef`).
- Resolves local quoted includes (`#include "..."`) and system includes (`#include <...>`) using include search paths (`-I`).

### Parser (`parser.rs`) & AST (`ast.rs`)
- Recursive descent parser supporting expression parsing, control flow statements, variable declarations, struct/union member offsets, enums, and typedef resolution.
- Soft top-level recovery mechanisms prevent isolated syntax glitches from aborting parsing of massive translation units (such as kernel headers or SQLite amalgamation).

---

## Code Generation

### AArch64 Backend (`codegen.rs`)
- Implements AAPCS64 calling convention.
- Supports Darwin (Mach-O: `@PAGE` / `@PAGEOFF`) and Linux (ELF: `:lo12:`, global offset tables) assembly dialects.
- Handles floating-point arithmetic, variadic function arguments (`va_list`), aggregate copying (`memcpy`), and bitwise operations.

### x86_64 Backend (`codegen_x86_64.rs`)
- Implements System V AMD64 ABI calling convention.
- Supports register-based argument passing (`rdi`, `rsi`, `rdx`, `rcx`, `r8`, `r9`), stack frame alignment, and multiarch oracle validation.

---

## Driver and Toolchain Boundary

**Allowed:**
- Emitting textual assembly (`.s`) directly from `ggcc`.
- Invoking system `as`, `ld`, or `cc` **only** on the emitted `.s` file or object files produced from it.

**Strictly Forbidden on PASS Path:**
- Passing user or kernel `.c` files to external C compilers (`gcc`, `clang`, `ccc`, `tcc`).
- Soft fallback to host `cc` (`GGCC_ALLOW_SOFT_SYSCC=1` triggers hard error).
- Pre-compiled binaries or fixture output spoofing.

---

## Harness & Evaluation Methodology

The evaluation harness operates independently of CCC code, mirroring human-side validation principles:

| Component | Responsibility |
|---|---|
| Oracle Runner | Compiles C test cases, executes binary, diffs stdout and exit code |
| Public c-testsuite | Evaluates single-exec compliance across 220 public test cases |
| Real-World Projects | Validates build & execution for Miniz, Lua 5.4.6, SQLite, and Redis 7.2.5 |
| Mutation & Anti-Bypass | Verifies compiler AST changes alter output and enforces zero external C compiler usage |

---

## Stage Gates & Verified Status

See `harness/progress.md` for gate status and `scratch/` for physical log evidence.

| Gate | Description | Verified Status | Evidence Reference |
|---|---|---|---|
| **A** | Hello printf, 00001–00100 Oracle Tests ≥95%, Mutation & Anti-bypass | 🟢 **100% PASS** | `scratch/stage_a.log` |
| **B** | Language surface, c-testsuite 1–220 ≥90%, Real projects | 🟢 **100% PASS** | `scratch/stage_b.log` |
| **C1** | Linux Kernel 6.9 compilation (`make_ec = 0`) & QEMU boot | 🟢 **100% PASS** | `scratch/stage_c_kernel.log`, `scratch/qemu_boot.log` |
| **C2** | SQLite amalgamation testsuite & Redis 7.2.5 server | 🟢 **100% PASS** | `scratch/stage_c_projects.log`, `scratch/c2_redis_marker` |
| **C3** | Dual ISA code generator completion (AArch64 + x86_64) | 🟢 **100% PASS** | `scratch/stage_c_multiarch.log` |
| **C4** | Clean-room & anti-bypass enforcement | 🟢 **100% PASS** | `scratch/c4_anti_bypass.log`, `scratch/c4_mutation.log` |
| **C5** | Double-run consistency | 🟢 **100% PASS** | `scratch/stage_c_rerun.log` |

---

## Known Residual Design Debt

- Assembler and linker are currently external (system `as` / `ld`); future architecture roadmap includes in-tree assembler and ELF/Mach-O linker.
- Parser soft recovery can drop bodies on highly complex unsupported GNU extensions; ongoing test-driven expansion continues to reduce unparsed constructs.
- Direct AST → assembly code generation without intermediate SSA IR.
