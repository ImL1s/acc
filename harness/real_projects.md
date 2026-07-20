# Fixed real-project list

## Stage B (exactly 3)
| # | Name | Location | Build | Test |
|---|------|----------|-------|------|
| 1 | tinyc | `third_party/real/tinyc/` | see `build.sh` | `./build.sh test` |
| 2 | miniz_smoke | `third_party/real/miniz_smoke/` | see `build.sh` | `./build.sh test` |
| 3 | lua_smoke | `third_party/real/lua_smoke/` | see `build.sh` | `./build.sh test` |

Each `build.sh` **must** set `CC` to shipped ggcc only (never system gcc for `.c` compilation of the project sources).

## Stage C2 (≥2 large)
| # | Name | Notes |
|---|------|--------|
| 1 | sqlite | amalgamation + `testfixture` or official suite in Docker Linux |
| 2 | redis | basic `make test` subset in Docker Linux |

## Stage C1
Linux **6.9** kernel build + QEMU boot via `harness/docker/`.
