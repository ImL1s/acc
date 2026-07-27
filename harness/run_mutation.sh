#!/usr/bin/env zsh
# Wrapper for mutation_check.sh to ensure run_mutation.sh command line parity
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec zsh "$ROOT/harness/mutation_check.sh" "$@"
