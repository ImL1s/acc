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
DEP_MF=""
c_sources=()
s_sources=()   # .S / .s
other_inputs=()  # .o .a ...
passthru_sys=()  # flags kept for system as/ld only
ignored=()
ggcc_flags=()    # -I/-D forwarded to ggcc

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
    -MF)
      i=$((i+1))
      DEP_MF="${args[$i]:-}"
      deps=1
      i=$((i+1)); continue
      ;;
    -MF*)
      DEP_MF="${a#-MF}"
      deps=1
      i=$((i+1)); continue
      ;;
    -MT|-MQ)
      i=$((i+2)); continue
      ;;
    -MT*|-MQ*) i=$((i+1)); continue ;;
    # Kernel uses -Wp,-MMD,path (and sometimes -Wp,-MD,path). Must not hit -W*.
    -Wp,*)
      # Comma-separated preprocessor flags after -Wp,
      IFS=',' read -r -a _wp_parts <<< "${a#-Wp,}"
      _wp_i=0
      while [[ $_wp_i -lt ${#_wp_parts[@]} ]]; do
        _w="${_wp_parts[$_wp_i]}"
        case "$_w" in
          -MMD|-MD)
            # gcc: -Wp,-MMD,path.d  → depfile is the next comma field
            deps=1
            _next="${_wp_parts[$((_wp_i+1))]:-}"
            if [[ -n "$_next" && "$_next" != -* ]]; then
              DEP_MF="$_next"
              _wp_i=$((_wp_i+1))
            fi
            ;;
          -M|-MM|-MG|-MP) deps=1 ;;
          -MF)
            _wp_i=$((_wp_i+1))
            DEP_MF="${_wp_parts[$_wp_i]:-}"
            deps=1
            ;;
          -MF*)
            DEP_MF="${_w#-MF}"
            deps=1
            ;;
          -MT|-MQ) _wp_i=$((_wp_i+1)) ;;
          *) ;;
        esac
        _wp_i=$((_wp_i+1))
      done
      unset IFS _wp_parts _wp_i _w
      i=$((i+1)); continue
      ;;
    # -I/-D go to ggcc (kernel builds); also keep for probe preprocess path.
    -I)
      ggcc_flags+=("-I" "${args[$((i+1))]:-}")
      ignored+=("-I" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -I*)
      ggcc_flags+=("$a"); ignored+=("$a"); i=$((i+1)); continue
      ;;
    -D)
      ggcc_flags+=("-D" "${args[$((i+1))]:-}")
      ignored+=("-D" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -D*)
      ggcc_flags+=("$a"); ignored+=("$a"); i=$((i+1)); continue
      ;;
    # -include is required by kernel (kconfig.h / compiler-version.h). Forward to ggcc.
    -include)
      ggcc_flags+=("-include" "${args[$((i+1))]:-}")
      ignored+=("-include" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -include*)
      ggcc_flags+=("$a"); ignored+=("$a"); i=$((i+1)); continue
      ;;
    -U|-idirafter|-isystem|-iquote)
      ignored+=("$a" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -U*|-idirafter*|-isystem*|-iquote*)
      ignored+=("$a"); i=$((i+1)); continue
      ;;
    # Assembler/linker-relevant flags for system tools
    # Note: -Wa,* must NOT be swallowed by -W* below (as-version.sh uses -Wa,--version).
    -Wa,*|-Wl,*|-L*|-l*|-shared|-static|-pie|-no-pie|-nostdlib|-nostartfiles|-nodefaultlibs|-r)
      passthru_sys+=("$a"); i=$((i+1)); continue
      ;;
    -T)
      passthru_sys+=("$a" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -T*)
      passthru_sys+=("$a"); i=$((i+1)); continue
      ;;
    # Drop compiler-ish flags ggcc cannot use.
    # Note: -Wp,* handled above (before -W*). -Wa,* handled in passthru.
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
    -x)
      # language: c / assembler-with-cpp / none
      i=$((i+1))
      xlang="${args[$i]:-}"
      ignored+=("-x" "$xlang")
      i=$((i+1))
      continue
      ;;
    -)
      # stdin as input (kernel cc-version.sh: CC -E -P -x c -)
      other_inputs+=("-")
      i=$((i+1))
      continue
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
    /dev/null)
      # kernel cc-option probes: -c -x c /dev/null
      other_inputs+=("$a"); i=$((i+1)); continue
      ;;
    *)
      other_inputs+=("$a"); i=$((i+1)); continue
      ;;
  esac
done

# --- Probe-only paths (no real kernel/user .c) ---------------------------------
# Linux scripts/cc-version.sh runs: $(CC) -E -P -x c -  <<EOF  with __GNUC__ check.
# scripts/Kconfig.include cc-option runs: $(CC) -c -x c /dev/null
# These are NOT compilation of project sources; allow system cc only for probes.

is_probe_input() {
  # true if there is no real .c file among inputs
  [[ ${#c_sources[@]} -eq 0 ]]
}

# Dependency-only probes: do not invoke gcc on .c
if [[ "$deps" -eq 1 && "$mode" == "preprocess" ]]; then
  if [[ -n "$out" ]]; then
    : >"$out"
  fi
  exit 0
fi

# Preprocess-only
if [[ "$mode" == "preprocess" ]]; then
  if [[ ${#c_sources[@]} -gt 0 ]]; then
    echo "ggcc_cc_wrapper: BLOCKED -E on real .c not implemented; refusing gcc fallback for: ${c_sources[*]}" >&2
    exit 1
  fi
  # Probe stdin /dev/null / no file: system preprocessor OK (defines __GNUC__)
  # so Kconfig accepts the compiler name/version. Not used for kernel TUs.
  exec "$SYSCC" -E "${passthru_sys[@]}" "${ignored[@]}" "${s_sources[@]}" "${other_inputs[@]}"
fi

# Compile probe with no real .c and no real .S: /dev/null or as-version probes.
# as-version.sh: $(CC) -Wa,--version -c -x assembler-with-cpp /dev/null -o /dev/null
# Real .S/.s sources must NOT take this path — they need depfile write below.
if [[ "$mode" == "compile" && ${#c_sources[@]} -eq 0 && ${#s_sources[@]} -eq 0 ]]; then
  # Forward to system cc (probes only; no project sources).
  if [[ -n "$out" ]]; then
    exec "$SYSCC" -c "${passthru_sys[@]}" "${ignored[@]}" -o "$out" "${other_inputs[@]}"
  else
    exec "$SYSCC" -c "${passthru_sys[@]}" "${ignored[@]}" -o /dev/null "${other_inputs[@]}" 2>/dev/null || exit 0
  fi
fi

# Pure assembly / objects / link with no .c → system tools only
if [[ ${#c_sources[@]} -eq 0 ]]; then
  # Ensure gcc-style depfile exists when kbuild passed -Wp,-MMD (fixdep needs it).
  write_depfile_asm() {
    local srcf="${1:-}"
    [[ "$deps" -eq 1 || -n "$DEP_MF" ]] || return 0
    [[ -n "$srcf" || -n "$out" ]] || return 0
    local dfile
    if [[ -n "$DEP_MF" ]]; then
      dfile="$DEP_MF"
    elif [[ -n "$out" ]]; then
      local base dir
      base="$(basename "$out")"
      dir="$(dirname "$out")"
      dfile="$dir/.${base}.d"
    else
      dfile=".$(basename "${srcf:-x}" .S).o.d"
    fi
    mkdir -p "$(dirname "$dfile")" 2>/dev/null || true
    printf '%s: %s\n' "${out:-out.o}" "${srcf:-}" >"$dfile"
  }
  if [[ "$mode" == "compile" ]]; then
    # assemble .S/.s → .o  (need -I/-D/-include from ignored for cpp of .S)
    if [[ ${#s_sources[@]} -eq 1 && -n "$out" ]]; then
      set +e
      "$SYSCC" -c -o "$out" "${passthru_sys[@]}" "${ignored[@]}" "${s_sources[@]}" "${other_inputs[@]}"
      ec=$?
      set -e
      write_depfile_asm "${s_sources[0]}"
      exit "$ec"
    fi
    set +e
    "$SYSCC" -c "${passthru_sys[@]}" "${ignored[@]}" ${out:+-o "$out"} "${s_sources[@]}" "${other_inputs[@]}"
    ec=$?
    set -e
    if [[ ${#s_sources[@]} -ge 1 ]]; then
      write_depfile_asm "${s_sources[0]}"
    elif [[ -n "$out" ]]; then
      write_depfile_asm ""
    fi
    exit "$ec"
  fi
  if [[ "$mode" == "asm" ]]; then
    exec "$SYSCC" -S "${passthru_sys[@]}" "${ignored[@]}" ${out:+-o "$out"} "${s_sources[@]}" "${other_inputs[@]}"
  fi
  # preprocess / LDS / link — keep -I/-D so cpp_lds_S and friends work
  if [[ "$mode" == "preprocess" ]]; then
    set +e
    if [[ -n "$out" ]]; then
      "$SYSCC" -E "${passthru_sys[@]}" "${ignored[@]}" -o "$out" "${s_sources[@]}" "${other_inputs[@]}"
    else
      "$SYSCC" -E "${passthru_sys[@]}" "${ignored[@]}" "${s_sources[@]}" "${other_inputs[@]}"
    fi
    ec=$?
    set -e
    if [[ ${#s_sources[@]} -ge 1 ]]; then
      write_depfile_asm "${s_sources[0]}"
    elif [[ -n "$out" ]]; then
      write_depfile_asm ""
    fi
    exit "$ec"
  fi
  # link
  exec "$SYSCC" "${passthru_sys[@]}" "${ignored[@]}" ${out:+-o "$out"} "${s_sources[@]}" "${other_inputs[@]}"
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
"$GGCC" --target-os "$TARGET_OS" -m "$GGCC_M" "${ggcc_flags[@]}" -S -o "$asm_out" "$src" 2>"$work/ggcc.err"
ec=$?
set -e
if [[ $ec -ne 0 ]]; then
  echo "ggcc_cc_wrapper: ggcc failed on $src (ec=$ec)" >&2
  head -50 "$work/ggcc.err" >&2 || true
  # Policy: do NOT fall back to gcc/clang on the .c
  exit "$ec"
fi

# Write a minimal gcc-compatible depfile when -MD/-MMD was requested.
write_depfile() {
  local srcf="$1"
  [[ "$deps" -eq 1 || -n "$DEP_MF" ]] || return 0
  local dfile
  if [[ -n "$DEP_MF" ]]; then
    dfile="$DEP_MF"
  elif [[ -n "$out" ]]; then
    local base dir
    base="$(basename "$out")"
    dir="$(dirname "$out")"
    dfile="$dir/.${base}.d"
  else
    dfile=".$(basename "$srcf" .c).o.d"
  fi
  mkdir -p "$(dirname "$dfile")" 2>/dev/null || true
  printf '%s: %s\n' "${out:-$(basename "$srcf" .c).o}" "$srcf" >"$dfile"
}

case "$mode" in
  asm)
    if [[ -n "$out" ]]; then
      cp "$asm_out" "$out"
    else
      cp "$asm_out" "$(basename "$src" .c).s"
    fi
    write_depfile "$src"
    exit 0
    ;;
  compile)
    # system assembler only — never recompile .c
    if [[ -n "$out" ]]; then
      "$SYSCC" -c -o "$out" "$asm_out"
    else
      "$SYSCC" -c -o "$(basename "$src" .c).o" "$asm_out"
    fi
    write_depfile "$src"
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
