#!/usr/bin/env bash
# Structural anti-bypass / clean-room audit.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
fail=0

do_grep() {
  if command -v rg >/dev/null 2>&1; then
    rg -n "$@"
  else
    grep -rnE "$@"
  fi
}

echo "== clean-room: no CCC tree =="
if [[ -d "$ROOT/claudes-c-compiler" ]] || [[ -d "$ROOT/vendor/claudes-c-compiler" ]]; then
  echo "FAIL: vendored CCC tree present"
  fail=1
fi
if do_grep 'anthropics/claudes-c-compiler|Claude.s C Compiler|ccc-x86' src harness oracles 2>/dev/null; then
  echo "FAIL: CCC provenance markers under implementation paths"
  fail=1
else
  echo "PASS: no CCC markers in src/harness/oracles"
fi

echo "== no external C compiler on user .c in driver =="
# driver must not Command::new("gcc") / clang on the input .c
if do_grep 'Command::new\("(gcc|clang|ccc)"\)' src 2>/dev/null; then
  echo "FAIL: spawns external C compiler by name"
  fail=1
else
  echo "PASS: no gcc/clang/ccc Command::new"
fi

# Ensure compile path reads source and runs parser
if ! do_grep 'parser::parse' src/driver.rs >/dev/null; then
  echo "FAIL: driver does not call parser::parse"
  fail=1
else
  echo "PASS: driver calls parser::parse"
fi

if ! do_grep 'emit_assembly' src/driver.rs >/dev/null; then
  echo "FAIL: driver does not emit assembly via codegen"
  fail=1
else
  echo "PASS: driver uses codegen::emit_assembly"
fi

echo "== no prebuilt fixture binaries in oracles =="
if find oracles -type f \( -perm /111 -o -name '*.o' -o -name 'a.out' \) 2>/dev/null | grep -q .; then
  echo "FAIL: unexpected binaries under oracles/"
  fail=1
else
  echo "PASS: oracles are source + expected text only"
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "ALL anti-bypass checks passed"
