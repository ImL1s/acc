# Progress (NO-DOWNGRADE)

## Stage A — PASS
## Stage B — PASS (202/220, 3 projects)
## Stage C — PARTIAL — **NOT complete**

| Gate | Status |
|------|--------|
| C5 | PASS |
| C3 | PASS |
| C4 | held |
| C2 | **BLOCKED** — `sqlite3.c` → asm → `.o` → link OK; **libversion smoke PASS** (`3.45.3`/`3045003`); open/exec still SIGSEGV; full suite not green |
| C1 | **BLOCKED** — Docker/kernel scripts ready, no boot |

### C2 evidence (`{SCRATCH}/stage_c_projects.log`)
- Full amalgamation frontend→codegen EXIT 0 (~12MB `.s`)
- Docker assemble+link with system `cc` on `.s` only
- `smoke_ver`: **PASS** after Linux AAPCS64 printf fix
- `smoke_open`: SIGSEGV (runtime correctness remaining)
- VERDICT: still **BLOCKED** for Stage C2 full criteria (need full SQLite tests or Redis + second project)

### Session highlights
- git init + plan + worktree-ready harness
- Bitwise compound assign, storage keywords, hex LL, POSIX/PP stubs
- Call-arg comma fix; large frame offsets; Darwin vs Linux varargs ABI
- Kernel docker wrapper scripts (prep only)

### Next
1. Fix sqlite3_open path (memory/init/globals)
2. Full test or Redis as second large project  
3. C1 kernel first compile failure under wrapper

**Do not claim done until C1+C2 hard gates green.**
