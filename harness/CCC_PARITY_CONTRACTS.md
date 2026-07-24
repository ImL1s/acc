# CCC Parity Contracts Specification

## 1. Clean-Room & Integrity Mandate

1. **Clean-Room Isolation**: Developers and agents MUST NOT read, clone, copy, vendor, or reference source code from the Anthropic CCC repository (`src/` or any module).
2. **No Compiler Bypass**: The default compilation pipeline must process C source code through `ggcc`/`acc` internal lexer, parser, type checker, IR/AST, and code generator.
   - **Forbidden**: Invoking external C compilers (`gcc`, `clang`, `ccc`, `tcc`) to compile user `.c` code.
   - **Allowed**: Invoking system `as` / `ld` (or `cc` as driver) solely to assemble and link assembly (`.s`) produced by `ggcc`/`acc`.
3. **No Hardcoding / Mocking**:
   - Hardcoding expected test outputs or return values based on file paths or known inputs is strictly prohibited.
   - Prebuilt binary fixtures in test oracles are strictly prohibited.
   - Soft-bar substitutions (e.g., using `sqlite_reg` instead of `testfixture veryquick.test`, or SDS instead of full Redis RESP) are forbidden.

---

## 2. Parity Gate Contracts & Failure Criteria

### Gate 1: Cargo Unit & Integration Tests (`cargo_test`)
- **Command**: `cargo test --release`
- **Pass Criterion**: Exit code `0`; 100% of unit tests pass.
- **Failure Criterion**: Any failing test or non-zero exit code.

### Gate 2: In-Repo Oracles (`inrepo_oracles`)
- **Command**: `zsh harness/run_oracle.sh`
- **Pass Criterion**: Exit code `0`; 100% of fixtures under `oracles/` compile, run, match expected stdout, and match expected return code.
- **Failure Criterion**: Any compiler crash, stdout mismatch, or exit code mismatch.

### Gate 3: c-testsuite Single-Exec (`ctestsuite`)
- **Command**: `zsh harness/run_ctestsuite.sh`
- **Pass Criterion**: Exit code `0`; Stage A range (00001–00100) continuous pass rate ≥ 95% (≥ 95/100 passes).
- **Failure Criterion**: Pass rate below 95% for 00001–00100 or non-zero script exit code.

### Gate 4: Multiarch 4-ISA Suite (`multiarch_4isa`)
- **Command**: `bash harness/run_multiarch_4isa.sh`
- **Pass Criterion**: Continuous IDs 00001–00100 achieve ≥ 95% pass rate across all 4 target ISAs (`x86-64`, `i686`, `aarch64`, `riscv64`).
- **Failure Criterion**: Any ISA achieving < 95% pass rate.

### Gate 5: Anti-Bypass & Provenance Audit (`anti_bypass_audit`)
- **Command**: `zsh scripts/anti_bypass_audit.sh`
- **Pass Criterion**: Exit code `0`; 0 occurrences of CCC provenance strings, 0 external C compiler invocations in `src/`, verified call to `parser::parse` and `emit_assembly`.
- **Failure Criterion**: Any provenance violation or missing pipeline stage.

### Gate 6: Mutation Verification (`mutation_check`)
- **Command**: `zsh harness/mutation_check.sh`
- **Pass Criterion**: Binary generated from code containing a dynamic timestamp string outputs exact matching string.
- **Failure Criterion**: Mismatched stdout or compilation error.

### Gate 7: Linux 6.9 Kernel QEMU Boot (`kernel_boot`)
- **Command**: `bash scripts/kernel_boot.sh`
- **Pass Criterion**: QEMU serial output displays BusyBox `/bin/sh` prompt on both `arm64` and `x86_64`.
- **Failure Criterion**: Boot failure, kernel panic, or soft `acc-init:` prompt without BusyBox shell.

### Gate 8: SQLite `veryquick.test` (`sqlite_test`)
- **Command**: `harness/real_projects/sqlite_test.sh`
- **Pass Criterion**: Official `testfixture` binary compiled by `acc` completes `veryquick.test` with exactly 0 errors across 317,930 assertions.
- **Failure Criterion**: Compilation failure or errors > 0. (`sqlite_reg` is invalid).

### Gate 9: Redis 7.2.5 Live Network (`redis_test`)
- **Command**: `harness/real_projects/redis_test.sh`
- **Pass Criterion**: `redis-server` compiled by `acc` starts, listens on TCP port, and successfully responds to RESP `PING`, `SET`, `GET`.
- **Failure Criterion**: Server crash, build failure, or protocol error. (SDS-only test is invalid).

### Gate 10: PostgreSQL 237 Check (`postgres_test`)
- **Command**: `harness/real_projects/postgres_test.sh`
- **Pass Criterion**: `initdb` executes without SEGV/crash and 237 regression tests pass.
- **Failure Criterion**: `initdb` crash (e.g. exit code 139) or test failures.

### Gate 11: FFmpeg 7331 FATE `checkasm` (`ffmpeg_test`)
- **Command**: `harness/real_projects/ffmpeg_test.sh`
- **Pass Criterion**: `checkasm` executable compiled by `acc` passes all 7331 assembly assertions.
- **Failure Criterion**: Test failures or crash.

### Gate 12: Built-in Assembler & Linker (`builtin_check`)
- **Command**: `harness/builtin_check.sh`
- **Pass Criterion**: M1–M5 freestanding ELF object & executable emission without invoking host `cc`/`ld`.
- **Failure Criterion**: Fallback or invocation of system `cc`/`ld`.

### Gate 13: DWARF Debug Info (`dwarf_check`)
- **Command**: `harness/dwarf_check.sh`
- **Pass Criterion**: Generated ELF contains valid DWARF v4/v5 sections readable by `gdb` and `lldb`.
- **Failure Criterion**: Missing or corrupted DWARF sections.

---

## 3. Evaluation Rules & Evidence Standard

1. **Strict Gate Execution**: All active gates must be executed sequentially or concurrently under `./harness/run_ccc_parity.sh`.
2. **Evidence Path Rules**:
   - Structured JSON output at `evidence/<sha>/summary.json`
   - Markdown summary table at `evidence/<sha>/summary.md`
   - Full logs per gate at `evidence/<sha>/<gate>/<gate>.log`
3. **Evidence Integrity**: SHA256 hashes must be calculated for each execution log file and recorded in `summary.json`.
4. **Failure Propagation**: If any required gate returns status `FAIL`, the overall run status is `FAIL`, and `./harness/run_ccc_parity.sh` MUST exit with a non-zero status code.
