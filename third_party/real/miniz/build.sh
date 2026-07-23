#!/usr/bin/env zsh
# Stage B real project: miniz-3.0.2 amalgamation + compress/uncompress roundtrip.
# CC=ggcc compiles .c → .s; system cc assembles/links only.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CC="${CC:-$ROOT/target/release/ggcc}"
if [[ ! -x "$CC" ]]; then
  (cd "$ROOT" && cargo build --release)
  CC="$ROOT/target/release/ggcc"
fi
cd "$(dirname "$0")"

"$CC" -S -o miniz.s miniz.c
"$CC" -S -o test_compress.s test_compress.c -I.
cc -o miniz_bin miniz.s test_compress.s

case "${1:-test}" in
  test)
    out="$(./miniz_bin)"
    ret=$?
    echo "miniz exit=$ret out=$out"
    [[ "$ret" -eq 0 && "$out" == miniz\ ok* ]]
    ;;
  build) ;;
  *) echo "usage: $0 [test|build]"; exit 2 ;;
esac
