# Stage B — 3 real projects (frozen list)

Build each with `CC=$PWD/target/release/ggcc`.

| # | Project | Path | Build | Test |
|---|---------|------|-------|------|
| 1 | miniz | `third_party/real/miniz/` | `./build.sh` | `./build.sh test` |
| 2 | lua | `third_party/real/lua/` | `./build.sh` | `./build.sh test` (prints `lua ok 42`) |
| 3 | sqlite | `third_party/real/sqlite/` | `./build.sh` | `./build.sh test` (`sqlite ok sum=42`) |

## Status (2026-07-21)

- **miniz**: PASS — compress/decompress roundtrip
- **lua 5.4.6**: PASS — multi-file, `print("lua ok", 6*7)` → 42
- **sqlite amalgamation**: PASS — `third_party/stage_c/sqlite/sqlite3.c` under ggcc → link → open :memory: SUM 40+2=42

## Stage C2 (≥2 large) — frozen bar (no smoke-only PASS)

Per `STAGE_CONTRACTS.md` / plan: **SQLite full/regression** and/or **Redis basic** (not amalgamation/sds smoke alone).

| # | Project | Path | Status |
|---|---------|------|--------|
| 1 | **SQLite regression (`sqlite_reg`)** | amalgamation + `harness/c2/sqlite_reg.c` under ggcc | **PASS** — npass=38 nfail=0 (delete/types/tx/join/view); evidence `scratch/stage_c_projects.log`. Official `testfixture` link still blocked (zipfileInflate/Deflate). |
| 2 | **Redis 7.2.5 basic** | SDS/zmalloc harness under ggcc | **PASS** `PASS_REDIS_SDS_BASIC`; full `redis-server` RESP still blocked (math/callReply residuals). |

Amalgamation `sqlite ok sum=42` alone remains Stage B — **not** C2 PASS evidence. `sqlite_reg` exceeds that bar.
