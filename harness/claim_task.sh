#!/usr/bin/env zsh
# Claim a task lock under harness/current_tasks/ (git-friendly single file lock).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
name="${1:-}"
if [[ -z "$name" ]]; then
  echo "usage: $0 <task_name>" >&2
  exit 2
fi
lock="$ROOT/harness/current_tasks/${name}.txt"
if [[ -e "$lock" ]]; then
  echo "ERROR: lock already held: $lock" >&2
  cat "$lock" >&2 || true
  exit 1
fi
{
  echo "agent=${GGCC_AGENT_ID:-local}"
  echo "host=$(hostname)"
  echo "pid=$$"
  echo "time=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$lock"
echo "claimed $lock"
