# ggcc — from-scratch C compiler (clean-room)

Minimal-to-serious C compiler written from scratch in Rust.
**Does not use or contain Anthropic CCC / `claudes-c-compiler` sources.**

## Build

```bash
cargo build --release
```

## Compile

```bash
./target/release/ggcc -o hello oracles/hello/main.c
./hello

# ISA selection (default aarch64)
./target/release/ggcc -m aarch64 -o t t.c
./target/release/ggcc -m x86_64  -o t t.c   # macOS: assembles with cc -arch x86_64
```

Pipeline: **lex → parse → (aarch64 | x86_64) asm → system `cc` assemble/link**  
(`cc` never receives your `.c` file.)

## Oracles / harness

```bash
# In-repo fixtures (hello, control flow, multi-fn, pointers, …)
./harness/run_oracle.sh

# Public suite (vendored c-testsuite); require ≥40 passes on slice 1–45
CTEST_START=1 CTEST_END=45 CTEST_MIN_PASS=40 ./harness/run_ctestsuite.sh

# Stage C3 multiarch subset (same IDs on aarch64 + x86_64)
./harness/run_multiarch.sh

./harness/mutation_check.sh
./scripts/anti_bypass_audit.sh
```

Agent workflow: `AGENT_PROMPT.md`, task locks via `harness/claim_task.sh` / `release_task.sh`, notes in `harness/progress.md`.

## Scope vs CCC

This is **CCC-direction** completeness (real language + public single-exec scale), not kernel/PostgreSQL/multi-arch parity. See `harness/progress.md`.

## Layout

- `src/` — compiler
- `oracles/` — in-repo expected stdout/exit fixtures
- `third_party/c-testsuite/` — public oracle suite
- `harness/` — runners + locks
