#!/usr/bin/env zsh
# Oracle runner: compile each oracles/* fixture with the project compiler and check stdout/exit.
# Success is only match against expected.stdout / expected.ret — not self-reported prose.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BIN="${ACC_BIN:-${GGCC_BIN:-$ROOT/target/release/acc}}"
ORACLE_DIR="$ROOT/oracles"
WORKDIR="${ACC_ORACLE_WORK:-${GGCC_ORACLE_WORK:-$ROOT/target/oracle_work}}"

if [[ ! -x "$BIN" ]]; then
  echo "Building acc (release)..."
  cargo build --release
  BIN="$ROOT/target/release/acc"
fi

if [[ ! -x "$BIN" ]]; then
  echo "ERROR: compiler binary not found at $BIN" >&2
  exit 2
fi

mkdir -p "$WORKDIR"

failures=0
ran=0

for dir in "$ORACLE_DIR"/*(N/); do
  name="${dir:t}"
  src="$dir/main.c"
  exp_out="$dir/expected.stdout"
  exp_ret="$dir/expected.ret"
  if [[ ! -f "$src" ]]; then
    continue
  fi
  if [[ ! -f "$exp_out" || ! -f "$exp_ret" ]]; then
    echo "ERROR: $name missing expected.stdout or expected.ret" >&2
    failures=$((failures + 1))
    continue
  fi

  ran=$((ran + 1))
  out_bin="$WORKDIR/$name"
  rm -f "$out_bin"

  echo "== oracle: $name =="
  if ! "$BIN" -o "$out_bin" "$src"; then
    echo "FAIL $name: compiler exited non-zero"
    failures=$((failures + 1))
    continue
  fi
  if [[ ! -x "$out_bin" && ! -f "$out_bin" ]]; then
    echo "FAIL $name: output binary missing"
    failures=$((failures + 1))
    continue
  fi

  set +e
  actual_out="$("$out_bin" 2>&1)"
  actual_ret=$?
  set -e

  expected_ret="$(tr -d ' \t\r\n' < "$exp_ret")"
  # Normalize trailing newlines for stdout compare (C programs print \n; file may or may not).
  expected_out="$(cat "$exp_out")"
  # Strip a single trailing newline from both for stable compare of "line" content,
  # but preserve internal newlines.
  if [[ "$actual_ret" != "$expected_ret" ]]; then
    echo "FAIL $name: exit code actual=$actual_ret expected=$expected_ret"
    failures=$((failures + 1))
    continue
  fi
  if [[ "$actual_out" != "$expected_out" ]]; then
    echo "FAIL $name: stdout mismatch"
    echo "--- expected ---"
    printf '%s\n' "$expected_out" | cat -A
    echo "--- actual ---"
    printf '%s\n' "$actual_out" | cat -A
    failures=$((failures + 1))
    continue
  fi
  echo "PASS $name"
done

echo "== summary: ran=$ran failures=$failures =="
if [[ "$ran" -eq 0 ]]; then
  echo "ERROR: no oracles found" >&2
  exit 2
fi
if [[ "$failures" -ne 0 ]]; then
  exit 1
fi
exit 0
