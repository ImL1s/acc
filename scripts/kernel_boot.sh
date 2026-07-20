#!/usr/bin/env bash
# Thin entry for Stage C1 — delegates to harness/docker/build_kernel.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export SCRATCH="${SCRATCH:?SCRATCH required}"
exec bash "$ROOT/harness/docker/build_kernel.sh"
