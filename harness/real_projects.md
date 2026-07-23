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

## Stage C2 (≥2 large) — CCC-strict frozen bar

Per `STAGE_CONTRACTS.md` / `docs/plans/2026-07-23-ccc-full-parity.md`:

| # | Project | Required PASS evidence | Soft evidence (NOT C2 PASS) |
|---|---------|------------------------|-----------------------------|
| 1 | **SQLite** | Official **`testfixture`** + **`test/veryquick.test`** under ggcc (suite summary in log; nfail=0 or documented ledger skips only) | `sqlite_reg` 38/38, amalgamation `sqlite ok sum=42` |
| 2 | **Redis** | Built **`redis-server`** + live RESP **`PING`→`PONG`**, **`SET`/`GET`** | SDS harness / `PASS_REDIS_SDS*` |

Amalgamation smoke remains Stage B. **`sqlite_reg` and SDS are explicitly not C2 PASS** after the honesty reset.
