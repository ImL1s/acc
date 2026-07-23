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
# Default kernel arch = host/container native. On Apple Silicon Docker (aarch64)
# building x86_64 produces unassemblable .s (host as is aarch64). Prefer arm64
# unless user forces KERNEL_ARCH=x86_64 with --platform linux/amd64.
if [[ -z "${KERNEL_ARCH:-}" ]]; then
  KERNEL_ARCH=x86_64
fi
DOCKER_PLATFORM_ARGS=()
case "$(uname -m)" in
  arm64|aarch64)
    if [[ "$KERNEL_ARCH" == "x86_64" || "$KERNEL_ARCH" == "x86" ]]; then
      DOCKER_PLATFORM_ARGS=(--platform linux/amd64)
    fi
    ;;
esac
JOBS="${JOBS:-4}"
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
  docker build ${DOCKER_BUILD_FLAGS:-} ${DOCKER_PLATFORM_ARGS[@]+"${DOCKER_PLATFORM_ARGS[@]}"} -t "$IMAGE" -f "$ROOT/harness/docker/Dockerfile.linux" "$ROOT/harness/docker" 2>&1 | tee -a "$LOG"
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
# Map ggcc -m flag
case "$KERNEL_ARCH" in
  arm64|aarch64) GGCC_M=aarch64 ;;
  *) GGCC_M=x86_64 ;;
esac
if [[ ${#DOCKER_PLATFORM_ARGS[@]} -gt 0 ]]; then
  log "docker platform: linux/amd64 (forced for x86_64 kernel on aarch64 host)"
fi
set +e
# shellcheck disable=SC2086
KSRC_REL="${SRC_DIR#$ROOT/}"
docker run --rm \
  ${DOCKER_PLATFORM_ARGS[@]+"${DOCKER_PLATFORM_ARGS[@]}"} \
  -v "$ROOT":/work \
  -v "$SCRATCH":/scratch \
  -w /work \
  -e KERNEL_ARCH="$KERNEL_ARCH" \
  -e KARCH="$KARCH" \
  -e GGCC_M="$GGCC_M" \
  -e VER="$VER" \
  -e KSRC_REL="$KSRC_REL" \
  -e JOBS="$JOBS" \
  -e GGCC_ALLOW_SOFT_SYSCC=0 \
  -e GGCC_SOFT_FREESTANDING=0 \
  "$IMAGE" bash -lc '
    set -euo pipefail
    LOG=/scratch/stage_c_kernel.log
    log() { echo "$@" | tee -a "$LOG"; }

    log "container: $(uname -a)"
    log "KERNEL_ARCH=$KERNEL_ARCH KARCH=$KARCH GGCC_M=$GGCC_M"
    log "GGCC_ALLOW_SOFT_SYSCC=${GGCC_ALLOW_SOFT_SYSCC:-0} (must stay 0 for C1)"
    log "=== cargo build --release (Linux ggcc; separate target dir) ==="
    # Do NOT overwrite host macOS target/release/ggcc with Linux ELF.
    export CARGO_TARGET_DIR=/work/target-linux
    cargo build --release 2>&1 | tee -a "$LOG"
    GGCC=/work/target-linux/release/ggcc
    test -x "$GGCC"
    export GGCC
    export GGCC_ARCH="$GGCC_M"
    export GGCC_TARGET_OS=linux
    export SYSCC=gcc
    # Explicit: no soft body skip / no soft system-CC on .c
    unset GGCC_SOFT_SKIP_BODIES || true
    # Soft freestanding body replacements OFF on C1 PASS path (emit real C).
    unset GGCC_SOFT_FREESTANDING || true
    export GGCC_SOFT_FREESTANDING=0
    export GGCC_ALLOW_SOFT_SYSCC=0
    WRAP=/work/harness/docker/ggcc_cc_wrapper.sh
    chmod +x "$WRAP"
    "$GGCC" --help 2>&1 | head -8 | tee -a "$LOG" || true

    # Trivial probe inside container (native arch asm)
    echo "int x;" > /scratch/kprobe.c
    set +e
    "$GGCC" --target-os linux -m "$GGCC_M" -S -o /scratch/kprobe.s /scratch/kprobe.c 2>>"$LOG"
    log "kprobe_ec=$?"
    set -e
    head -10 /scratch/kprobe.s >>"$LOG" 2>/dev/null || true

    rm -rf /tmp/linux-src
    mkdir -p /tmp/linux-src
    if [[ -f "/scratch/linux-$VER.tar.xz" ]]; then
      log "extracting /scratch/linux-$VER.tar.xz to container-local /tmp/linux-src..."
      tar -xJf "/scratch/linux-$VER.tar.xz" -C /tmp/linux-src --strip-components=1
    elif [[ -d "/work/$KSRC_REL" ]]; then
      log "copying /work/$KSRC_REL to container-local /tmp/linux-src..."
      cp -r "/work/$KSRC_REL"/* /tmp/linux-src/ 2>/dev/null || true
    fi
    cd /tmp/linux-src
    log "=== make ARCH=$KARCH tinyconfig ==="
    make ARCH="$KARCH" defconfig 2>&1 | tee -a "$LOG" | tail -20 || true
    make ARCH="$KARCH" tinyconfig 2>&1 | tee -a "$LOG" | tail -20
    if [[ "$KERNEL_ARCH" == "x86_64" || "$KERNEL_ARCH" == "x86" ]]; then
      echo "CONFIG_64BIT=y" >> .config
      make ARCH="$KARCH" olddefconfig 2>&1 | tee -a "$LOG" | tail -20
    fi

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

    # Artifacts? Prefer arch-matching Image paths (stale other-arch images ignored by path).
    bz=""
    if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
      for cand in arch/arm64/boot/Image arch/arm64/boot/Image.gz; do
        if [[ -f "$cand" ]]; then bz="$cand"; break; fi
      done
    else
      if [[ -f arch/x86/boot/bzImage ]]; then bz=arch/x86/boot/bzImage; fi
    fi
    if [[ -z "$bz" && -f vmlinux ]]; then bz=vmlinux; fi
    if [[ -n "$bz" ]]; then
      log "kernel_image: $bz"
      log "=== QEMU boot attempt ==="
      set +e
      if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
        timeout 30 qemu-system-aarch64 -M virt -cpu cortex-a57 -kernel "$bz" \
          -nographic -append "console=ttyAMA0" \
          2>&1 | tee /scratch/qemu_boot.log | tee -a "$LOG" | tail -40
      else
        timeout 30 qemu-system-x86_64 -kernel "$bz" \
          -nographic -append "console=ttyS0 earlyprintk=serial" \
          -serial mon:stdio -no-reboot \
          2>&1 | tee /scratch/qemu_boot.log | tee -a "$LOG" | tail -40
      fi
      qec=${PIPESTATUS[0]}
      set -e
      log "qemu_ec=$qec"
      if grep -qE "Linux version|Kernel command line|Run /init" /scratch/qemu_boot.log 2>/dev/null; then
        log "BOOT_EVIDENCE: serial shows kernel start strings"
        echo PASS_BOOT > /scratch/c1_boot_marker
      else
        log "BOOT_EVIDENCE: missing (no boot strings in serial log)"
      fi
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
