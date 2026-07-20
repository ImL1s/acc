#!/usr/bin/env zsh
# Run vendored public c-testsuite single-exec cases with the project compiler.
# Success = compile + run + match expected (stdout+exit). No skip-as-pass.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BIN="${GGCC_BIN:-$ROOT/target/release/ggcc}"
SUITE="${CTESTSUITE_DIR:-$ROOT/third_party/c-testsuite/tests/single-exec}"
WORKDIR="${GGCC_ORACLE_WORK:-$ROOT/target/ctest_work}"
# Optional: limit range, e.g. START=1 END=50
START="${CTEST_START:-1}"
END="${CTEST_END:-220}"
# Stop after N passes if set (0 = no limit)
MIN_PASS="${CTEST_MIN_PASS:-0}"

if [[ ! -x "$BIN" ]]; then
  cargo build --release
  BIN="$ROOT/target/release/ggcc"
fi

if [[ ! -d "$SUITE" ]]; then
  echo "ERROR: c-testsuite not found at $SUITE" >&2
  exit 2
fi

mkdir -p "$WORKDIR"
passed=0
failed=0
skipped=0
pass_ids=()

for i in $(seq "$START" "$END"); do
  id=$(printf "%05d" "$i")
  src="$SUITE/${id}.c"
  exp="$SUITE/${id}.c.expected"
  if [[ ! -f "$src" ]]; then
    continue
  fi
  # expected file may be empty
  if [[ ! -f "$exp" ]]; then
    exp_out=""
    exp_ret=0
  else
    # c-testsuite expected is full stdout; exit is always 0 when match file used
    # Format: some have only empty meaning exit 0 no stdout
    exp_out="$(cat "$exp")"
    exp_ret=0
  fi

  out_bin="$WORKDIR/$id"
  rm -f "$out_bin"
  # Per-test wall clock limit (macOS has no GNU timeout by default).
  set +e
  compile_log="$(perl -e 'alarm shift; exec @ARGV' 8 "$BIN" -o "$out_bin" "$src" 2>&1)"
  cstatus=$?
  set -e
  if [[ $cstatus -ne 0 || ! -f "$out_bin" ]]; then
    echo "FAIL $id compile"
    echo "$compile_log" | head -5
    failed=$((failed + 1))
    continue
  fi

  set +e
  actual_out="$(perl -e 'alarm shift; exec @ARGV' 8 "$out_bin" 2>&1)"
  actual_ret=$?
  set -e
  # 142 = 128+14 SIGALRM from perl alarm on some systems; treat as fail
  if [[ $actual_ret -gt 128 ]]; then
    echo "FAIL $id timeout/signal ret=$actual_ret"
    failed=$((failed + 1))
    continue
  fi

  # Command substitution strips trailing newlines — normalize both sides
  # For empty expected, both should be empty and ret 0
  if [[ "$actual_ret" -ne "$exp_ret" ]]; then
    echo "FAIL $id exit actual=$actual_ret expected=$exp_ret"
    failed=$((failed + 1))
    continue
  fi
  if [[ "$actual_out" != "$exp_out" ]]; then
    # also try with trailing newline restored for expected
    if [[ "$actual_out"$'\n' == "$exp_out" || "$actual_out" == "$exp_out"$'\n' ]]; then
      :
    else
      echo "FAIL $id stdout"
      failed=$((failed + 1))
      continue
    fi
  fi

  echo "PASS $id"
  pass_ids+=("$id")
  passed=$((passed + 1))
done

echo "== c-testsuite summary: passed=$passed failed=$failed (range $START-$END) =="
echo "PASS_IDS: ${pass_ids[*]}"

if [[ "$MIN_PASS" -gt 0 && "$passed" -lt "$MIN_PASS" ]]; then
  echo "ERROR: need at least $MIN_PASS passes, got $passed" >&2
  exit 1
fi
exit 0
