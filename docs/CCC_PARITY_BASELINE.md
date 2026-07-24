# CCC Parity Baseline Specification

## 1. Pinned Reference Implementation

- **Repository**: `https://github.com/anthropics/claudes-c-compiler.git`
- **Commit SHA**: `6f1b99acb2f4ec2414592136c2009fe7713deec3`
- **Comparison Date**: `2026-02-05T17:07:47Z`

---

## 2. Public Capabilities Enumeration

The target for full parity against the Anthropic Claude's C Compiler (CCC) reference snapshot encompasses 8 core capability domains across 4 target Instruction Set Architectures (ISAs):

1. **4 Target ISAs**: Full assembly code generation and execution target support for `x86-64`, `i686`, `AArch64` (`arm64`), and `RISC-V 64` (`riscv64`).
2. **Linux 6.9 Kernel QEMU Boot**: Compilation and linking of a bootable Linux 6.9 kernel booting under QEMU to an interactive BusyBox userspace shell prompt (`/bin/sh`).
3. **SQLite `testfixture` `veryquick.test`**: Full compilation of the SQLite database engine test fixture running `veryquick.test` with 0 test errors across all ~317,930 test assertions.
4. **Redis 7.2.5 Live Network Test**: Compilation of the `redis-server` binary capable of handling real-time TCP network requests using the RESP protocol (`PING`, `SET`, `GET`).
5. **PostgreSQL 237 Check**: Compilation and execution of PostgreSQL 237 regression tests (`make check` passing post-`initdb`).
6. **FFmpeg 7331 FATE `checkasm`**: Compilation and execution of FFmpeg assembly verification test suite (`checkasm`).
7. **Built-in Assembler and Linker**: Integrated code emission, ELF object file creation, and static ELF linking (Milestones M1–M5 direct executable emission without host `cc`/`ld` dependency).
8. **DWARF Debug Info**: Emission of standard DWARF v4/v5 debugging symbols (`.debug_info`, `.debug_line`, `.debug_abbrev`) compatible with `gdb` and `lldb`.

---

## 3. Executable Contracts Matrix

| Contract ID | Capability Name | Reproduction Command | Expected Pass Condition | Initial Status | Evidence Path | Known Differences / Notes |
|---|---|---|---|---|---|---|
| `GATE-01` | Cargo & Unit Tests | `cargo test --release` | 100% pass (0 failing tests) | `UNVERIFIED` | `evidence/<sha>/cargo_test/cargo_test.log` | Standard Rust unit test suite for lexer, parser, codegen. |
| `GATE-02` | In-Repo Oracles | `zsh harness/run_oracle.sh` | 100% pass across all `oracles/*` fixtures | `UNVERIFIED` | `evidence/<sha>/inrepo_oracles/inrepo_oracles.log` | Verifies stdout and return code against expected outputs. |
| `GATE-03` | c-testsuite (Stage A) | `zsh harness/run_ctestsuite.sh` | Range 00001-00100 continuous pass rate ≥ 95% | `UNVERIFIED` | `evidence/<sha>/ctestsuite/ctestsuite.log` | Vendored public C testsuite single-exec tests. |
| `GATE-04` | Multiarch 4-ISA | `bash harness/run_multiarch_4isa.sh` | 100 tests (00001-00100) pass rate ≥ 95% on x86-64, i686, arm64, riscv64 | `UNVERIFIED` | `evidence/<sha>/multiarch_4isa/multiarch_4isa.log` | Cross-compilation and execution via host or QEMU Docker containers. |
| `GATE-05` | Anti-Bypass Audit | `zsh scripts/anti_bypass_audit.sh` | 0 violations (no CCC provenance string, no external C compiler invocation) | `UNVERIFIED` | `evidence/<sha>/anti_bypass_audit/anti_bypass_audit.log` | Ensures clean-room enforcement and non-bypass compilation path. |
| `GATE-06` | Mutation Proof | `zsh harness/mutation_check.sh` | Binary stdout matches dynamically generated string in source | `UNVERIFIED` | `evidence/<sha>/mutation_check/mutation_check.log` | Prevents hardcoded binary or static output cheating. |
| `GATE-07` | Linux 6.9 Kernel QEMU Boot | `bash scripts/kernel_boot.sh` | Kernel boots to BusyBox `/bin/sh` prompt on arm64 and x86_64 | `UNVERIFIED` | `evidence/<sha>/kernel_boot/kernel_boot.log` | Requires Linux environment / Docker with QEMU. |
| `GATE-08` | SQLite `veryquick.test` | `harness/real_projects/sqlite_test.sh` | 0 errors out of 317,930 tests in `testfixture` | `UNVERIFIED` | `evidence/<sha>/sqlite/sqlite.log` | Strict C2 bar (`sqlite_reg` mock is strictly forbidden). |
| `GATE-09` | Redis 7.2.5 Network Test | `harness/real_projects/redis_test.sh` | `redis-server` boots and handles live RESP PING/SET/GET | `UNVERIFIED` | `evidence/<sha>/redis/redis.log` | Strict C2 bar (SDS mock is strictly forbidden). |
| `GATE-10` | PostgreSQL 237 Check | `harness/real_projects/postgres_test.sh` | `initdb` succeeds and `make check` passes 237 tests | `UNVERIFIED` | `evidence/<sha>/postgres/postgres.log` | Full database regression test run under compiled `acc`. |
| `GATE-11` | FFmpeg 7331 FATE `checkasm` | `harness/real_projects/ffmpeg_test.sh` | FATE `checkasm` passes 7331 assertions | `UNVERIFIED` | `evidence/<sha>/ffmpeg/ffmpeg.log` | Audio/video processing math & assembly validation. |
| `GATE-12` | Built-in Assembler/Linker | `harness/builtin_check.sh` | Freestanding ELF object & binary emission without system `cc`/`ld` (M1-M5) | `UNVERIFIED` | `evidence/<sha>/builtin/builtin.log` | Native ELF generation capability. |
| `GATE-13` | DWARF Debug Info | `harness/dwarf_check.sh` | Emitted ELF binaries contain valid DWARF v4/v5 debug sections | `UNVERIFIED` | `evidence/<sha>/dwarf/dwarf.log` | Inspectable by `gdb` / `lldb` / `dwarfdump`. |

---

*Note: Initial status for all gates is UNVERIFIED until Phase 0 clean run via `./harness/run_ccc_parity.sh --clean` completes.*
