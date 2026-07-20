#!/usr/bin/env bash
# ggcc_cc_wrapper.sh — pretend to be $CC for kernel / make, without feeding .c to gcc.
#
# Policy (clean-room Stage C1):
#   - User/kernel .c  → only ggcc (frontend → asm)
#   - .S / .s / .o / link → system as/ld/cc (assemble & link only)
#   - NEVER pass a .c source to system gcc/clang as the C compiler
#
# ggcc currently understands a tiny flag set (-o -S -m --target-os). Kernel make
# passes many gcc flags; we strip unknown flags before invoking ggcc.
# HOSTCC for kconfig/fixdep may still be system gcc — that is intentional and
# does not compile kernel .c.
set -euo pipefail

GGCC="${GGCC:?GGCC must point at a Linux-runnable ggcc binary}"
SYSCC="${SYSCC:-cc}"   # assemble / link only
SYSAS="${SYSAS:-as}"
TARGET_OS="${GGCC_TARGET_OS:-linux}"
ARCH="${GGCC_ARCH:-x86_64}"
# Map kernel ARCH → ggcc -m
case "$ARCH" in
  x86_64|x86|i386) GGCC_M=x86_64 ;;
  arm64|aarch64)   GGCC_M=aarch64 ;;
  *)               GGCC_M=x86_64 ;;
esac

# --- parse make/gcc-style argv ---
out=""
mode=link   # link | compile (-c) | asm (-S) | preprocess (-E)
deps=0
c_sources=()
s_sources=()   # .S / .s
other_inputs=()  # .o .a ...
passthru_sys=()  # flags kept for system as/ld only
ignored=()

i=0
args=("$@")
while [[ $i -lt $# ]]; do
  a="${args[$i]}"
  case "$a" in
    -c) mode=compile; i=$((i+1)); continue ;;
    -S) mode=asm; i=$((i+1)); continue ;;
    -E) mode=preprocess; i=$((i+1)); continue ;;
    -o)
      i=$((i+1))
      out="${args[$i]:-}"
      i=$((i+1))
      continue
      ;;
    -o*)
      out="${a#-o}"
      i=$((i+1))
      continue
      ;;
    -M|-MM|-MD|-MMD|-MG|-MP) deps=1; i=$((i+1)); continue ;;
    -MF|-MT|-MQ)
      # drop arg
      i=$((i+2)); continue
      ;;
    -MF*|-MT*|-MQ*) i=$((i+1)); continue ;;
    # Keep -I/-D/-U/-include for future ggcc use in env (not yet supported by CLI).
    # Do not forward to system cc when compiling .c.
    -I|-D|-U|-include|-idirafter|-isystem|-iquote)
      ignored+=("$a" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -I*|-D*|-U*|-include*|-idirafter*|-isystem*|-iquote*)
      ignored+=("$a"); i=$((i+1)); continue
      ;;
    # Assembler/linker-relevant flags for system tools
    -Wl,*|-L*|-l*|-shared|-static|-pie|-no-pie|-nostdlib|-nostartfiles|-nodefaultlibs|-r|-Wl)
      passthru_sys+=("$a"); i=$((i+1)); continue
      ;;
    -T)
      passthru_sys+=("$a" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -T*)
      passthru_sys+=("$a"); i=$((i+1)); continue
      ;;
    # Drop compiler-ish flags ggcc cannot use
    -W*|-f*|-m*|-O*|-g*|-std=*|-pedantic*|--param*|-pipe|-pthread|-P|-C|-dM|-dD)
      ignored+=("$a"); i=$((i+1)); continue
      ;;
    --version|-v|-V)
      echo "ggcc-wrapper (CC for Stage C1; C via ggcc, as/ld system)"
      exit 0
      ;;
    -dumpmachine)
      case "$GGCC_M" in
        x86_64) echo "x86_64-linux-gnu" ;;
        aarch64) echo "aarch64-linux-gnu" ;;
      esac
      exit 0
      ;;
    -dumpversion)
      echo "0.1.0"
      exit 0
      ;;
    --help)
      echo "ggcc_cc_wrapper: CC wrapper; .c→ggcc, as/ld system"
      exit 0
      ;;
    -print-*)
      # kernel/tool probes; empty path, never compile .c with gcc
      echo
      exit 0
      ;;
    -*)
      # unknown flag: keep for system link/asm path only
      passthru_sys+=("$a"); i=$((i+1)); continue
      ;;
    *.c)
      c_sources+=("$a"); i=$((i+1)); continue
      ;;
    *.C|*.cc|*.cpp|*.cxx)
      echo "ggcc_cc_wrapper: C++ not supported: $a" >&2
      exit 1
      ;;
    *.S|*.s)
      s_sources+=("$a"); i=$((i+1)); continue
      ;;
    *)
      other_inputs+=("$a"); i=$((i+1)); continue
      ;;
  esac
done

# Dependency-only probes: do not invoke gcc on .c
if [[ "$deps" -eq 1 && "$mode" == "preprocess" ]]; then
  # Emit a trivial dep line so make does not treat as hard fail when probing
  if [[ -n "$out" ]]; then
    : >"$out"
  fi
  exit 0
fi

# Preprocess-only: ggcc has no -E; fail honestly (do not fall back to gcc on .c)
if [[ "$mode" == "preprocess" ]]; then
  if [[ ${#c_sources[@]} -gt 0 ]]; then
    echo "ggcc_cc_wrapper: BLOCKED -E/preprocess not implemented in ggcc; refusing gcc fallback for: ${c_sources[*]}" >&2
    exit 1
  fi
  # Non-C preprocess (rare): system ok
  exec "$SYSCC" -E "${passthru_sys[@]}" "${s_sources[@]}" "${other_inputs[@]}"
fi

# Pure assembly / objects / link with no .c → system tools only
if [[ ${#c_sources[@]} -eq 0 ]]; then
  if [[ "$mode" == "compile" ]]; then
    # assemble .S/.s → .o
    if [[ ${#s_sources[@]} -eq 1 && -n "$out" ]]; then
      exec "$SYSCC" -c -o "$out" "${passthru_sys[@]}" "${s_sources[@]}" "${other_inputs[@]}"
    fi
    exec "$SYSCC" -c "${passthru_sys[@]}" ${out:+-o "$out"} "${s_sources[@]}" "${other_inputs[@]}"
  fi
  if [[ "$mode" == "asm" ]]; then
    exec "$SYSCC" -S "${passthru_sys[@]}" ${out:+-o "$out"} "${s_sources[@]}" "${other_inputs[@]}"
  fi
  # link
  exec "$SYSCC" "${passthru_sys[@]}" ${out:+-o "$out"} "${s_sources[@]}" "${other_inputs[@]}"
fi

# --- .c present: ggcc only for C, then system as for objects ---
if [[ ${#c_sources[@]} -gt 1 ]]; then
  echo "ggcc_cc_wrapper: multiple .c in one invocation not supported yet: ${c_sources[*]}" >&2
  exit 1
fi
src="${c_sources[0]}"

tmpdir="${TMPDIR:-/tmp}"
work="$(mktemp -d "$tmpdir/ggcc-wrap.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

asm_out="$work/out.s"
obj_out="$work/out.o"

set +e
"$GGCC" --target-os "$TARGET_OS" -m "$GGCC_M" -S -o "$asm_out" "$src" 2>"$work/ggcc.err"
ec=$?
set -e
if [[ $ec -ne 0 ]]; then
  echo "ggcc_cc_wrapper: ggcc failed on $src (ec=$ec)" >&2
  head -50 "$work/ggcc.err" >&2 || true
  # Policy: do NOT fall back to gcc/clang on the .c
  exit "$ec"
fi

case "$mode" in
  asm)
    if [[ -n "$out" ]]; then
      cp "$asm_out" "$out"
    else
      cp "$asm_out" "$(basename "$src" .c).s"
    fi
    exit 0
    ;;
  compile)
    # system assembler only — never recompile .c
    if [[ -n "$out" ]]; then
      "$SYSCC" -c -o "$out" "$asm_out"
    else
      "$SYSCC" -c -o "$(basename "$src" .c).o" "$asm_out"
    fi
    exit 0
    ;;
  link)
    # compile C → asm → obj, then link with any other inputs
    "$SYSCC" -c -o "$obj_out" "$asm_out"
    if [[ -n "$out" ]]; then
      "$SYSCC" -o "$out" "$obj_out" "${passthru_sys[@]}" "${s_sources[@]}" "${other_inputs[@]}"
    else
      "$SYSCC" -o a.out "$obj_out" "${passthru_sys[@]}" "${s_sources[@]}" "${other_inputs[@]}"
    fi
    exit 0
    ;;
  *)
    echo "ggcc_cc_wrapper: unknown mode $mode" >&2
    exit 2
    ;;
esac
