# CCC Parity Harness Run Summary

- **Compiler Git SHA**: `bd92ef84efde231b5f8b8973022f630619861bec`
- **Started**: `2026-07-24T17:24:40Z`
- **Finished**: `2026-07-24T17:41:03Z`
- **Overall Status**: **FAIL**

## Gate Results Table

| Gate Name | Command | Status | Exit Code | Pass Count | Fail Count | Duration | Log File | Log SHA-256 |
|---|---|---|---|---|---|---|---|---|
| `cargo_test` | `cargo test --release` | **PASS** | 0 | 73 | 0 | 69s | [`evidence/bd92ef84efde231b5f8b8973022f630619861bec/cargo_test/cargo_test.log`](evidence/bd92ef84efde231b5f8b8973022f630619861bec/cargo_test/cargo_test.log) | `3927549d9ded2712...` |
| `inrepo_oracles` | `zsh harness/run_oracle.sh` | **PASS** | 0 | 7 | 0 | 4s | [`evidence/bd92ef84efde231b5f8b8973022f630619861bec/inrepo_oracles/inrepo_oracles.log`](evidence/bd92ef84efde231b5f8b8973022f630619861bec/inrepo_oracles/inrepo_oracles.log) | `dcb95d4b47f81e71...` |
| `ctestsuite` | `zsh harness/run_ctestsuite.sh` | **PASS** | 0 | 216 | 4 | 110s | [`evidence/bd92ef84efde231b5f8b8973022f630619861bec/ctestsuite/ctestsuite.log`](evidence/bd92ef84efde231b5f8b8973022f630619861bec/ctestsuite/ctestsuite.log) | `f0c08851fa095f99...` |
| `multiarch_4isa` | `bash harness/run_multiarch_4isa.sh` | **FAIL** | 1 | 275 | 125 | 796s | [`evidence/bd92ef84efde231b5f8b8973022f630619861bec/multiarch_4isa/multiarch_4isa.log`](evidence/bd92ef84efde231b5f8b8973022f630619861bec/multiarch_4isa/multiarch_4isa.log) | `ea055061f7c40dc8...` |
| `mutation_check` | `zsh harness/mutation_check.sh` | **PASS** | 0 | 1 | 0 | 1s | [`evidence/bd92ef84efde231b5f8b8973022f630619861bec/mutation_check/mutation_check.log`](evidence/bd92ef84efde231b5f8b8973022f630619861bec/mutation_check/mutation_check.log) | `58b2e210979a18a2...` |
| `anti_bypass_audit` | `zsh scripts/anti_bypass_audit.sh` | **PASS** | 0 | 1 | 0 | 1s | [`evidence/bd92ef84efde231b5f8b8973022f630619861bec/anti_bypass_audit/anti_bypass_audit.log`](evidence/bd92ef84efde231b5f8b8973022f630619861bec/anti_bypass_audit/anti_bypass_audit.log) | `3cb5019704c2fc86...` |
