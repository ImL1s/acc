#!/usr/bin/env zsh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CC="${CC:-$ROOT/target/release/ggcc}"
if [[ ! -x "$CC" ]]; then
  (cd "$ROOT" && cargo build --release)
  CC="$ROOT/target/release/ggcc"
fi
cd "$(dirname "$0")"
# Multi-TU: compile each .c then link objects with system cc (link only)
"$CC" -S -o adler.s adler.c
"$CC" -S -o main.s main.c
cc -o miniz_bin adler.s main.s
case "${1:-test}" in
  test)
    out="$(./miniz_bin)"
    ret=$?
    echo "miniz_smoke exit=$ret out=$out"
    [[ "$ret" -eq 0 && "$out" == "miniz_smoke ok" ]]
    ;;
esac
