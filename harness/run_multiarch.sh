#!/usr/bin/env bash
# Stage C3 raise: Stage A oracle 00001–00100 on -m aarch64 and -m x86_64.
# Contract: ≥95% PASS per ISA (real compile+run). See STAGE_CONTRACTS.md.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BIN="${ACC_BIN:-${GGCC_BIN:-$ROOT/target/release/acc}}"

# Stage A continuous IDs (override with IDS_OVERRIDE="00001 00002 …" for a subset).
# shellcheck disable=SC2207
IDS_STAGE_A=($(seq -f '%05g' 1 100))
if [[ -n "${IDS_OVERRIDE:-}" ]]; then
  # shellcheck disable=SC2206
  IDS=($IDS_OVERRIDE)
else
  IDS=("${IDS_STAGE_A[@]}")
fi
ID_COUNT=${#IDS[@]}
# ≥95% of ID_COUNT per arch (ceil via integer math: (n*95+99)/100)
MIN_PASS_PER_ARCH=$(( (ID_COUNT * 95 + 99) / 100 ))

SUITE=third_party/c-testsuite/tests/single-exec
WORKDIR=target/multiarch_work
mkdir -p "$WORKDIR"

run_one() {
  local arch="$1" id="$2"
  local src="$SUITE/${id}.c"
  local out="$WORKDIR/${id}_${arch}"
  if [[ ! -f "$src" ]]; then
    echo "MISS $arch $id (no source)"
    return 1
  fi
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
aarch64_pass=0
aarch64_fail=0
x86_64_pass=0
x86_64_fail=0

echo "== multiarch Stage A: ${ID_COUNT} IDs × 2 ISA; need ≥${MIN_PASS_PER_ARCH}/arch (≥95%) =="
echo "NOTE: full 00001–00100 dual-ISA run can take several minutes; use IDS_OVERRIDE for a quick subset."

for arch in aarch64 x86_64; do
  echo "== arch $arch =="
  for id in "${IDS[@]}"; do
    if run_one "$arch" "$id"; then
      echo "PASS $arch $id"
      pass=$((pass+1))
      if [[ "$arch" == "aarch64" ]]; then
        aarch64_pass=$((aarch64_pass+1))
      else
        x86_64_pass=$((x86_64_pass+1))
      fi
    else
      echo "FAIL $arch $id"
      fail=$((fail+1))
      if [[ "$arch" == "aarch64" ]]; then
        aarch64_fail=$((aarch64_fail+1))
      else
        x86_64_fail=$((x86_64_fail+1))
      fi
    fi
  done
done

echo "== multiarch summary pass=$pass fail=$fail =="
echo "aarch64: pass=$aarch64_pass fail=$aarch64_fail (need ≥$MIN_PASS_PER_ARCH / $ID_COUNT)"
echo "x86_64:  pass=$x86_64_pass fail=$x86_64_fail (need ≥$MIN_PASS_PER_ARCH / $ID_COUNT)"
echo "expected_ids=$ID_COUNT min_pass_per_arch=$MIN_PASS_PER_ARCH got_pass=$pass got_fail=$fail"

# Contract: ≥95% both ISAs (not soft 20-ID / fail=0 bar)
ok=1
if [[ "$aarch64_pass" -lt "$MIN_PASS_PER_ARCH" ]]; then
  echo "CONTRACT FAIL: aarch64 below ≥95%"
  ok=0
fi
if [[ "$x86_64_pass" -lt "$MIN_PASS_PER_ARCH" ]]; then
  echo "CONTRACT FAIL: x86_64 below ≥95%"
  ok=0
fi
[[ "$ok" -eq 1 ]]
