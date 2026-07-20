#!/usr/bin/env zsh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CC="${CC:-$ROOT/target/release/ggcc}"
if [[ ! -x "$CC" ]]; then
  (cd "$ROOT" && cargo build --release)
  CC="$ROOT/target/release/ggcc"
fi
cd "$(dirname "$0")"
"$CC" -o tinyc_bin main.c
case "${1:-test}" in
  test)
    out="$(./tinyc_bin)"
    ret=$?
    echo "tinyc exit=$ret out=$out"
    [[ "$ret" -eq 0 && "$out" == "tinyc ok" ]]
    ;;
  *) ;;
esac
