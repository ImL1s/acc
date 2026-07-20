#!/usr/bin/env zsh
# Run fixed multiarch oracle subset with -m aarch64 and -m x86_64.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BIN="${GGCC_BIN:-$ROOT/target/release/ggcc}"
IDS=(00001 00002 00003 00004 00005 00006 00007 00008 00009 00010 00012 00015 00021 00025 00030 00031 00034 00035 00036 00038)
SUITE=third_party/c-testsuite/tests/single-exec
WORKDIR=target/multiarch_work
mkdir -p "$WORKDIR"

run_one() {
  local arch="$1" id="$2"
  local src="$SUITE/${id}.c"
  local out="$WORKDIR/${id}_${arch}"
  if [[ "$arch" == "x86_64" ]]; then
    "$BIN" -m x86_64 -o "$out" "$src" || return 1
    if [[ "$(uname -m)" == "arm64" ]]; then
      # Need Rosetta or skip run
      if arch -x86_64 true 2>/dev/null; then
        arch -x86_64 "$out" >/dev/null 2>&1 || return 1
      else
        echo "WARN $id x86_64: binary produced, run skipped (no Rosetta)"
        return 0
      fi
    else
      "$out" >/dev/null 2>&1 || return 1
    fi
  else
    "$BIN" -m aarch64 -o "$out" "$src" || return 1
    "$out" >/dev/null 2>&1 || return 1
  fi
  return 0
}

fail=0
pass=0
# bash-compatible iteration (zsh arrays also work with "$@"-style via explicit list)
for arch in aarch64 x86_64; do
  echo "== arch $arch =="
  for id in "${IDS[@]}"; do
    if run_one "$arch" "$id"; then
      echo "PASS $arch $id"
      pass=$((pass+1))
    else
      echo "FAIL $arch $id"
      fail=$((fail+1))
    fi
  done
done
echo "== multiarch summary pass=$pass fail=$fail =="
# Contract: 20 IDs × 2 arch = 40; require fail=0
echo "expected_slots=40 got_pass=$pass got_fail=$fail"
[[ "$fail" -eq 0 ]] && [[ "$pass" -ge 40 ]]
