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
| 1 | **SQLite full testfixture** | `third_party/stage_c/sqlite_full/sqlite-src-3450300` + ggcc-built `testfixture` | **STRONG** — regression subset 20/22 suites, 17914 tests, 10 errors (~99.94%); prior batch2 ~60k green |
| 2 | **Redis 7.2.5 basic** | redis-server RESP PING/SET/GET under ggcc | **PASS** `PASS_REDIS_DEFAULT_LATENCY` |

Amalgamation `sqlite ok sum=42` / sds smoke remain Stage B / intermediate only — **not** C2 PASS evidence.
