#!/usr/bin/env zsh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
name="${1:-}"
if [[ -z "$name" ]]; then
  echo "usage: $0 <task_name>" >&2
  exit 2
fi
lock="$ROOT/harness/current_tasks/${name}.txt"
rm -f "$lock"
echo "released $lock"
