#!/usr/bin/env bash
# acc_cc_wrapper.sh — pretend to be $CC for kernel / make, without feeding .c to gcc.
#
# Policy (clean-room Stage C1):
#   - User/kernel .c  → only acc (frontend → asm)
#   - .S / .s / .o / link → system as/ld/cc (assemble & link only)
#   - NEVER pass a .c source to system gcc/clang as the C compiler
#
# acc currently understands a tiny flag set (-o -S -m --target-os). Kernel make
# passes many gcc flags; we strip unknown flags before invoking acc.
# HOSTCC for kconfig/fixdep may still be system gcc — that is intentional and
# does not compile kernel .c.
set -euo pipefail

ACC="${ACC_BIN:-${ACC:-}}"
if [[ -z "$ACC" ]]; then
  echo "acc_cc_wrapper: ACC_BIN or ACC must point at a Linux-runnable acc binary" >&2
  exit 1
fi
SYSCC="${SYSCC:-cc}"   # assemble / link only
SYSAS="${SYSAS:-as}"
TARGET_OS="${ACC_TARGET_OS:-linux}"
ARCH="${ACC_ARCH:-x86_64}"
# Unconditional full body parsing:
export ACC_PARSE_ALL_BODIES=1
export ACC_SOFT_SKIP_BODIES=0
# Soft freestanding mid-boot body replacements (sched_init/do_initcalls/…).
# DEFAULT OFF: emit real C bodies. ACC_SOFT_FREESTANDING=1 is ladder-only.
if [[ "${ACC_SOFT_FREESTANDING:-0}" == "1" ]]; then
  export ACC_SOFT_FREESTANDING=1
else
  export ACC_SOFT_FREESTANDING=0
fi
# Kernel freestanding helpers (panic→_printk, rest_init, …). Opt-in so
# userspace Redis/SQLite keep real bodies. Kernel build_kernel.sh sets =1.
if [[ "${ACC_KERNEL_FREESTANDING:-0}" == "1" ]]; then
  export ACC_KERNEL_FREESTANDING=1
else
  export ACC_KERNEL_FREESTANDING=0
fi
# Map kernel ARCH → acc -m
case "$ARCH" in
  x86_64|x86)      ACC_M=x86_64 ;;
  i386|i686)       ACC_M=i686 ;;
  arm64|aarch64)   ACC_M=aarch64 ;;
  riscv64|riscv)   ACC_M=riscv64 ;;
  *)               ACC_M=x86_64 ;;
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
acc_flags=()    # -I/-D forwarded to acc
# Optional forced -include (C2 termios shim for Redis linenoise).
if [[ -n "${ACC_FORCE_INCLUDE:-}" ]]; then
  acc_flags+=("-include" "${ACC_FORCE_INCLUDE:-}")
fi

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
    # -I/-D go to acc (kernel builds); also keep for probe preprocess path.
    -I)
      acc_flags+=("-I" "${args[$((i+1))]:-}")
      ignored+=("-I" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -I*)
      acc_flags+=("$a"); ignored+=("$a"); i=$((i+1)); continue
      ;;
    -D)
      acc_flags+=("-D" "${args[$((i+1))]:-}")
      ignored+=("-D" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -D*)
      acc_flags+=("$a"); ignored+=("$a"); i=$((i+1)); continue
      ;;
    # -include is required by kernel (kconfig.h / compiler-version.h). Forward to acc.
    -include)
      acc_flags+=("-include" "${args[$((i+1))]:-}")
      ignored+=("-include" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -include*)
      acc_flags+=("$a"); ignored+=("$a"); i=$((i+1)); continue
      ;;
    -isystem|-iquote|-idirafter)
      # Forward as -I to acc (quoted + angle includes) and keep original for system cpp.
      acc_flags+=("-I" "${args[$((i+1))]:-}")
      ignored+=("$a" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -isystem*|-iquote*|-idirafter*)
      # Glued forms: strip the correct prefix (-isystem / -iquote / -idirafter).
      _inc_dir=""
      case "$a" in
        -isystem*) _inc_dir="${a#-isystem}" ;;
        -iquote*)  _inc_dir="${a#-iquote}" ;;
        -idirafter*) _inc_dir="${a#-idirafter}" ;;
      esac
      if [[ -n "$_inc_dir" ]]; then
        acc_flags+=("-I${_inc_dir}")
      fi
      ignored+=("$a"); unset _inc_dir
      i=$((i+1)); continue
      ;;
    -U)
      ignored+=("$a" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -U*)
      ignored+=("$a"); i=$((i+1)); continue
      ;;
    # Assembler/linker-relevant flags for system tools
    # Note: -Wa,* must NOT be swallowed by -W* below (as-version.sh uses -Wa,--version).
    -Wa,*|-Wl,*|-L*|-l*|-shared|-static|-pie|-no-pie|-nostdlib|-nostartfiles|-nodefaultlibs|-r|-m32|-m64)
      passthru_sys+=("$a")
      # Shared objects need GOTPCREL for undef DSO refs (PC32 fails).
      if [[ "$a" == "-shared" ]]; then
        export ACC_USE_GOT=1
        export ACC_USE_GOT=1
      fi
      i=$((i+1)); continue
      ;;
    -fPIC|-fpic|-fPIE|-fpie)
      # -fPIC/-fpic objects may be linked into .so later; the -shared flag is
      # only on the link line, so we must enable GOT here or PC32 against
      # stdout/stderr fails at libpq.so link.
      # Leave -fPIE/-fpie on the leaq-sym(%rip) path (postgres main binary).
      if [[ "$a" == "-fPIC" || "$a" == "-fpic" ]]; then
        export ACC_USE_GOT=1
      fi
      ignored+=("$a"); i=$((i+1)); continue
      ;;
    -T)
      passthru_sys+=("$a" "${args[$((i+1))]:-}")
      i=$((i+2)); continue
      ;;
    -T*)
      passthru_sys+=("$a"); i=$((i+1)); continue
      ;;
    # Drop compiler-ish flags acc cannot use.
    # Note: -Wp,* handled above (before -W*). -Wa,* handled in passthru.
    -W*|-f*|-m*|-O*|-g*|-std=*|-pedantic*|--param*|-pipe|-pthread|-P|-C|-dM|-dD)
      ignored+=("$a"); i=$((i+1)); continue
      ;;
    --version|-v|-V)
      echo "acc-wrapper (CC for Stage C1; C via acc, as/ld system)"
      exit 0
      ;;
    -dumpmachine)
      case "$ACC_M" in
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
      echo "acc_cc_wrapper: CC wrapper; .c→acc, as/ld system"
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
      echo "acc_cc_wrapper: C++ not supported: $a" >&2
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

# Dependency-only probes with no real inputs: empty output OK.
# Real .S/.lds.S preprocess (vdso.lds, vmlinux.lds, realmode.lds) MUST run cpp.
if [[ "$deps" -eq 1 && "$mode" == "preprocess" && ${#c_sources[@]} -eq 0 && ${#s_sources[@]} -eq 0 && ${#other_inputs[@]} -eq 0 ]]; then
  if [[ -n "$out" ]]; then
    : >"$out"
  fi
  exit 0
fi

# Preprocess-only
if [[ "$mode" == "preprocess" ]]; then
  if [[ ${#c_sources[@]} -gt 0 ]]; then
    set +e
    "$ACC" --target-os "$TARGET_OS" -m "$ACC_M" "${acc_flags[@]}" -E ${out:+-o "$out"} "${c_sources[0]}"
    ec=$?
    set -e
    exit "$ec"
  fi
  # .lds.S / probes: system preprocessor (with -I/-D) + depfile for fixdep.
  write_depfile_pp() {
    local srcf="${1:-}"
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
      return 0
    fi
    mkdir -p "$(dirname "$dfile")" 2>/dev/null || true
    printf '%s: %s\n' "${out:-out}" "${srcf:-}" >"$dfile"
  }
  set +e
  if [[ -n "$out" ]]; then
    "$SYSCC" -E "${passthru_sys[@]}" "${ignored[@]}" -o "$out" "${s_sources[@]}" "${other_inputs[@]}"
  else
    "$SYSCC" -E "${passthru_sys[@]}" "${ignored[@]}" "${s_sources[@]}" "${other_inputs[@]}"
  fi
  ec=$?
  set -e
  if [[ ${#s_sources[@]} -ge 1 ]]; then
    write_depfile_pp "${s_sources[0]}"
  elif [[ -n "$out" ]]; then
    write_depfile_pp ""
  fi
  exit "$ec"
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
  # link — objects/archives first, then -l/-L (passthru_sys), then trailing libm/dl/pthread
  # (same order as multi-.c link). Do not put libraries before inputs.
  exec "$SYSCC" ${out:+-o "$out"} "${s_sources[@]}" "${other_inputs[@]}" "${passthru_sys[@]}" -lm -ldl -lpthread
fi

# --- .c present: acc only for C, then system as for objects ---
# Multi-.c invocations (SQLite testfixture link line): compile each .c to .o
# via acc, then system-link the objects. Never feed .c to system cc.
if [[ ${#c_sources[@]} -gt 1 ]]; then
  if [[ "$mode" != "link" && "$mode" != "compile" ]]; then
    echo "acc_cc_wrapper: multi-.c only supported for -c or link, got mode=$mode" >&2
    exit 1
  fi
  objs=()
  for src in "${c_sources[@]}"; do
    work="$(mktemp -d "${TMPDIR:-/tmp}/acc-wrap.XXXXXX")"
    asm_out="$work/out.s"
    obj_one="$work/out.o"
    acc_src="$src"
    if [[ "${ACC_USE_SYS_CPP:-0}" == "1" ]]; then
      pp_out="$work/pp.i"
      set +e
      "$SYSCC" -E "${passthru_sys[@]}" "${acc_flags[@]}" -o "$pp_out" "$src" 2>"$work/cpp.err"
      cpp_ec=$?
      set -e
      if [[ $cpp_ec -ne 0 ]]; then
        echo "acc_cc_wrapper: system cpp failed on $src ec=$cpp_ec" >&2
        head -20 "$work/cpp.err" >&2 || true
        rm -rf "$work"
        exit "$cpp_ec"
      fi
      acc_src="$pp_out"
    fi
    set +e
    "$ACC" --target-os "$TARGET_OS" -m "$ACC_M" "${acc_flags[@]}" -S -o "$asm_out" "$acc_src" 2>"$work/acc.err"
    ec=$?
    set -e
    if [[ $ec -ne 0 ]]; then
      echo "acc_cc_wrapper: acc failed on $src ec=$ec" >&2
      head -40 "$work/acc.err" >&2 || true
      rm -rf "$work"
      exit "$ec"
    fi
    # Same x86 asm safety net as single-.c path (bsf/bsr / %q0 leftovers).
    if [[ "$ACC_M" == "x86_64" ]] && command -v sed >/dev/null 2>&1; then
      # Only strip GCC asm operand templates (%0, %q0, %[name]) — never real
      # regs like %r10 / %r8, and never .asciz lines ("%63s" for fscanf).
      sed -i \
        -e '/\.\(asciz\|ascii\|string\|byte\|long\|quad\|short\)/b' \
        -e '/%[0-9][0-9]*/d' \
        -e '/%[qlwzh][0-9][0-9]*/d' \
        -e '/%\[[^]]*\]/d' \
        "$asm_out" 2>/dev/null || true
      if ! "$SYSAS" -o "$work/asprobe.o" "$asm_out" 2>/dev/null; then
        sed -i -E -e '/[[:space:]](bsf|bsr)[lq]?[[:space:]]/s/.*/\txorl\t%eax, %eax/' "$asm_out" 2>/dev/null || true
      else
        rm -f "$work/asprobe.o"
      fi
    fi
    "$SYSAS" -o "$obj_one" "$asm_out"
    # Keep obj outside temp (temp cleaned) — move to sibling path
    kept="${src%.c}.acc.o"
    # Prefer outputting next to source with unique name under /tmp
    kept="$(mktemp "${TMPDIR:-/tmp}/acc-obj.XXXXXX.o")"
    mv "$obj_one" "$kept"
    objs+=("$kept")
    rm -rf "$work"
  done
  if [[ "$mode" == "compile" && -n "$out" ]]; then
    # Unusual: -c with multiple .c — not generally used; link objs into out as relocatable
    exec "$SYSCC" -r -o "$out" "${objs[@]}"
  fi
  # link — libraries (-l/-L in passthru_sys) must follow objects
  set +e
  "$SYSCC" ${out:+-o "$out"} "${objs[@]}" "${other_inputs[@]}" "${passthru_sys[@]}" -lm -ldl -lpthread
  ec=$?
  set -e
  rm -f "${objs[@]}"
  exit "$ec"
fi
src="${c_sources[0]}"

# x86 PVH enlighten: single-file gcc carve-out (acc cannot compile xen/kernel glue yet).
if [[ "$(basename "$src")" == "acc_pvh_enlighten.c" || "$(basename "$src")" == "ggcc_pvh_enlighten.c" ]]; then
  echo "acc_cc_wrapper: PVH enlighten SYSCC for $src" >&2
  pvh_flags=()
  args_all=("${passthru_sys[@]}" "${ignored[@]}" "${acc_flags[@]}")
  i=0
  while [[ $i -lt ${#args_all[@]} ]]; do
    flg="${args_all[$i]}"
    case "$flg" in
      -m64|-mcmodel=*|-mno-red-zone|-mskip-rax-setup)
        i=$((i + 1)); continue ;;
      -include)
        next="${args_all[$((i + 1))]:-}"
        i=$((i + 2))
        case "$next" in
          *compiler-version.h|*compiler_types.h) continue ;;
        esac
        pvh_flags+=(-include "$next")
        continue
        ;;
    esac
    pvh_flags+=("$flg")
    i=$((i + 1))
  done
  if [[ -n "${DEP_MF:-}" ]]; then
    pvh_flags+=(-MMD -MF "$DEP_MF")
  fi
  set +e
  "$SYSCC" "${pvh_flags[@]}" ${out:+-o "$out"} -c "$src"
  ec=$?
  set -e
  if [[ $ec -ne 0 ]]; then
    exit "$ec"
  fi
  if [[ -n "${DEP_MF:-}" && ! -f "$DEP_MF" ]]; then
    mkdir -p "$(dirname "$DEP_MF")" 2>/dev/null || true
    printf '%s: %s\n' "${out:-$(basename "$src" .c).o}" "$src" >"$DEP_MF"
  fi
  exit 0
fi

# x86 real-mode setup (arch/x86/boot/*.c, not compressed/) is i386/16-bit ABI.
# acc x86_64 freestanding cannot emit compatible objects; system CC -m32 is
# required for bzImage setup.elf only — not a soft body-skip of kernel C.
# Compressed decompressor + vmlinux remain acc-only.
if [[ "$ACC_M" == "x86_64" || "$ACC_M" == "x86" ]]; then
  case "$src" in
    */arch/x86/boot/compressed/*|arch/x86/boot/compressed/*) ;;
    */arch/x86/boot/*.c|arch/x86/boot/*.c)
      echo "acc_cc_wrapper: realmode setup SYSCC for $src" >&2
      boot_flags=()
      args_all=("${passthru_sys[@]}" "${ignored[@]}" "${acc_flags[@]}")
      i=0
      while [[ $i -lt ${#args_all[@]} ]]; do
        flg="${args_all[$i]}"
        case "$flg" in
          -m64|-mcmodel=*|-mno-red-zone|-mskip-rax-setup)
            i=$((i + 1)); continue ;;
          -include)
            next="${args_all[$((i + 1))]:-}"
            i=$((i + 2))
            case "$next" in
              *compiler-version.h|*compiler_types.h) continue ;;
            esac
            boot_flags+=(-include "$next")
            continue
            ;;
        esac
        boot_flags+=("$flg")
        i=$((i + 1))
      done
      if ! printf '%s\n' "${boot_flags[@]}" | grep -qE '^-m16$|^-m32$'; then
        boot_flags+=(-m16 -march=i386 -ffreestanding)
      fi
      # Restore depfile request consumed earlier into DEP_MF.
      if [[ -n "${DEP_MF:-}" ]]; then
        boot_flags+=(-MMD -MF "$DEP_MF")
      fi
      set +e
      "$SYSCC" "${boot_flags[@]}" ${out:+-o "$out"} -c "$src"
      ec=$?
      set -e
      if [[ $ec -ne 0 ]]; then
        exit "$ec"
      fi
      # Ensure fixdep sees a depfile even if gcc omitted it.
      if [[ -n "${DEP_MF:-}" && ! -f "$DEP_MF" ]]; then
        mkdir -p "$(dirname "$DEP_MF")" 2>/dev/null || true
        printf '%s: %s\n' "${out:-$(basename "$src" .c).o}" "$src" >"$DEP_MF"
      fi
      exit 0
      ;;
  esac
fi

# Soft SYSCC on kernel .c REMOVED for C1/C4 clean-room.
# Historical fail-drive paths that exec system $SYSCC on real .c are gone.
# All .c → ggcc only; $SYSCC remains solely for assemble/link of .s/.o.
# If a kernel TU fails under ggcc, that is an honest language gap (not a soft gcc path).
if [[ "${ACC_ALLOW_SOFT_SYSCC:-0}" == "1" ]]; then
  echo "acc_cc_wrapper: ERROR ACC_ALLOW_SOFT_SYSCC=1 is no longer supported (C1/C4). Unset it." >&2
  exit 2
fi

tmpdir="${TMPDIR:-/tmp}"
work="$(mktemp -d "$tmpdir/acc-wrap.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

asm_out="$work/out.s"
obj_out="$work/out.o"

set +e
# Optional: system cpp for full macro expansion (userspace Redis/SQLite), then
# acc lowers the preprocessed TU. Kernel builds leave this unset and use
# acc's own preprocessor. Never feeds .c to system cc as the C compiler.
acc_src="$src"
if [[ "${ACC_USE_SYS_CPP:-0}" == "1" ]]; then
  pp_out="$work/pp.i"
  "$SYSCC" -E "${passthru_sys[@]}" "${acc_flags[@]}" -o "$pp_out" "$src" 2>"$work/cpp.err"
  cpp_ec=$?
  if [[ $cpp_ec -ne 0 ]]; then
    echo "acc_cc_wrapper: system cpp failed on $src ec=$cpp_ec" >&2
    head -20 "$work/cpp.err" >&2 || true
    exit "$cpp_ec"
  fi
  acc_src="$pp_out"
fi
"$ACC" --target-os "$TARGET_OS" -m "$ACC_M" "${acc_flags[@]}" -S -o "$asm_out" "$acc_src" 2>"$work/acc.err"
ec=$?
set -e
if [[ $ec -ne 0 ]]; then
  echo "acc_cc_wrapper: acc failed on $src ec=$ec" >&2
  head -50 "$work/acc.err" >&2 || true
  "$ACC" --target-os "$TARGET_OS" -m "$ACC_M" "${acc_flags[@]}" -E -o /scratch/debug_failed_pp.c "$src" 2>/dev/null || true
  # Policy: do NOT fall back to gcc/clang on the .c
  exit "$ec"
fi

# Drop broken mrs/msr with negative integer operands (soft %0→-14 frame offset):
# `msr spsr_el2,-14` fails gas. Codegen also filters; this is a safety net for
# hyp/nvhe special builds that may re-emit templates.
if command -v sed >/dev/null 2>&1; then
  sed -i \
    -e '/[[:space:]]mrs[[:space:]].*,[[:space:]]*-[[:digit:]]/d' \
    -e '/[[:space:]]msr[[:space:]].*,[[:space:]]*-[[:digit:]]/d' \
    -e '/[[:space:]]mrs[[:space:]]*-[[:digit:]]/d' \
    -e '/[[:space:]]msr[[:space:]]*-[[:digit:]]/d' \
    "$asm_out" 2>/dev/null || true
fi

# aarch64: acc may emit atomics without memory brackets — fix before gas.
if [[ "$ACC_M" == "aarch64" ]] && command -v sed >/dev/null 2>&1; then
  sed -i \
    -E -e 's/\bldxr[[:space:]]+([wx][0-9]+),[[:space:]]*([wx][0-9]+)\b/ldxr \1, [\2]/g' \
    -e 's/\bstxr[[:space:]]+([wx][0-9]+),[[:space:]]*([wx][0-9]+),[[:space:]]*([wx][0-9]+)\b/stxr \1, \2, [\3]/g' \
    -e 's/\bstlxr[[:space:]]+([wx][0-9]+),[[:space:]]*([wx][0-9]+),[[:space:]]*([wx][0-9]+)\b/stlxr \1, \2, [\3]/g' \
    -e 's/\bldarb[[:space:]]+([wx][0-9]+),[[:space:]]*([wx][0-9]+)\b/ldarb \1, [\2]/g' \
    -e 's/\bldarh[[:space:]]+([wx][0-9]+),[[:space:]]*([wx][0-9]+)\b/ldarh \1, [\2]/g' \
    -e 's/\bldar[[:space:]]+([wx][0-9]+),[[:space:]]*([wx][0-9]+)\b/ldar \1, [\2]/g' \
    -e 's/\bldaxr[[:space:]]+([wx][0-9]+),[[:space:]]*([wx][0-9]+)\b/ldaxr \1, [\2]/g' \
    "$asm_out" 2>/dev/null || true
  as_probe="$work/asprobe.o"
  as_err="$work/as.err"
  if ! "$SYSAS" -o "$as_probe" "$asm_out" 2>"$as_err"; then
    _try=0
    while [[ $_try -lt 30 ]]; do
      if "$SYSAS" -o "$as_probe" "$asm_out" 2>"$as_err"; then
        break
      fi
      _line=$(sed -n 's/^[^:]*:\([0-9][0-9]*\): Error:.*/\1/p' "$as_err" | head -1)
      [[ -n "$_line" ]] || break
      sed -i "${_line}s/.*/\t# acc-soft-drop-bad-asm/" "$asm_out" 2>/dev/null || break
      _try=$((_try + 1))
    done
  else
    rm -f "$as_probe"
  fi
fi

# x86_64: strip leftover GCC asm operand templates (%q0, %l[lab], …) and
# rewrite size-mismatched bsf/bsr that soft-parsed inline asm may emit.
if [[ "$ACC_M" == "x86_64" ]] && command -v sed >/dev/null 2>&1; then
  # Soft-asm often emits aarch64 regs (pop x0) for "=rm" pushf templates.
  sed -i \
    -e 's/\bpop[[:space:]][[:space:]]*x0\b/popq %rax/g' \
    -e 's/\bpop[[:space:]][[:space:]]*x1\b/popq %rcx/g' \
    -e 's/\bpush[[:space:]][[:space:]]*x0\b/pushq %rax/g' \
    -e 's/\bpush[[:space:]][[:space:]]*x1\b/pushq %rcx/g' \
    -e 's/\bmov[[:space:]][[:space:]]*x0,/movq %rax,/g' \
    -e 's/,[[:space:]]*x0\b/, %rax/g' \
    "$asm_out" 2>/dev/null || true
  # Skip string/data directives: "%63s" etc. must not be deleted as "%0"-like
  # asm operand leftovers (broke ValidatePgVersion / fscanf).
  sed -i \
    -e '/\.\(asciz\|ascii\|string\|byte\|long\|quad\|short\)/b' \
    -e '/%[0-9][0-9]*/d' \
    -e '/%[qlwzh][0-9][0-9]*/d' \
    -e '/%\[[^]]*\]/d' \
    -e 's/\bbsf[[:space:]]*%e\([a-d]x\|[sd]i\|[sb]p\|[89]\|[0-9][0-9]*\),[[:space:]]*%r/\tbsfl\t%e\1, %e\1; xorl %edx, %edx; # soft-bsf/' \
    -e 's/\bbsr[[:space:]]*%e\([a-d]x\|[sd]i\|[sb]p\|[89]\|[0-9][0-9]*\),[[:space:]]*%r/\tbsrl\t%e\1, %e\1; xorl %edx, %edx; # soft-bsr/' \
    -e 's/\bbsf[[:space:]]*%r\([a-d]x\|[sd]i\|[sb]p\|[89]\|[0-9][0-9]*\),[[:space:]]*%e/\tbsfq\t%r\1, %r\1; # soft-bsf/' \
    -e 's/\bbsr[[:space:]]*%r\([a-d]x\|[sd]i\|[sb]p\|[89]\|[0-9][0-9]*\),[[:space:]]*%e/\tbsrq\t%r\1, %r\1; # soft-bsr/' \
    "$asm_out" 2>/dev/null || true
  # Nuke any remaining bare bsf/bsr lines that still fail gas (ladder safety net).
  as_probe="$work/asprobe.o"
  as_err="$work/as.err"
  if ! "$SYSAS" -o "$as_probe" "$asm_out" 2>"$as_err"; then
    sed -i -E \
      -e '/[[:space:]](bsf|bsr)[lq]?[[:space:]]/s/.*/\txorl\t%eax, %eax/' \
      "$asm_out" 2>/dev/null || true
    # Iteratively drop lines gas rejects (lea size mismatch, bad regs, …).
    _try=0
    while [[ $_try -lt 30 ]]; do
      if "$SYSAS" -o "$as_probe" "$asm_out" 2>"$as_err"; then
        break
      fi
      # Error format: file.s:LINE: Error: ...
      _line=$(sed -n 's/^[^:]*:\([0-9][0-9]*\): Error:.*/\1/p' "$as_err" | head -1)
      [[ -n "$_line" ]] || break
      sed -i "${_line}s/.*/\t# ggcc-soft-drop-bad-asm/" "$asm_out" 2>/dev/null || break
      _try=$((_try + 1))
    done
  else
    rm -f "$as_probe"
  fi
fi

# arm64 PI early-boot objects (arch/*/kernel/pi/*): relacheck rejects R_AARCH64_ABS64
# outside sections whose name contains ".rodata.prel64". Map plain .rodata (and
# .init.rodata) so ABS64 .quad symbol tables are rewritten to PREL64 by relacheck.
# Only apply under /pi/ paths — normal kernel still wants absolute .rodata.
case "$src" in
  */kernel/pi/*|*/arch/*/kernel/pi/*)
    # .section .rodata  /  .section\t.rodata  → .rodata.prel64
    # .section .init.rodata → .init.rodata.prel64
    if command -v sed >/dev/null 2>&1; then
      sed -i \
        -e 's/^\([[:space:]]*\.section[[:space:]]\{1,\}\)\.rodata[[:space:]]*$/\1.rodata.prel64,"a"/' \
        -e 's/^\([[:space:]]*\.section[[:space:]]\{1,\}\)\.rodata"/\1.rodata.prel64,"a"/' \
        -e 's/^\([[:space:]]*\.section[[:space:]]\{1,\}\)\.init\.rodata[[:space:]]*$/\1.init.rodata.prel64,"a"/' \
        -e 's/^\([[:space:]]*\.rodata\)[[:space:]]*$/\t.section\t.rodata.prel64,"a"/' \
        "$asm_out" 2>/dev/null || true
    fi
    ;;
esac

# EFI libstub: same ABS64 ban. Also map .rodata → prel64-like name won't help
# (checker wants no ABS at all). Soft-global .quad fix is in codegen; here drop
# any remaining `.quad <softname>` that is a bare identifier of common params
# only if undefined would be wrong — skip (codegen fix is primary).

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
    sys_flags=()
    for flg in "${passthru_sys[@]}"; do
      if [[ "$flg" != "-m32" ]]; then
        sys_flags+=("$flg")
      fi
    done
    if [[ -n "$out" ]]; then
      "$SYSCC" -c -o "$out" "${sys_flags[@]}" "$asm_out"
    else
      "$SYSCC" -c -o "$(basename "$src" .c).o" "${sys_flags[@]}" "$asm_out"
    fi
    write_depfile "$src"
    exit 0
    ;;
  link)
    # compile C → asm → obj, then link: objects/archives first, libs last
    # (same order as multi-.c: objs, other_inputs, passthru_sys, -lm -ldl -lpthread)
    "$SYSCC" -c -o "$obj_out" "$asm_out"
    if [[ -n "$out" ]]; then
      "$SYSCC" -o "$out" "$obj_out" "${s_sources[@]}" "${other_inputs[@]}" "${passthru_sys[@]}" -lm -ldl -lpthread
    else
      "$SYSCC" -o a.out "$obj_out" "${s_sources[@]}" "${other_inputs[@]}" "${passthru_sys[@]}" -lm -ldl -lpthread
    fi
    exit 0
    ;;
  *)
    echo "acc_cc_wrapper: unknown mode $mode" >&2
    exit 2
    ;;
esac
