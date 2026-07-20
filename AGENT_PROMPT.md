# GGCC agent prompt

You are working on **ggcc**: a from-scratch C compiler in Rust.

## Hard rules

1. **Clean-room**: Do **not** read, clone, copy, or vendor `anthropics/claudes-c-compiler` or any of its `src/`. Do not use it as a reference implementation.
2. **No bypass**: The default path must compile user `.c` with *this* compiler only.
   - Forbidden: shelling out to `gcc`/`clang`/`ccc` to compile user C; embedding full expected binaries for named fixtures; `if path == "hello.c"` special cases.
   - Allowed: system `as` / `cc` (or `ld`) **only** to assemble/link *assembly this compiler emitted*.
3. **Oracle-driven**: Success is only `harness/run_oracle.sh` (or equivalent) compile → run → stdout/exit match. Prose claims do not count.
4. **Task locks**: Before editing a module, claim a lock file under `harness/current_tasks/<task>.txt`. Remove the lock when done and push/commit if multi-agent.

## Goal (current milestone)

Compile a standard C hello world that calls `printf` from `main`, produce a host-runnable binary, print the greeting, exit 0. Also pass the other in-repo oracles under `oracles/`.

## How to work

1. Read `harness/progress.md` and failing oracles: `harness/run_oracle.sh`.
2. Claim a lock, fix the smallest failing piece (lexer / parser / codegen / driver).
3. Re-run oracles; update `harness/progress.md` with what works and what is next.
4. Prefer real parse → lower → emit assembly; keep the C subset minimal but real.

## Layout

- `src/` — compiler
- `oracles/` — public in-repo fixtures (`main.c` + expected.stdout + expected.ret)
- `harness/` — oracle runner, locks, progress
