#!/usr/bin/env zsh
# Ralph-style loop: claim work from progress, run oracles, never compile user C with gcc.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PROMPT="${1:-$ROOT/AGENT_PROMPT.md}"
while true; do
  COMMIT=$(git rev-parse --short=6 HEAD 2>/dev/null || echo nogit)
  LOG="harness/agent_logs/agent_${COMMIT}_$(date +%s).log"
  mkdir -p harness/agent_logs
  echo "=== agent iteration $(date -u +%Y-%m-%dT%H:%M:%SZ) ===" | tee -a "$LOG"
  ./harness/run_oracle.sh 2>&1 | tee -a "$LOG" || true
  CTEST_START=1 CTEST_END=100 ./harness/run_ctestsuite.sh 2>&1 | tee -a "$LOG" || true
  echo "Update harness/progress.md with failures; claim a lock; fix; unlock." | tee -a "$LOG"
  # Single-shot by default unless GGCC_AGENT_FOREVER=1
  if [[ "${GGCC_AGENT_FOREVER:-0}" != "1" ]]; then
    break
  fi
  sleep 2
done
