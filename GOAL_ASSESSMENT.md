# ACC Goal Assessment

**Assessment Date**: 2026-07-28  
**Goal**: **NOT COMPLETE**  
**Status**: **IN_PROGRESS**  
**Method**: Grounded codebase audit & empirical verification (`src/codegen_x86_64.rs`, `src/codegen.rs`, `src/parser.rs`, `src/ast.rs`, `Cargo.toml`, `harness/`)

---

## Executive Summary

The `acc` C compiler is an in-tree clean-room C compiler written entirely in Rust. It compiles C source files via an internal preprocessor, lexer, parser, and target assembly generators (`codegen.rs` for AArch64, `codegen_x86_64.rs` for x86_64, `codegen_i686.rs` for i686, `codegen_riscv.rs` for RISC-V 64), invoking system assemblers and linkers (`as`, `ld`, `cc`) to produce native executables.

Recent work fixed x86_64 code generation for unary operations (`UnaryOp::Neg`, `UnaryOp::BitNot`) and shift operations (`BinOp::Shl`, `BinOp::Shr`, `<<=`, `>>=`), ensuring that 32-bit vs 64-bit operand width, signedness, count masking, and zero/sign extension (`movl %eax, %eax` vs `movslq %eax, %rax`) are correctly handled. All 23 compiler warnings across `src/` and `tests/` have been eliminated, and `cargo test --release` as well as `cargo test test_single_exec_00200` pass with zero warnings and 0 failures.

---

## 1. Actual Repository Structure

The actual codebase structure consists of:
- **Executable**: Single `acc` binary output target defined in `Cargo.toml` and `src/main.rs`.
- **Frontend & Preprocessor**:
  - `src/lexer.rs` & `src/token.rs`: Lexical analyzer and token definitions.
  - `src/preprocess.rs`: Preprocessor handling `#include`, macro expansion, conditional compilation (`#ifdef`/`#if`), and pre-included headers.
  - `src/parser.rs` & `src/ast.rs`: Recursive-descent parser producing AST representation (`Expr`, `Stmt`, `Type`).
  - `src/assigned_names.rs`: Name resolution helpers.
- **Code Generators**:
  - `src/codegen.rs`: Core AArch64 code generator and target dispatcher (`Target::Aarch64`, `Target::X86_64`, `Target::I686`, `Target::Riscv64`).
  - `src/codegen_x86_64.rs`: System V x86_64 code generator.
  - `src/codegen_i686.rs`: 32-bit i686 code generator.
  - `src/codegen_riscv.rs`: RISC-V 64 code generator.
- **Optional Built-in Toolchain**:
  - `src/assembler/`: Built-in assembler (optional feature).
  - `src/linker/`: Built-in linker (optional feature).
- **Harness & Verification Scripts**:
  - `harness/`: Integration testing scripts (`run_oracle.sh`, `run_ctestsuite.sh`, `run_multiarch.sh`, `run_ccc_parity.sh`).
  - `tests/`: Integration test modules (`m1_1_frontend_tests.rs`, `single_exec_tests.rs`).

---

## 2. Status Synchronization & Findings

- **Architecture**: Single `acc` binary supporting target selection via `-m aarch64|x86_64|i686|riscv64` and `--target-os darwin|linux`.
- **Test Status**: `cargo build --release` produces ZERO warnings. `cargo test --release` passes with ZERO warnings and 0 failures. `test_single_exec_00200` passes cleanly.
- **Goal Status**: `Goal: NOT COMPLETE` / `Status: IN_PROGRESS` until all downstream parity and release gates are fully met and audited.
