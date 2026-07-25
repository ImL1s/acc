#!/usr/bin/env bash
# Official CCC Parity Harness Runner
# Executes enabled baseline gates, records execution logs, evidence SHA-256 hashes,
# and structured JSON / Markdown summaries under evidence/<ggcc-sha>/.
set -euo pipefail

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CLEAN=0
for arg in "$@"; do
  if [[ "$arg" == "--clean" ]]; then
    CLEAN=1
  fi
done

if [[ "$CLEAN" -eq 1 ]]; then
  echo "== Phase 0 Reset: Cleaning build artifacts and scratch logs =="
  cargo clean || rm -rf target || true
  rm -rf target/oracle_work target/ctest_work scratch/stage_a_4isa_work || true
  rm -f scratch/*.log scratch/*.txt || true
fi

echo "== Building ggcc (release profile) =="
cargo build --release
if [[ -f target/release/acc ]]; then
  cp -f target/release/acc target/release/ggcc
fi

GGCC_SHA="$(git rev-parse HEAD)"
EVIDENCE_DIR="$ROOT/evidence/$GGCC_SHA"
mkdir -p "$EVIDENCE_DIR"

RUN_TIMESTAMP_START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Tool versions
RUSTC_VER="$(rustc --version 2>/dev/null || echo "unknown")"
CARGO_VER="$(cargo --version 2>/dev/null || echo "unknown")"
GIT_VER="$(git --version 2>/dev/null || echo "unknown")"

echo "== GGCC Commit SHA: $GGCC_SHA =="
echo "== Evidence Output: $EVIDENCE_DIR =="

# Gates to run
GATES=(
  "cargo_test|cargo test --release"
  "inrepo_oracles|zsh harness/run_oracle.sh"
  "ctestsuite|zsh harness/run_ctestsuite.sh"
  "multiarch_4isa|bash harness/run_multiarch_4isa.sh"
  "mutation_check|zsh harness/mutation_check.sh"
  "anti_bypass_audit|zsh scripts/anti_bypass_audit.sh"
)

OVERALL_PASS=1
GATE_RESULTS=()

for gate_spec in "${GATES[@]}"; do
  IFS="|" read -r GATE_NAME GATE_CMD <<< "$gate_spec"
  GATE_DIR="$EVIDENCE_DIR/$GATE_NAME"
  mkdir -p "$GATE_DIR"
  LOG_FILE="$GATE_DIR/$GATE_NAME.log"
  
  echo "----------------------------------------------------"
  echo "Running Gate: $GATE_NAME"
  echo "Command: $GATE_CMD"
  echo "Log: $LOG_FILE"
  echo "----------------------------------------------------"

  START_TIME=$(date +%s)
  START_ISO="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

  set +e
  eval "$GATE_CMD" 2>&1 | tee "$LOG_FILE"
  EXIT_CODE=${PIPESTATUS[0]}
  set -e

  FINISH_TIME=$(date +%s)
  FINISH_ISO="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  DURATION=$(( FINISH_TIME - START_TIME ))

  STATUS="PASS"
  if [[ "$EXIT_CODE" -ne 0 ]]; then
    STATUS="FAIL"
    OVERALL_PASS=0
  fi

  # Compute log sha256 hash (flush stdio buffer first to avoid race)
  sync 2>/dev/null || true
  sleep 0.5
  LOG_HASH="$(python3 -c "import hashlib; print(hashlib.sha256(open('$LOG_FILE', 'rb').read()).hexdigest())")"

  # Determine architecture
  ARCH_NAME="$(uname -m)"

  # Parse pass/fail counts from log output safely using wc -l
  PASS_COUNT=0
  FAIL_COUNT=0

  case "$GATE_NAME" in
    cargo_test)
      if grep -q "test result: ok." "$LOG_FILE"; then
        PASS_COUNT=$( (grep -oE "[0-9]+ passed" "$LOG_FILE" || true) | head -1 | awk '{print $1}')
        FAIL_COUNT=$( (grep -oE "[0-9]+ failed" "$LOG_FILE" || true) | head -1 | awk '{print $1}')
        PASS_COUNT=${PASS_COUNT:-0}
        FAIL_COUNT=${FAIL_COUNT:-0}
      else
        FAIL_COUNT=1
      fi
      ;;
    inrepo_oracles)
      PASS_COUNT=$( (grep -E "^PASS " "$LOG_FILE" || true) | wc -l | tr -d ' ')
      FAIL_COUNT=$( (grep -E "^FAIL " "$LOG_FILE" || true) | wc -l | tr -d ' ')
      PASS_COUNT=${PASS_COUNT:-0}
      FAIL_COUNT=${FAIL_COUNT:-0}
      ;;
    ctestsuite)
      PASS_COUNT=$( (grep -E "^PASS [0-9]+" "$LOG_FILE" || true) | wc -l | tr -d ' ')
      FAIL_COUNT=$( (grep -E "^FAIL [0-9]+" "$LOG_FILE" || true) | wc -l | tr -d ' ')
      PASS_COUNT=${PASS_COUNT:-0}
      FAIL_COUNT=${FAIL_COUNT:-0}
      ;;
    multiarch_4isa)
      PASS_COUNT=$( (grep -E "^PASS (aarch64|x86_64|i686|riscv64) " "$LOG_FILE" || true) | wc -l | tr -d ' ')
      FAIL_COUNT=$( (grep -E "^(FAIL|TIMEOUT) (aarch64|x86_64|i686|riscv64) " "$LOG_FILE" || true) | wc -l | tr -d ' ')
      PASS_COUNT=${PASS_COUNT:-0}
      FAIL_COUNT=${FAIL_COUNT:-0}
      ;;
    mutation_check)
      if [[ "$STATUS" == "PASS" ]]; then
        PASS_COUNT=1
      else
        FAIL_COUNT=1
      fi
      ;;
    anti_bypass_audit)
      if [[ "$STATUS" == "PASS" ]]; then
        PASS_COUNT=1
      else
        FAIL_COUNT=1
      fi
      ;;
  esac

  echo "Gate $GATE_NAME finished with exit code $EXIT_CODE ($STATUS) in ${DURATION}s"

  # Store result formatted for json helper
  GATE_RESULTS+=("$GATE_NAME::$GATE_CMD::$START_ISO::$FINISH_ISO::$DURATION::$EXIT_CODE::$STATUS::$PASS_COUNT::$FAIL_COUNT::$ARCH_NAME::evidence/$GGCC_SHA/$GATE_NAME/$GATE_NAME.log::$LOG_HASH")
done

RUN_TIMESTAMP_FINISH="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
OVERALL_STATUS_STR="FAIL"
if [[ "$OVERALL_PASS" -eq 1 ]]; then
  OVERALL_STATUS_STR="PASS"
fi

# Export variables for python generator script
export GGCC_SHA EVIDENCE_DIR RUN_TIMESTAMP_START RUN_TIMESTAMP_FINISH OVERALL_STATUS_STR RUSTC_VER CARGO_VER GIT_VER
RAW_RESULTS_STR="$(printf "%s\n" "${GATE_RESULTS[@]}")"
export RAW_RESULTS_STR

python3 - <<'EOF'
import json
import os

sha = os.environ.get("GGCC_SHA", "")
evidence_dir = os.environ.get("EVIDENCE_DIR", "")
start_time = os.environ.get("RUN_TIMESTAMP_START", "")
finish_time = os.environ.get("RUN_TIMESTAMP_FINISH", "")
overall_status = os.environ.get("OVERALL_STATUS_STR", "FAIL")
rustc_ver = os.environ.get("RUSTC_VER", "")
cargo_ver = os.environ.get("CARGO_VER", "")
git_ver = os.environ.get("GIT_VER", "")

raw_results_str = os.environ.get("RAW_RESULTS_STR", "").strip()
raw_results = raw_results_str.split('\n') if raw_results_str else []

gates_data = []
for line in raw_results:
    if not line:
        continue
    parts = line.split('::')
    if len(parts) >= 12:
        gates_data.append({
            "gate": parts[0],
            "command": parts[1],
            "start_time": parts[2],
            "finish_time": parts[3],
            "duration_seconds": int(parts[4]),
            "exit_code": int(parts[5]),
            "status": parts[6],
            "pass_count": int(parts[7]),
            "fail_count": int(parts[8]),
            "architecture": parts[9],
            "log_file": parts[10],
            "evidence_hash": parts[11]
        })

summary_json = {
    "compiler_sha": sha,
    "timestamp_start": start_time,
    "timestamp_finish": finish_time,
    "overall_status": overall_status,
    "tool_versions": {
        "rustc": rustc_ver,
        "cargo": cargo_ver,
        "git": git_ver
    },
    "gates": gates_data
}

json_path = os.path.join(evidence_dir, "summary.json")
with open(json_path, "w") as f:
    json.dump(summary_json, f, indent=2)

# Generate summary.md
md_lines = []
md_lines.append("# CCC Parity Harness Run Summary\n")
md_lines.append(f"- **Compiler Git SHA**: `{sha}`")
md_lines.append(f"- **Started**: `{start_time}`")
md_lines.append(f"- **Finished**: `{finish_time}`")
md_lines.append(f"- **Overall Status**: **{overall_status}**\n")
md_lines.append("## Gate Results Table\n")
md_lines.append("| Gate Name | Command | Status | Exit Code | Pass Count | Fail Count | Duration | Log File | Log SHA-256 |")
md_lines.append("|---|---|---|---|---|---|---|---|---|")

for g in gates_data:
    hash_short = g['evidence_hash'][:16] + "..." if g['evidence_hash'] else "N/A"
    md_lines.append(f"| `{g['gate']}` | `{g['command']}` | **{g['status']}** | {g['exit_code']} | {g['pass_count']} | {g['fail_count']} | {g['duration_seconds']}s | [`{g['log_file']}`]({g['log_file']}) | `{hash_short}` |")

md_path = os.path.join(evidence_dir, "summary.md")
with open(md_path, "w") as f:
    f.write("\n".join(md_lines) + "\n")

print("\nSummary files generated at:")
print(f"  JSON: {json_path}")
print(f"  MD:   {md_path}")
EOF

echo ""
echo "===================================================="
echo " CCC Parity Run Finished: Overall Status $OVERALL_STATUS_STR"
echo "===================================================="
cat "$EVIDENCE_DIR/summary.md"

if [[ "$OVERALL_PASS" -eq 1 ]]; then
  exit 0
else
  exit 1
fi
