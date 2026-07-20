#!/usr/bin/env bash
# Stage C1: fetch Linux 6.9, minimal config, attempt CC=ggcc (via wrapper) + QEMU boot path.
#
# Honest status: full kernel is expected to FAIL with current ggcc language coverage
# (GNU attributes, inline asm, complex preprocessor, kernel headers, -E, etc.).
# This script records VERDICT + last failure; it does NOT claim C1 complete on partial smoke.
#
# Policy: kernel .c is compiled only by ggcc (wrapper). System as/ld/cc assemble & link
# emitted .s/.o only. HOSTCC may be system gcc for kconfig/host tools (not kernel .c).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="${SCRATCH:?SCRATCH required (evidence dir for stage_c_kernel.log)}"
VER="${KERNEL_VER:-6.9}"
SRC_DIR="${KERNEL_SRC:-$ROOT/third_party/linux-$VER}"
LOG="$SCRATCH/stage_c_kernel.log"
WRAPPER="$ROOT/harness/docker/ggcc_cc_wrapper.sh"
IMAGE="${GGCC_DOCKER_IMAGE:-ggcc-linux}"
# Default x86_64 for QEMU bzImage path; override KERNEL_ARCH=arm64 if desired.
KERNEL_ARCH="${KERNEL_ARCH:-x86_64}"
JOBS="${JOBS:-$(command -v nproc >/dev/null 2>&1 && nproc || sysctl -n hw.ncpu 2>/dev/null || echo 2)}"
mkdir -p "$SCRATCH"

log() { echo "$@" | tee -a "$LOG"; }

: >"$LOG"
{
  echo "# Stage C1 Linux $VER kernel attempt"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(uname -a)"
  echo "ROOT=$ROOT"
  echo "SCRATCH=$SCRATCH"
  echo "KERNEL_ARCH=$KERNEL_ARCH"
  echo "WRAPPER=$WRAPPER"
  echo "IMAGE=$IMAGE"
} >>"$LOG"

verdict() {
  local v="$1"
  shift
  log "VERDICT: $v"
  if [[ $# -gt 0 ]]; then
    log "blocked_reason: $*"
  fi
}

# --- 0. Preconditions ---
if [[ ! -f "$WRAPPER" ]]; then
  verdict "BLOCKED" "missing $WRAPPER"
  exit 2
fi
chmod +x "$WRAPPER" 2>/dev/null || true

# Host ggcc (may be Darwin binary — only used for freestanding -S smoke on host)
HOST_GGCC="${GGCC:-$ROOT/target/release/ggcc}"
if [[ -x "$HOST_GGCC" ]]; then
  log "host_ggcc: $HOST_GGCC"
  "$HOST_GGCC" --help 2>&1 | head -12 | tee -a "$LOG" || true
else
  log "host_ggcc: missing ($HOST_GGCC) — will rely on in-Docker cargo build"
fi

# --- 1. Docker availability + image ---
DOCKER_OK=0
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  DOCKER_OK=1
  log "docker: available"
  log "=== docker build $IMAGE ==="
  set +e
  docker build -t "$IMAGE" -f "$ROOT/harness/docker/Dockerfile.linux" "$ROOT/harness/docker" 2>&1 | tee -a "$LOG"
  db_ec=${PIPESTATUS[0]}
  set -e
  if [[ $db_ec -ne 0 ]]; then
    verdict "BLOCKED" "docker image build failed (ec=$db_ec); see log above"
    exit 2
  fi
  log "docker_image: $IMAGE built_or_cached ok"
else
  log "docker: UNAVAILABLE (daemon missing or not running)"
fi

# --- 2. Fetch Linux $VER ---
if [[ ! -d "$SRC_DIR" ]]; then
  log "fetching linux-$VER ..."
  mkdir -p "$ROOT/third_party"
  TAR="$SCRATCH/linux-$VER.tar.xz"
  if [[ ! -f "$TAR" ]]; then
    set +e
    curl -fL --retry 3 -o "$TAR" \
      "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${VER}.tar.xz" 2>&1 | tee -a "$LOG"
    curl_ec=${PIPESTATUS[0]}
    set -e
    if [[ $curl_ec -ne 0 ]]; then
      verdict "BLOCKED" "fetch_failed linux-$VER (curl ec=$curl_ec)"
      exit 2
    fi
  else
    log "using cached tarball $TAR"
  fi
  tar -xJf "$TAR" -C "$ROOT/third_party" 2>&1 | tee -a "$LOG"
fi
if [[ ! -d "$SRC_DIR" ]]; then
  verdict "BLOCKED" "kernel source missing after fetch: $SRC_DIR"
  exit 2
fi
log "kernel_src: $SRC_DIR"

# --- 3. Host freestanding Linux ELF asm smoke (no claim of kernel boot) ---
cat >"$SCRATCH/kstub.c" <<'C'
/* freestanding stub — Linux ELF path only; not a kernel */
void _start(void);
void _start(void) {
  for (;;) { }
}
C
if [[ -x "$HOST_GGCC" ]]; then
  set +e
  "$HOST_GGCC" --target-os linux -m x86_64 -S -o "$SCRATCH/kstub.s" "$SCRATCH/kstub.c" 2>>"$LOG"
  kstub_ec=$?
  set -e
  log "kstub_compile_ec=$kstub_ec"
  head -20 "$SCRATCH/kstub.s" >>"$LOG" 2>/dev/null || true
else
  log "kstub_compile_ec=skipped (no host ggcc)"
fi

# Without Docker we cannot run Linux make / QEMU honestly on macOS.
if [[ "$DOCKER_OK" -ne 1 ]]; then
  verdict "BLOCKED" "Docker required for C1 on this host; Linux make + QEMU not run. Scripts ready under harness/docker/."
  exit 3
fi

# --- 4. Inside Docker: build Linux ggcc, tinyconfig, make with CC=wrapper ---
# Map arch for kernel Makefile
case "$KERNEL_ARCH" in
  x86_64|x86) KARCH=x86 ;;
  arm64|aarch64) KARCH=arm64 ;;
  *) KARCH=x86 ;;
esac

log "=== docker: cargo build ggcc (Linux binary) + tinyconfig + make CC=wrapper ==="
set +e
docker run --rm \
  -v "$ROOT":/work \
  -v "$SCRATCH":/scratch \
  -w /work \
  -e KERNEL_ARCH="$KERNEL_ARCH" \
  -e KARCH="$KARCH" \
  -e VER="$VER" \
  -e JOBS="$JOBS" \
  "$IMAGE" bash -lc '
    set -euo pipefail
    LOG=/scratch/stage_c_kernel.log
    log() { echo "$@" | tee -a "$LOG"; }

    log "container: $(uname -a)"
    log "=== cargo build --release (Linux ggcc) ==="
    cargo build --release 2>&1 | tee -a "$LOG"
    GGCC=/work/target/release/ggcc
    test -x "$GGCC"
    export GGCC
    export GGCC_ARCH="$KERNEL_ARCH"
    export GGCC_TARGET_OS=linux
    export SYSCC=gcc
    WRAP=/work/harness/docker/ggcc_cc_wrapper.sh
    chmod +x "$WRAP"
    "$GGCC" --help 2>&1 | head -8 | tee -a "$LOG" || true

    # Trivial probe inside container
    echo "int x;" > /scratch/kprobe.c
    set +e
    "$GGCC" --target-os linux -m x86_64 -S -o /scratch/kprobe.s /scratch/kprobe.c 2>>"$LOG"
    log "kprobe_ec=$?"
    set -e
    head -10 /scratch/kprobe.s >>"$LOG" 2>/dev/null || true

    cd "/work/third_party/linux-$VER"
    log "=== make ARCH=$KARCH tinyconfig ==="
    make ARCH="$KARCH" tinyconfig 2>&1 | tee -a "$LOG" | tail -20

    # Optional: enable early printk / serial for QEMU if config exists
    if [[ -f .config ]]; then
      # Keep tinyconfig minimal; document that full boot needs more symbols later
      log "config: tinyconfig generated ($(wc -l < .config) lines)"
    fi

    log "=== make ARCH=$KARCH CC=ggcc_cc_wrapper HOSTCC=gcc (expect language fail) ==="
    # HOSTCC=gcc: kconfig/fixdep host tools only — not kernel .c
    # CC=wrapper: kernel .c → ggcc only
    set +e
    make ARCH="$KARCH" \
      CC="$WRAP" \
      HOSTCC=gcc \
      HOSTCXX=g++ \
      -j"$JOBS" \
      2>&1 | tee /scratch/kernel_make_full.log | tee -a "$LOG" | tail -80
    make_ec=${PIPESTATUS[0]}
    set -e
    log "make_ec=$make_ec"

    # Capture last failure snippet
    if [[ $make_ec -ne 0 ]]; then
      log "=== last_failure (tail kernel_make_full.log) ==="
      tail -60 /scratch/kernel_make_full.log | tee -a "$LOG"
      # Prefer first ggcc_cc_wrapper / ERROR line
      log "=== first_ggcc_error ==="
      grep -n -E "ggcc_cc_wrapper:|ERROR:|error:" /scratch/kernel_make_full.log | head -30 | tee -a "$LOG" || true
    fi

    # Artifacts?
    bz=""
    for cand in arch/x86/boot/bzImage arch/arm64/boot/Image vmlinux; do
      if [[ -f "$cand" ]]; then
        bz="$cand"
        log "kernel_image: $cand ($(stat -c%s "$cand" 2>/dev/null || stat -f%z "$cand") bytes)"
      fi
    done

    if [[ -n "$bz" ]]; then
      log "=== QEMU boot attempt ==="
      case "$KERNEL_ARCH" in
        x86_64|x86)
          set +e
          timeout 30 qemu-system-x86_64 -kernel "$bz" \
            -nographic -append "console=ttyS0 earlyprintk=serial" \
            -serial mon:stdio -no-reboot \
            2>&1 | tee /scratch/qemu_boot.log | tee -a "$LOG" | tail -40
          qec=${PIPESTATUS[0]}
          set -e
          log "qemu_ec=$qec"
          if grep -qE "Linux version|Kernel command line|Run /init" /scratch/qemu_boot.log 2>/dev/null; then
            log "BOOT_EVIDENCE: serial shows kernel start strings"
            echo PASS_BOOT > /scratch/c1_boot_marker
          else
            log "BOOT_EVIDENCE: missing (no boot strings in serial log)"
          fi
          ;;
        arm64|aarch64)
          set +e
          timeout 30 qemu-system-aarch64 -M virt -cpu cortex-a57 -kernel "$bz" \
            -nographic -append "console=ttyAMA0" \
            2>&1 | tee /scratch/qemu_boot.log | tee -a "$LOG" | tail -40
          qec=${PIPESTATUS[0]}
          set -e
          log "qemu_ec=$qec"
          ;;
      esac
    else
      log "kernel_image: none — skip QEMU (build did not produce bzImage/Image/vmlinux)"
    fi

    # Exit code for outer script
    if [[ -f /scratch/c1_boot_marker ]]; then
      exit 0
    fi
    exit 3
  '
docker_ec=$?
set -e
log "docker_run_ec=$docker_ec"

# --- 5. Verdict ---
if [[ -f "$SCRATCH/c1_boot_marker" ]]; then
  verdict "PASS" # should not happen until language ready
  log "C1 boot marker present — review qemu_boot.log for evidence"
  exit 0
fi

# Default honest blocked path
last_fail="see kernel_make_full.log / stage_c_kernel.log tail"
if [[ -f "$SCRATCH/kernel_make_full.log" ]]; then
  last_fail="$(grep -E "ggcc_cc_wrapper:|ERROR:" "$SCRATCH/kernel_make_full.log" | head -3 | tr '\n' ' ' || true)"
fi
if [[ -z "${last_fail// }" ]]; then
  last_fail="make failed before producing bootable image; ggcc language coverage insufficient for kernel C"
fi

verdict "BLOCKED" "full Linux $VER QEMU boot not achieved. last_failure: $last_fail. Expected gaps: preprocessor (-E/-I/-D), GNU C extensions, attributes, inline asm, bitfields/complex types, kernel headers, freestanding builtins. Wrapper correctly refuses gcc fallback on .c. Docker image path exercised when docker available."
exit 3
