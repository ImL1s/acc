#!/usr/bin/env bash
# Stage C2 extras: zlib smoke under ggcc (Status extras / ledger zlib row).
#
# Looks for a vendored zlib tree under third_party, then attempts a minimal
# compress/decompress smoke with ggcc_cc_wrapper (or host ggcc → .s + system cc).
# If sources are missing or the smoke is not ready, writes scratch/c2_zlib.log
# and exits nonzero (OK for Status extras track; ledger stays TODO until PASS).
#
# Usage:
#   export SCRATCH=${SCRATCH:-$PWD/scratch}
#   bash harness/docker/run_zlib_smoke.sh
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="${SCRATCH:-$ROOT/scratch}"
LOG="$SCRATCH/c2_zlib.log"
WRAP="${ACC_CC_WRAPPER:-$ROOT/harness/docker/acc_cc_wrapper.sh}"
ACC="${ACC_BIN:-$ROOT/target/release/acc}"

mkdir -p "$SCRATCH"

{
  echo "# zlib smoke attempt"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(uname -a)"
  echo "SCRATCH=$SCRATCH"
  echo "ROOT=$ROOT"
} >"$LOG"

# Candidate trees (first hit wins). Official zlib expected; miniz is Stage B soft only.
CANDIDATES=(
  "$ROOT/third_party/stage_c/zlib"
  "$ROOT/third_party/zlib"
  "$ROOT/third_party/real/zlib"
)

ZLIB_SRC=""
for d in "${CANDIDATES[@]}"; do
  if [[ -f "$d/zlib.h" && ( -f "$d/adler32.c" || -f "$d/deflate.c" || -f "$d/zlib.c" ) ]]; then
    ZLIB_SRC="$d"
    break
  fi
done

# Also accept a single amalgamation if present
if [[ -z "$ZLIB_SRC" ]]; then
  for f in \
    "$ROOT/third_party/stage_c/zlib/zlib.c" \
    "$ROOT/third_party/zlib/zlib.c"; do
    if [[ -f "$f" ]]; then
      ZLIB_SRC="$(dirname "$f")"
      break
    fi
  done
fi

{
  echo "candidates_checked:"
  for d in "${CANDIDATES[@]}"; do
    echo "  - $d (exists=$([[ -d $d ]] && echo yes || echo no))"
  done
  echo "ZLIB_SRC=${ZLIB_SRC:-MISSING}"
} | tee -a "$LOG"

if [[ -z "$ZLIB_SRC" ]]; then
  {
    echo "status: NOT READY — no third_party zlib tree (zlib.h + sources)"
    echo "note: third_party/real/miniz is Stage B soft bar; does not satisfy CCC zlib row"
    echo "VERDICT: TODO — exit 2"
  } | tee -a "$LOG"
  exit 2
fi

WORKDIR="$SCRATCH/zlib_smoke"
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"

# Minimal smoke: inflate/deflate roundtrip via zlib.h API
cat >"$WORKDIR/zsmoke.c" <<'C'
#include "zlib.h"
#include <stdio.h>
#include <string.h>
int main(void) {
  const char *in = "ggcc-zlib-smoke";
  unsigned char out[128];
  unsigned char back[128];
  uLongf out_len = sizeof(out);
  uLongf back_len = sizeof(back);
  if (compress(out, &out_len, (const Bytef *)in, (uLong)strlen(in)) != Z_OK) {
    printf("compress fail\n");
    return 1;
  }
  if (uncompress(back, &back_len, out, out_len) != Z_OK) {
    printf("uncompress fail\n");
    return 2;
  }
  back[back_len] = '\0';
  if (strcmp((char *)back, in) != 0) {
    printf("roundtrip mismatch\n");
    return 3;
  }
  printf("zlib_smoke_ok\n");
  return 0;
}
C

echo "ZLIB_SRC=$ZLIB_SRC" | tee -a "$LOG"
INC=(-I"$ZLIB_SRC")

# Prefer wrapper (docker C2 path); fall back to ggcc -S + system cc on host.
set +e
if [[ -x "$WRAP" ]]; then
  echo "build: ggcc_cc_wrapper" | tee -a "$LOG"
  # Build common zlib objects if present
  objs=()
  for src in adler32 compress crc32 deflate infback inffast inflate inftrees trees uncompr zutil; do
    if [[ -f "$ZLIB_SRC/${src}.c" ]]; then
      "$WRAP" -c -o "$WORKDIR/${src}.o" "$ZLIB_SRC/${src}.c" "${INC[@]}" >>"$LOG" 2>&1
      ec=$?
      echo "compile_${src}_ec=$ec" | tee -a "$LOG"
      if [[ $ec -ne 0 ]]; then
        echo "VERDICT: FAIL — compile ${src}.c ec=$ec" | tee -a "$LOG"
        exit 1
      fi
      objs+=("$WORKDIR/${src}.o")
    fi
  done
  "$WRAP" -c -o "$WORKDIR/zsmoke.o" "$WORKDIR/zsmoke.c" "${INC[@]}" >>"$LOG" 2>&1
  ec_smoke=$?
  echo "compile_zsmoke_ec=$ec_smoke" | tee -a "$LOG"
  if [[ $ec_smoke -ne 0 ]]; then
    echo "VERDICT: FAIL — compile zsmoke.c" | tee -a "$LOG"
    exit 1
  fi
  cc -o "$WORKDIR/zsmoke" "$WORKDIR/zsmoke.o" "${objs[@]}" >>"$LOG" 2>&1
  ec_link=$?
  echo "link_ec=$ec_link" | tee -a "$LOG"
  if [[ $ec_link -ne 0 ]]; then
    echo "VERDICT: FAIL — link" | tee -a "$LOG"
    exit 1
  fi
  "$WORKDIR/zsmoke" 2>&1 | tee -a "$LOG"
  ec_run=${PIPESTATUS[0]}
elif [[ -x "$GGCC" ]]; then
  echo "build: ggcc -S + system cc" | tee -a "$LOG"
  asm_objs=()
  for src in adler32 compress crc32 deflate infback inffast inflate inftrees trees uncompr zutil; do
    if [[ -f "$ZLIB_SRC/${src}.c" ]]; then
      "$GGCC" --target-os linux -S -o "$WORKDIR/${src}.s" "$ZLIB_SRC/${src}.c" "${INC[@]}" >>"$LOG" 2>&1
      ec=$?
      echo "ggcc_${src}_ec=$ec" | tee -a "$LOG"
      [[ $ec -eq 0 ]] || { echo "VERDICT: FAIL — ggcc ${src}.c" | tee -a "$LOG"; exit 1; }
      asm_objs+=("$WORKDIR/${src}.s")
    fi
  done
  "$GGCC" --target-os linux -S -o "$WORKDIR/zsmoke.s" "$WORKDIR/zsmoke.c" "${INC[@]}" >>"$LOG" 2>&1
  ec_smoke=$?
  echo "ggcc_zsmoke_ec=$ec_smoke" | tee -a "$LOG"
  [[ $ec_smoke -eq 0 ]] || { echo "VERDICT: FAIL — ggcc zsmoke.c" | tee -a "$LOG"; exit 1; }
  cc -o "$WORKDIR/zsmoke" "$WORKDIR/zsmoke.s" "${asm_objs[@]}" >>"$LOG" 2>&1
  ec_link=$?
  echo "link_ec=$ec_link" | tee -a "$LOG"
  [[ $ec_link -eq 0 ]] || { echo "VERDICT: FAIL — link" | tee -a "$LOG"; exit 1; }
  "$WORKDIR/zsmoke" 2>&1 | tee -a "$LOG"
  ec_run=${PIPESTATUS[0]}
else
  {
    echo "status: NOT READY — no ggcc_cc_wrapper and no target/release/ggcc"
    echo "VERDICT: TODO — exit 2"
  } | tee -a "$LOG"
  exit 2
fi
set -e

if [[ "${ec_run:-1}" -eq 0 ]] && grep -q zlib_smoke_ok "$LOG"; then
  echo "VERDICT: PASS — zlib_smoke_ok" | tee -a "$LOG"
  exit 0
fi

echo "VERDICT: FAIL — smoke run ec=${ec_run:-unset}" | tee -a "$LOG"
exit 1
