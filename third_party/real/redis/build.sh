#!/usr/bin/env zsh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CC="${CC:-$ROOT/target/release/ggcc}"
if [[ ! -x "$CC" ]]; then
  (cd "$ROOT" && cargo build --release)
  CC="$ROOT/target/release/ggcc"
fi
HERE="$(cd "$(dirname "$0")" && pwd)"
REDIS="$ROOT/third_party/stage_c/redis/redis-7.2.5/src"
WORKDIR="${TMPDIR:-/tmp}/ggcc_redis_$$"
mkdir -p "$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT
"$CC" -S -o "$WORKDIR/sds.s" "$REDIS/sds.c" -I"$REDIS"
"$CC" -S -o "$WORKDIR/zmalloc.s" "$REDIS/zmalloc.c" -I"$REDIS"
"$CC" -S -o "$WORKDIR/smoke.s" "$HERE/smoke.c" -I"$REDIS"
cc -o "$HERE/redis_bin" "$WORKDIR/sds.s" "$WORKDIR/zmalloc.s" "$WORKDIR/smoke.s"
case "${1:-test}" in
  test)
    out="$("$HERE/redis_bin")"
    ret=$?
    echo "redis exit=$ret out=$out"
    # Accept expanded basic suite (npass=…) or legacy one-line smoke.
    [[ "$ret" -eq 0 && ( "$out" == redis\ ok\ basic\ npass=*\ sds=redis-ok-7\ len=10 || \
      "$out" == redis\ ok\ sds=redis-ok-7\ len=10 ) ]]
    ;;
  build) ;;
  *) echo "usage: $0 [test|build]"; exit 2 ;;
esac
