#!/usr/bin/env bash
# GCC c-torture execute subset runner (D-torture T1–T2).
#
# Vendors public FSF GCC tests under third_party/gcc-torture (symlink to sparse
# gcc-sparse/gcc/testsuite/gcc.c-torture/execute). Falls back to curated
# c-testsuite IDs only when the vendor tree is absent.
#
# GCC mode log: scratch/torture_gcc_subset.log
# Interim fallback: scratch/torture_subset.log
#
# Never marks PASS in the ledger; evidence only.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SCRATCH="${SCRATCH:-$ROOT/scratch}"
mkdir -p "$SCRATCH"

BIN="${ACC_BIN:-${GGCC_BIN:-$ROOT/target/release/acc}}"
SUITE="${CTESTSUITE_DIR:-$ROOT/third_party/c-testsuite/tests/single-exec}"
WORKDIR="${ACC_ORACLE_WORK:-${GGCC_TORTURE_WORK:-$ROOT/target/torture_work}}"
TORTURE_DIR="${TORTURE_DIR:-$ROOT/third_party/gcc-torture}"
IMAGE="${ACC_DOCKER_IMAGE:-${GGCC_DOCKER_IMAGE:-acc-linux}}"
PLATFORM="${GGCC_DOCKER_PLATFORM:-linux/amd64}"
# Status-track default: x86_64/linux (ggcc defaults to -m aarch64 — wrong for Docker amd64).
ARCH="${GGCC_ARCH:-${ACC_ARCH:-x86_64}}"
TARGET_OS="${GGCC_TARGET_OS:-${ACC_TARGET_OS:-linux}}"
GGCC_FLAGS=(-m "$ARCH" --target-os "$TARGET_OS")

# Interim c-testsuite IDs when vendor tree missing.
DEFAULT_IDS=(00001 00002 00005 00010 00020 00030 00040 00050 00060 00070 00080 00090 00100)

stamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }

need_docker() {
  [[ "$(uname -s)" != "Linux" ]] || return 1
  [[ -f "$BIN" ]] || return 0
  file -b "$BIN" 2>/dev/null | grep -q 'ELF.*x86-64'
}

run_in_docker() {
  local inner_log="/work/scratch/torture_gcc_subset.log"
  echo "re-exec in docker ($IMAGE) for Linux x86_64 ggcc…" >&2
  if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "docker image $IMAGE missing; building…" >&2
    docker build -t "$IMAGE" -f "$ROOT/harness/docker/Dockerfile.linux" "$ROOT/harness/docker"
  fi
  # Map host TORTURE_LOG into the container work tree when under $ROOT.
  local docker_log="${TORTURE_LOG:-}"
  if [[ -n "$docker_log" && "$docker_log" == "$ROOT"/* ]]; then
    docker_log="/work/${docker_log#"$ROOT"/}"
  elif [[ -n "$docker_log" && "$docker_log" != /* ]]; then
    docker_log="/work/$docker_log"
  fi
  docker run --rm --platform "$PLATFORM" \
    -v "$ROOT:/work" -w /work \
    -e TORTURE_DIR="/work/third_party/gcc-torture" \
    -e TORTURE_LIMIT="${TORTURE_LIMIT:-100}" \
    -e TORTURE_LOG="${docker_log}" \
    -e GGCC_BIN="/work/target-linux/release/ggcc" \
    -e GGCC_ARCH="${ARCH}" \
    -e GGCC_TARGET_OS="${TARGET_OS}" \
    -e ACC_ARCH="${ARCH}" \
    -e ACC_TARGET_OS="${TARGET_OS}" \
    -e GGCC_USE_DOCKER=0 \
    "$IMAGE" bash -lc '
set -euo pipefail
export PATH=/usr/local/cargo/bin:/usr/bin:/bin:$PATH
if [[ ! -x "$GGCC_BIN" ]]; then
  echo "building target-linux ggcc in container…" >&2
  CARGO_TARGET_DIR=/work/target-linux cargo build --release
fi
exec bash /work/harness/run_torture_subset.sh
'
  exit $?
}

if [[ "${GGCC_USE_DOCKER:-auto}" == "1" ]] || { [[ "${GGCC_USE_DOCKER:-auto}" == "auto" ]] && need_docker; }; then
  run_in_docker
fi

mode="c-testsuite-interim"
LOG="$SCRATCH/torture_subset.log"
ids=()

torture_has_c() {
  [[ -e "$TORTURE_DIR" ]] || return 1
  local first
  first="$(find -L "$TORTURE_DIR" -maxdepth 1 -type f -name '*.c' -print -quit 2>/dev/null || true)"
  [[ -n "$first" ]]
}

if torture_has_c; then
  mode="gcc-torture-execute"
  LOG="${TORTURE_LOG:-$SCRATCH/torture_gcc_subset.log}"
  limit="${TORTURE_LIMIT:-100}"
  while IFS= read -r f; do
    [[ ${#ids[@]} -lt "$limit" ]] || break
    ids+=("$f")
  done < <(find -L "$TORTURE_DIR" -maxdepth 1 -type f -name '*.c' | sort)
else
  ids=("${DEFAULT_IDS[@]}")
  LOG="${TORTURE_LOG:-$SCRATCH/torture_subset.log}"
fi

: >"$LOG"

{
  echo "=== torture_subset start $(stamp) ==="
  echo "root=$ROOT"
  echo "bin=$BIN"
  echo "suite=$SUITE"
  echo "torture_dir=$TORTURE_DIR"
  echo "arch=$ARCH target_os=$TARGET_OS"
  echo "ggcc_flags=${GGCC_FLAGS[*]}"
  echo "host=$(uname -s)/$(uname -m)"
} | tee -a "$LOG"

if [[ ! -x "$BIN" ]]; then
  echo "building release ggcc..." | tee -a "$LOG"
  cargo build --release 2>&1 | tee -a "$LOG" | tail -5
  BIN="$ROOT/target/release/ggcc"
fi

if [[ ! -x "$BIN" ]]; then
  echo "ERROR: ggcc binary missing: $BIN" | tee -a "$LOG"
  exit 2
fi

if [[ "$mode" == "gcc-torture-execute" ]]; then
  echo "mode=$mode vendor=FSF/gcc-sparse files=${#ids[@]} limit=${TORTURE_LIMIT:-100}" | tee -a "$LOG"
  echo "vendor_path=$TORTURE_DIR" | tee -a "$LOG"
else
  echo "mode=$mode (no vendor tree; using curated c-testsuite IDs)" | tee -a "$LOG"
  echo "note=NOT full GCC torture; ledger must stay TODO until real suite + ~99%" | tee -a "$LOG"
fi

mkdir -p "$WORKDIR"
passed=0
failed=0
skipped=0
fail_compile=()
fail_run=()

run_one_ctest() {
  local id="$1"
  local src="$SUITE/${id}.c"
  local exp="$SUITE/${id}.c.expected"
  local out_bin="$WORKDIR/$id"
  if [[ ! -f "$src" ]]; then
    echo "SKIP $id (missing src)" | tee -a "$LOG"
    skipped=$((skipped + 1))
    return 0
  fi
  rm -f "$out_bin"
  local compile_log
  set +e
  compile_log="$(perl -e 'alarm shift; exec @ARGV' 15 "$BIN" "${GGCC_FLAGS[@]}" -o "$out_bin" "$src" 2>&1)"
  local cstatus=$?
  set -e
  if [[ $cstatus -ne 0 || ! -f "$out_bin" ]]; then
    echo "FAIL $id compile" | tee -a "$LOG"
    echo "$compile_log" | tail -20 | tee -a "$LOG"
    failed=$((failed + 1))
    fail_compile+=("$id")
    return 0
  fi
  local got_out got_ec
  set +e
  got_out="$(perl -e 'alarm shift; exec @ARGV' 5 "$out_bin" 2>&1)"
  got_ec=$?
  set -e
  local exp_out=""
  if [[ -f "$exp" ]]; then
    exp_out="$(cat "$exp")"
  fi
  if [[ "$got_out" == "$exp_out" && $got_ec -eq 0 ]]; then
    echo "PASS $id" | tee -a "$LOG"
    passed=$((passed + 1))
  else
    echo "FAIL $id run ec=$got_ec" | tee -a "$LOG"
    failed=$((failed + 1))
    fail_run+=("$id")
  fi
}

run_one_file() {
  local src="$1"
  local base
  base="$(basename "$src" .c)"
  local out_bin="$WORKDIR/$base"
  rm -f "$out_bin"
  set +e
  local compile_log
  compile_log="$(perl -e 'alarm shift; exec @ARGV' 30 "$BIN" "${GGCC_FLAGS[@]}" -o "$out_bin" "$src" 2>&1)"
  local cstatus=$?
  set -e
  if [[ $cstatus -ne 0 || ! -f "$out_bin" ]]; then
    echo "FAIL $base compile" | tee -a "$LOG"
    echo "$compile_log" | tail -15 | tee -a "$LOG"
    failed=$((failed + 1))
    fail_compile+=("$base")
    return 0
  fi
  set +e
  perl -e 'alarm shift; exec @ARGV' 5 "$out_bin" >/dev/null 2>&1
  local ec=$?
  set -e
  if [[ $ec -eq 0 ]]; then
    echo "PASS $base" | tee -a "$LOG"
    passed=$((passed + 1))
  else
    echo "FAIL $base run ec=$ec" | tee -a "$LOG"
    failed=$((failed + 1))
    fail_run+=("$base")
  fi
}

if [[ "$mode" == "gcc-torture-execute" ]]; then
  for f in "${ids[@]}"; do
    run_one_file "$f"
  done
else
  for id in "${ids[@]}"; do
    run_one_ctest "$id"
  done
fi

attempted=$((passed + failed))
total=$((attempted + skipped))
rate="0.0"
if [[ $attempted -gt 0 ]]; then
  rate="$(awk "BEGIN {printf \"%.1f\", ($passed/$attempted)*100}")"
fi

{
  echo "=== torture_subset summary $(stamp) ==="
  echo "mode=$mode"
  echo "arch=$ARCH target_os=$TARGET_OS"
  echo "passed=$passed"
  echo "pass=$passed"
  echo "fail=$failed"
  echo "failed=$failed"
  echo "skipped=$skipped"
  echo "total=$total"
  echo "attempted=$attempted"
  echo "rate=${rate}%"
  echo "pass=${passed} fail=${failed} total=${total} rate=${rate}%"
  if [[ "$mode" == "gcc-torture-execute" ]]; then
    echo "vendor_path=$TORTURE_DIR"
    echo "bar=IN_PROGRESS_not_99pct"
    if [[ ${#fail_compile[@]} -gt 0 ]]; then
      echo "fail_compile_ids=${fail_compile[*]}"
    fi
    if [[ ${#fail_run[@]} -gt 0 ]]; then
      echo "fail_run_ids=${fail_run[*]}"
    fi
  else
    echo "bar=interim_only_not_gcc_torture_99pct"
  fi
  echo "=== torture_subset end ==="
} | tee -a "$LOG"

# Exit 0 even with failures so agents can collect evidence; CI may check failed==0 later.
exit 0
