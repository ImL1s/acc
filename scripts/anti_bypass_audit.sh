#!/usr/bin/env zsh
# Structural anti-bypass / clean-room audit.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
fail=0

GREP="$(command -v rg || echo "grep -E")"

echo "== clean-room: no CCC tree =="
if [[ -d "$ROOT/claudes-c-compiler" ]] || [[ -d "$ROOT/vendor/claudes-c-compiler" ]]; then
  echo "FAIL: vendored CCC tree present"
  fail=1
fi
if $GREP -n --exclude-dir=target 'anthropics/claudes-c-compiler|Claude.s C Compiler|ccc-x86' src harness oracles 2>/dev/null; then
  echo "FAIL: CCC provenance markers under implementation paths"
  fail=1
else
  echo "PASS: no CCC markers in src/harness/oracles"
fi

echo "== no external C compiler on user .c in driver =="
# driver must not Command::new("gcc") / clang on the input .c
if $GREP -n 'Command::new\("(gcc|clang|ccc)"\)' src 2>/dev/null; then
  echo "FAIL: spawns external C compiler by name"
  fail=1
else
  echo "PASS: no gcc/clang/ccc Command::new"
fi

# Ensure compile path reads source and runs parser
if ! $GREP -n 'parser::parse' src/driver.rs >/dev/null; then
  echo "FAIL: driver does not call parser::parse"
  fail=1
else
  echo "PASS: driver calls parser::parse"
fi

if ! $GREP -n 'emit_assembly' src/driver.rs >/dev/null; then
  echo "FAIL: driver does not emit assembly via codegen"
  fail=1
else
  echo "PASS: driver uses codegen::emit_assembly"
fi

echo "== no prebuilt fixture binaries in oracles =="
if find oracles -type f \( -perm +111 -o -name '*.o' -o -name 'a.out' \) 2>/dev/null | grep -q .; then
  echo "FAIL: unexpected binaries under oracles/"
  fail=1
else
  echo "PASS: oracles are source + expected text only"
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "ALL anti-bypass checks passed"
