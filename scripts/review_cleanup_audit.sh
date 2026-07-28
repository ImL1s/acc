#!/usr/bin/env bash
# Review Cleanup Verification Audit Script for acc
# Audits all acceptance criteria programmatically:
# 1. No assertive "Goal: COMPLETE" claims in docs/, no false PASS claims for Postgres 237/GCC Torture, and DESIGN_DOC.md source tree includes codegen_i686, codegen_riscv, assembler, linker
# 2. ci.yml does not contain '|| true' after fmt check
# 3. ci.yml does not contain '||' fallback in binary verification step (acc --help)
# 4. 'cat -A' does not appear in harness/ or scripts/
# 5. scripts/anti_bypass_audit.sh does not use '--exclude-dir'
# 6. .gitignore contains rules for a.out and *.s
# 7. README.md status badge does not say 'RELEASE-0.1.0'

set -euo pipefail

# Ensure execution is relative to the project root
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

errors=0

echo "========================================="
echo "   ACC Review Cleanup Acceptance Audit   "
echo "========================================="

# --- Check 1: Documentation Consistency & DESIGN_DOC Tree ---
echo "[Check 1/4] Auditing documentation consistency and DESIGN_DOC tree (R1)..."
check1_fail=0

# Search for assertive "Goal: COMPLETE" or "Status: COMPLETE" status claims
# Exclude lines containing negative assertions ("NOT complete") or instruction context ("Stamp", "only when", "until", etc.)
assertive_matches=$(grep -rnEi "(\*\*Goal:[[:space:]]*COMPLETE\*\*|Goal:[[:space:]]*\*\*COMPLETE\*\*|Goal[[:space:]]+is[[:space:]]+(\*\*)?COMPLETE|##[[:space:]]*Status:[[:space:]]*\*\*COMPLETE\*\*|\|[[:space:]]*Goal:[[:space:]]*COMPLETE[[:space:]]*\|)" docs/ 2>/dev/null | grep -vEi ":[0-9]+:.*(NOT|only|Stamp|edit|until|must|when|if)" || true)

if [ -n "$assertive_matches" ]; then
    echo "  FAIL: Found assertive 'Goal: COMPLETE' claims in docs/:"
    printf '%s\n' "$assertive_matches" | sed 's/^/    /'
    check1_fail=1
fi

# Audit docs/HANDOFF_CCC_STATUS_COMPLETE.md and docs/notes/ccc_status_snapshot.md for false claims if present
for doc_file in "docs/HANDOFF_CCC_STATUS_COMPLETE.md" "docs/notes/ccc_status_snapshot.md"; do
    if [ -f "$doc_file" ]; then
        if grep -qEi "Torture.*100%|100% on declared track" "$doc_file" 2>/dev/null; then
            echo "  FAIL: $doc_file contains unverified '100%' GCC Torture claim"
            check1_fail=1
        fi
        if grep -qEi "Postgres 237.*PASS|237/237 PASS|237/237 regression tests green" "$doc_file" 2>/dev/null; then
            echo "  FAIL: $doc_file contains unverified 'Postgres 237/237 PASS' claim"
            check1_fail=1
        fi
    fi
done

# Check DESIGN_DOC.md for required modules: codegen_i686, codegen_riscv, assembler, linker
missing_modules=0
for mod in "codegen_i686" "codegen_riscv" "assembler" "linker"; do
    if ! grep -q "$mod" DESIGN_DOC.md 2>/dev/null; then
        echo "  FAIL: DESIGN_DOC.md missing reference to module '$mod'."
        missing_modules=1
    fi
done
if [ "$missing_modules" -eq 1 ]; then
    check1_fail=1
fi

if [ "$check1_fail" -eq 0 ]; then
    echo "  PASS: Documentation consistency verified (no assertive completion claims, DESIGN_DOC tree verified)."
else
    errors=$((errors + 1))
fi

# --- Check 2: CI Fail-Open Prevention ---
echo "[Check 2/4] Auditing CI integrity in .github/workflows/ci.yml (R2)..."
check2_fail=0
if grep -nE 'cargo fmt.*[|][|][[:space:]]*true' .github/workflows/ci.yml >/dev/null 2>&1; then
    echo "  FAIL: .github/workflows/ci.yml contains '|| true' after fmt check"
    grep -nE 'cargo fmt.*[|][|][[:space:]]*true' .github/workflows/ci.yml | sed 's/^/    /'
    check2_fail=1
fi

if grep -nE 'acc --help[[:space:]]*[|][|]' .github/workflows/ci.yml >/dev/null 2>&1; then
    echo "  FAIL: .github/workflows/ci.yml contains '||' fallback in binary verification step"
    grep -nE 'acc --help[[:space:]]*[|][|]' .github/workflows/ci.yml | sed 's/^/    /'
    check2_fail=1
fi

if [ "$check2_fail" -eq 0 ]; then
    echo "  PASS: CI configuration is fail-closed (fmt check and --help verification strict)."
else
    errors=$((errors + 1))
fi

# --- Check 3: Cross-Platform Compatibility ---
echo "[Check 3/4] Auditing cross-platform compatibility in harness/ and scripts/ (R3)..."
check3_fail=0

cat_a_matches=$(grep -rnE '(^|[[:space:]])cat[[:space:]]+-A([[:space:]]|$)' harness/ scripts/ 2>/dev/null | grep -v "review_cleanup_audit.sh" || true)
if [ -n "$cat_a_matches" ]; then
    echo "  FAIL: Found non-portable 'cat -A' usage in harness/scripts:"
    printf '%s\n' "$cat_a_matches" | sed 's/^/    /'
    check3_fail=1
fi

if grep -e "--exclude-dir" scripts/anti_bypass_audit.sh >/dev/null 2>&1; then
    echo "  FAIL: scripts/anti_bypass_audit.sh uses non-portable '--exclude-dir'"
    grep -n -e "--exclude-dir" scripts/anti_bypass_audit.sh | sed 's/^/    /'
    check3_fail=1
fi

if [ "$check3_fail" -eq 0 ]; then
    echo "  PASS: Cross-platform compatibility verified (no 'cat -A' and no '--exclude-dir')."
else
    errors=$((errors + 1))
fi

# --- Check 4: Root Directory Cleanup & Status Badge ---
echo "[Check 4/4] Auditing root .gitignore rules and README status badge (R4)..."
check4_fail=0

if ! grep -qE '^[[:space:]]*a\.out[[:space:]]*$' .gitignore 2>/dev/null; then
    echo "  FAIL: .gitignore missing explicit rule for 'a.out'"
    check4_fail=1
fi
if ! grep -qE '^[[:space:]]*(/)?\*\.s[[:space:]]*$' .gitignore 2>/dev/null; then
    echo "  FAIL: .gitignore missing rule for '*.s' or '/*.s'"
    check4_fail=1
fi

if grep -qE 'RELEASE-0\.1\.0|RELEASE--0\.1\.0' README.md 2>/dev/null; then
    echo "  FAIL: README.md badge still claims 'RELEASE-0.1.0'"
    grep -nE 'RELEASE-0\.1\.0|RELEASE--0\.1\.0' README.md | sed 's/^/    /'
    check4_fail=1
fi

if [ "$check4_fail" -eq 0 ]; then
    echo "  PASS: Root cleanup rules and README status badge verified."
else
    errors=$((errors + 1))
fi

echo "========================================="
if [ "$errors" -eq 0 ]; then
    echo " RESULT: ALL AUDIT CHECKS PASSED (0 errors)"
    echo "========================================="
    exit 0
else
    echo " RESULT: AUDIT FAILED ($errors check(s) failed)"
    echo "========================================="
    exit 1
fi
