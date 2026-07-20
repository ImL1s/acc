#!/usr/bin/env zsh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CC="${CC:-$ROOT/target/release/ggcc}"
if [[ ! -x "$CC" ]]; then
  (cd "$ROOT" && cargo build --release)
  CC="$ROOT/target/release/ggcc"
fi
cd "$(dirname "$0")"
"$CC" -o lua_bin main.c
case "${1:-test}" in
  test)
    out="$(./lua_bin)"
    ret=$?
    echo "lua_smoke exit=$ret out=$out"
    [[ "$ret" -eq 0 && "$out" == "lua_smoke ok" ]]
    ;;
esac
