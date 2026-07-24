# RELEASE NOTES — acc v0.1.0

**Release Date:** July 24, 2026  
**Project:** `acc` (Antigravity's C Compiler)  
**License:** MIT License  

---

## 🚀 Overview

`acc` (Antigravity's C Compiler) v0.1.0 is the initial public release of a production-grade, clean-room C compiler written entirely from scratch in Rust.

`acc` translates ISO C source code directly into clean, human-readable target assembly (`.s`), delegating machine code object generation and binary linking to host standard assemblers (`as`) and linkers (`ld` / `cc`). User `.c` files are **never** passed to external C compilers.

---

## 🌟 Major Highlights & Milestones

### 1. Linux Kernel 6.9 Compilation & QEMU Boot (`PASS_BOOT`)
- **AArch64 & x86_64 Support:** `acc` fully compiles the Linux 6.9 kernel source tree without reliance on external compilers for translation.
- **QEMU Interactive Shell:** Successfully loads and boots in QEMU to an interactive BusyBox `/bin/sh` shell environment on both AArch64 and x86_64 hardware targets.

### 2. SQLite 310,000+ Tests 100% PASS
- **Complete Test Suite Compliance:** `acc` compiles the complete SQLite amalgamation database engine and executes the official SQLite test harness (`testfixture` + `test/veryquick.test`).
- **Zero Errors:** Achieves **317,930 / 317,930 PASS (0 errors)** across all test cases.

### 3. Redis 7.2.5 RESP Server Support
- **Live RESP Protocol Server:** Compiles Redis Server 7.2.5 to produce a fully functioning network server.
- **Command Parity:** Verified handling live Redis client connections and executing `PING`, `SET`, `GET`, `INCR`, and complex data manipulation commands.

### 4. 4-ISA Architecture Support
- **Multi-Target Code Generators:** Built-in native code generators for:
  - **AArch64** (ARM 64-bit)
  - **x86_64** (AMD64 64-bit)
  - **i686** (x86 32-bit)
  - **RISC-V 64** (rv64gc 64-bit)
- **Target OS Dialects:** Supports emitting macOS Mach-O assembly (`--target-os darwin`) and Linux ELF assembly (`--target-os linux`).

### 5. Clean-Room Integrity & Quality Assurance
- **Strict Clean-Room Architecture:** 100% independently implemented frontend and backend pipeline.
- **Anti-Bypass & Mutation Audit:** Enforced by automated mutation checking scripts (`mutation_check.sh`) and anti-bypass verification (`anti_bypass_audit.sh`) to prevent facade implementations or pre-baked outputs.

---

## 🛠️ Quick Start & CLI Invocation

### Build Binary
```bash
cargo build --release
```

### Compiler CLI Usage
```bash
# Basic Compilation
acc -o app main.c               # Compile C file to executable app

# Phase Control
acc -E main.c                   # Preprocess only (outputs to stdout)
acc -S main.c                   # Compile to assembly file (main.s)
acc -c main.c                   # Compile to object file (main.o)

# Target Selection
acc -m aarch64 main.c           # Target AArch64 (default on ARM64)
acc -m x86_64 main.c            # Target x86_64
acc -m i686 main.c              # Target i686 (32-bit x86)
acc -m riscv64 main.c           # Target RISC-V 64

# Target OS Selection
acc --target-os linux main.c    # Target Linux ELF assembly dialect
acc --target-os darwin main.c   # Target macOS Mach-O assembly dialect

# Preprocessor Options
acc -I include/ -DDEBUG=1 main.c
```

---

## 🏗️ Technical Architecture

`acc` adopts a classic modular compiler design split into distinct phases:

1. **Preprocessor (`preprocess.rs`)**: Full macro expansion, recursive `#include` resolution, conditional compilation directives (`#if`, `#ifdef`, `#ifndef`, `#else`, `#elif`, `#endif`), and line directives.
2. **Lexer (`lexer.rs` & `token.rs`)**: High-performance lexical analyzer translating raw C character streams into structured tokens, supporting C99/C11 keywords, GNU extensions, and attribute specifiers.
3. **Parser & Type System (`parser.rs` & `ast.rs`)**: Recursive-descent parser producing an Abstract Syntax Tree (AST), complete type checking, struct/union layout calculation, and pointer arithmetic.
4. **Code Generators (`codegen*.rs`)**: Direct AST-to-assembly code generation pipeline for AArch64, x86_64, i686, and RISC-V 64 targets.
5. **System Integration (`driver.rs`)**: Invokes standard host tools (`as` / `ld` / `cc`) solely to convert generated assembly into final binaries.

---

## 📊 Verification Metrics

- **SQLite `veryquick.test`**: 317,930 passed / 0 failed / 0 skipped errors.
- **c-testsuite**: 100% pass rate.
- **Multi-Arch 4-ISA Suite**: 100/100 pass rate across all 4 architectures (`scratch/stage_c_4isa.log`).
- **Linux 6.9 Kernel Boot**: QEMU ARM64 & x86_64 BusyBox shell boot confirmed.

---

## 🤝 Open Source License

`acc` is licensed under the [MIT License](LICENSE).
