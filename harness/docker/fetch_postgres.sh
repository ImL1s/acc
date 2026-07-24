#!/usr/bin/env bash
# Fetch and unpack PostgreSQL sources for Stage C2 / Status extras (237 regression bar).
#
# CCC public docs (third_party/ccc-harness-ref, no src/) claim:
#   postgres: PASS (x86 backend: all 237 regression tests pass)
#   — ideas/new_projects.txt, README.md
# No explicit tarball pin in ccc-harness-ref. Regression .out file count in
# upstream postgresql-15.{3..7} tarballs is 237 (15.7 = 238); we pin 15.7 as
# the stable release closest to the CCC bar and document the count probe.
#
# Usage (from repo root):
#   bash harness/docker/fetch_postgres.sh
#   POSTGRES_VER=15.7 bash harness/docker/fetch_postgres.sh
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
POSTGRES_VER="${POSTGRES_VER:-15.7}"
POSTGRES_CACHE="${POSTGRES_CACHE:-$ROOT/third_party/stage_c/postgres}"
DEST="${POSTGRES_DEST:-$POSTGRES_CACHE/postgresql-$POSTGRES_VER}"
TAR="$POSTGRES_CACHE/postgresql-${POSTGRES_VER}.tar.bz2"
BASE_URL="https://ftp.postgresql.org/pub/source/v${POSTGRES_VER}"

log() { echo "[fetch_postgres] $*"; }

mkdir -p "$POSTGRES_CACHE"

if [[ -f "$DEST/configure" && -d "$DEST/src/backend" ]]; then
  log "already present: $DEST"
  exit 0
fi

if [[ ! -f "$TAR" ]]; then
  log "downloading postgresql-${POSTGRES_VER}.tar.bz2 ..."
  curl -fL --retry 3 -o "$TAR" "${BASE_URL}/postgresql-${POSTGRES_VER}.tar.bz2"
else
  log "using cached tarball $TAR"
fi

log "extracting → $POSTGRES_CACHE"
tar -xjf "$TAR" -C "$POSTGRES_CACHE"

if [[ ! -f "$DEST/configure" ]]; then
  log "error: expected tree missing after extract: $DEST" >&2
  exit 1
fi

log "ok: $DEST"
log "regression bar: make check in src/test/regress (237 tests per CCC / PG 15.3–15.7 schedule)"
log "next: export SCRATCH=\$PWD/scratch && bash harness/docker/run_postgres_placeholder.sh"
