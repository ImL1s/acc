# CCC Parity Harness Run Summary

- **Compiler Git SHA**: `1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9`
- **Started**: `2026-07-25T16:28:23Z`
- **Finished**: `2026-07-25T16:37:25Z`
- **Overall Status**: **PASS**

## Gate Results Table

| Gate Name | Command | Status | Exit Code | Pass Count | Fail Count | Duration | Log File | Log SHA-256 |
|---|---|---|---|---|---|---|---|---|
| `cargo_test` | `cargo test --release` | **PASS** | 0 | 72 | 0 | 3s | [`evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/cargo_test/cargo_test.log`](evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/cargo_test/cargo_test.log) | `93804d92974e4c7f...` |
| `inrepo_oracles` | `zsh harness/run_oracle.sh` | **PASS** | 0 | 7 | 0 | 2s | [`evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/inrepo_oracles/inrepo_oracles.log`](evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/inrepo_oracles/inrepo_oracles.log) | `dcb95d4b47f81e71...` |
| `ctestsuite` | `zsh harness/run_ctestsuite.sh` | **PASS** | 0 | 214 | 6 | 84s | [`evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/ctestsuite/ctestsuite.log`](evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/ctestsuite/ctestsuite.log) | `93b077861637c243...` |
| `multiarch_4isa` | `bash harness/run_multiarch_4isa.sh` | **PASS** | 0 | 400 | 0 | 448s | [`evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/multiarch_4isa/multiarch_4isa.log`](evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/multiarch_4isa/multiarch_4isa.log) | `3f348fa484340cdd...` |
| `mutation_check` | `zsh harness/run_mutation.sh` | **PASS** | 0 | 1 | 0 | 1s | [`evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/mutation_check/mutation_check.log`](evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/mutation_check/mutation_check.log) | `62b2bf97ac752bb4...` |
| `anti_bypass_audit` | `zsh scripts/anti_bypass_audit.sh` | **PASS** | 0 | 1 | 0 | 0s | [`evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/anti_bypass_audit/anti_bypass_audit.log`](evidence/1d80a1c0ba8afefe0a962c77f6b254e6ca2869d9/anti_bypass_audit/anti_bypass_audit.log) | `3cb5019704c2fc86...` |
