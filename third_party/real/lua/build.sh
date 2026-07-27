#!/usr/bin/env zsh
# Stage B real project: Lua 5.4.6 multi-file under CC=acc.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CC="${ACC_CC:-${CC:-$ROOT/target/release/acc}}"
if [[ ! -x "$CC" ]]; then
  (cd "$ROOT" && cargo build --release)
  CC="$ROOT/target/release/acc"
fi
LUA_SRC="$ROOT/third_party/real/lua-5.4.6/src"
HERE="$(cd "$(dirname "$0")" && pwd)"
if [[ ! -d "$LUA_SRC" ]]; then
  echo "WARN: $LUA_SRC not found, delegating to lua_smoke"
  exec "$HERE/../lua_smoke/build.sh" "$@"
fi
WORKDIR="${TMPDIR:-/tmp}/acc_lua_build_$$"
mkdir -p "$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT

FLAGS=(-DLUA_USE_JUMPTABLE=0 -DLUA_NOBUILTIN=1 -I"$LUA_SRC")
CORE=(
  lapi lauxlib lbaselib lcode lcorolib lctype ldblib ldebug ldo ldump
  lfunc lgc llex lmathlib lmem loadlib lobject lopcodes lparser lstate
  lstring lstrlib ltable ltablib ltm lundump lutf8lib lvm lzio lua
)
objs=()
for f in $CORE; do
  "$CC" -S -o "$WORKDIR/${f}.s" "$LUA_SRC/${f}.c" "${FLAGS[@]}"
  objs+=("$WORKDIR/${f}.s")
done
"$CC" -S -o "$WORKDIR/linit_acc.s" "$HERE/linit_acc.c" "${FLAGS[@]}"
objs+=("$WORKDIR/linit_acc.s")
cc -o "$HERE/lua_bin" "${objs[@]}" -lm

case "${1:-test}" in
  test)
    # Prefer -e (non-interactive). Fall back to stdin script file.
    out="$("$HERE/lua_bin" -e 'print("lua ok", 6*7)' 2>&1)" || true
    ret=$?
    if [[ "$ret" -ne 0 || "$out" != *"lua ok"* || "$out" != *"42"* ]]; then
      script="$WORKDIR/smoke.lua"
      printf '%s\n' 'print("lua ok", 6*7)' > "$script"
      out="$("$HERE/lua_bin" "$script" 2>&1)"
      ret=$?
    fi
    echo "lua exit=$ret out=$out"
    [[ "$ret" -eq 0 && "$out" == *"lua ok"* && "$out" == *"42"* ]]
    ;;
  build) ;;
  *) echo "usage: $0 [test|build]"; exit 2 ;;
esac
