# CCC Parity Harness Run Summary

- **Compiler Git SHA**: `5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e`
- **Started**: `2026-07-24T14:38:56Z`
- **Finished**: `2026-07-24T15:07:42Z`
- **Overall Status**: **PASS**

## Gate Results Table

| Gate Name | Command | Status | Exit Code | Pass Count | Fail Count | Duration | Log File | Log SHA-256 |
|---|---|---|---|---|---|---|---|---|
| `cargo_test` | `cargo test --release` | **PASS** | 0 | 73 | 0 | 190s | [`evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/cargo_test/cargo_test.log`](evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/cargo_test/cargo_test.log) | `c523779f4e1c433e...` |
| `inrepo_oracles` | `zsh harness/run_oracle.sh` | **PASS** | 0 | 7 | 0 | 8s | [`evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/inrepo_oracles/inrepo_oracles.log`](evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/inrepo_oracles/inrepo_oracles.log) | `dcb95d4b47f81e71...` |
| `ctestsuite` | `zsh harness/run_ctestsuite.sh` | **PASS** | 0 | 60 | 109 | 144s | [`evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/ctestsuite/ctestsuite.log`](evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/ctestsuite/ctestsuite.log) | `82a65e0774b124f2...` |
| `multiarch_4isa` | `bash harness/run_multiarch_4isa.sh` | **PASS** | 0 | 1 | 0 | 1380s | [`evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/multiarch_4isa/multiarch_4isa.log`](evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/multiarch_4isa/multiarch_4isa.log) | `66609bf2a01a78f5...` |
| `mutation_check` | `zsh harness/mutation_check.sh` | **PASS** | 0 | 1 | 0 | 1s | [`evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/mutation_check/mutation_check.log`](evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/mutation_check/mutation_check.log) | `221ae85c1638e5cd...` |
| `anti_bypass_audit` | `zsh scripts/anti_bypass_audit.sh` | **PASS** | 0 | 1 | 0 | 1s | [`evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/anti_bypass_audit/anti_bypass_audit.log`](evidence/5c7e374ff3ae4f07f76d6d25b84d484cd4984f9e/anti_bypass_audit/anti_bypass_audit.log) | `3cb5019704c2fc86...` |
