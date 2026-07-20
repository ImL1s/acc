#!/usr/bin/env zsh
# Anti-hardcode: change the string in a hello-like program and require stdout to follow.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BIN="${GGCC_BIN:-$ROOT/target/release/ggcc}"
WORKDIR="${GGCC_ORACLE_WORK:-$ROOT/target/oracle_work}/mutation"
mkdir -p "$WORKDIR"

if [[ ! -x "$BIN" ]]; then
  cargo build --release
  BIN="$ROOT/target/release/ggcc"
fi

SRC="$WORKDIR/mut.c"
OUT="$WORKDIR/mut"
MSG="mutation-proof-$(date +%s)"

cat >"$SRC" <<EOF
#include <stdio.h>
int main(void) {
    printf("${MSG}\\n");
    return 0;
}
EOF

"$BIN" -o "$OUT" "$SRC"
got="$("$OUT")"
if [[ "$got" != "$MSG" ]]; then
  echo "FAIL mutation: expected '$MSG' got '$got'" >&2
  exit 1
fi
echo "PASS mutation: stdout follows source string ($MSG)"
