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
WRAPPER="$ROOT/harness/docker/acc_cc_wrapper.sh"
IMAGE="${ACC_DOCKER_IMAGE:-acc-linux}"
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
# Optional busybox initrd (A06). Build via harness/initrd/build_busybox_initrd.sh.
# When present, QEMU gets -initrd inside the container; PASS stamp remains A17's concern.
INITRD_ARCH="${INITRD_ARCH:-$KERNEL_ARCH}"
case "$INITRD_ARCH" in
  arm64|aarch64) INITRD_ARCH=arm64 ;;
  x86_64|x86|amd64) INITRD_ARCH=x86_64 ;;
esac
# Prefer uncompressed cpio when present (A08 freestanding unpack scans newc;
# gzip still needs decompressor before cpio walk).
INITRD_CPIO="$ROOT/harness/initrd/out/$INITRD_ARCH/initramfs.cpio"
INITRD_DEFAULT="$ROOT/harness/initrd/out/$INITRD_ARCH/initramfs.cpio.gz"
if [[ -z "${INITRD_PATH:-}" ]]; then
  if [[ -f "$INITRD_CPIO" ]]; then
    INITRD_PATH="$INITRD_CPIO"
  else
    INITRD_PATH="$INITRD_DEFAULT"
  fi
fi
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
  echo "INITRD_ARCH=$INITRD_ARCH"
  echo "INITRD_PATH=$INITRD_PATH"
  if [[ -f "$INITRD_PATH" ]]; then
    echo "INITRD: present ($(wc -c <"$INITRD_PATH") bytes) — QEMU will use -initrd"
  else
    echo "INITRD: missing (optional; build with harness/initrd/build_busybox_initrd.sh)"
  fi
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

# Host acc (may be Darwin binary — only used for freestanding -S smoke on host)
HOST_ACC="${ACC_BIN:-$ROOT/target/release/acc}"
if [[ -x "$HOST_ACC" ]]; then
  log "host_acc: $HOST_ACC"
  "$HOST_ACC" --help 2>&1 | head -12 | tee -a "$LOG" || true
else
  log "host_acc: missing ($HOST_ACC) — will rely on in-Docker cargo build"
fi

# --- 1. Docker availability + image ---
DOCKER_OK=0
if command -v docker >/dev/null 2>&1 && timeout 300 docker info >/dev/null 2>&1; then
  DOCKER_OK=1
  log "docker: available"
  if [[ "${ACC_SKIP_DOCKER_BUILD:-}" == "1" ]] \
    || timeout 15 docker image inspect "$IMAGE" >/dev/null 2>&1; then
    log "docker_image: $IMAGE present (skip build; set ACC_FORCE_DOCKER_BUILD=1 to rebuild)"
  else
    log "=== docker build $IMAGE ==="
    set +e
    docker build ${DOCKER_BUILD_FLAGS:-} ${DOCKER_PLATFORM_ARGS[@]+"${DOCKER_PLATFORM_ARGS[@]}"} -t "$IMAGE" -f "$ROOT/harness/docker/Dockerfile.linux" "$ROOT/harness/docker" 2>&1 | tee -a "$LOG"
    db_ec=${PIPESTATUS[0]}
    set -e
    if [[ $db_ec -ne 0 ]]; then
      verdict "BLOCKED" "docker image build failed (ec=$db_ec); see log above"
      exit 2
    fi
    log "docker_image: $IMAGE built ok"
  fi
else
  log "docker: UNAVAILABLE (daemon missing, slow, or not running)"
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
if [[ -x "$HOST_ACC" ]]; then
  set +e
  "$HOST_ACC" --target-os linux -m x86_64 -S -o "$SCRATCH/kstub.s" "$SCRATCH/kstub.c" 2>>"$LOG"
  kstub_ec=$?
  set -e
  log "kstub_compile_ec=$kstub_ec"
  head -20 "$SCRATCH/kstub.s" >>"$LOG" 2>/dev/null || true
else
  log "kstub_compile_ec=skipped (no host acc)"
fi

# Without Docker we cannot run Linux make / QEMU honestly on macOS.
if [[ "$DOCKER_OK" -ne 1 ]]; then
  verdict "BLOCKED" "Docker required for C1 on this host; Linux make + QEMU not run. Scripts ready under harness/docker/."
  exit 3
fi

# --- 4. Inside Docker: build Linux acc, tinyconfig, make with CC=wrapper ---
# Map arch for kernel Makefile
case "$KERNEL_ARCH" in
  x86_64|x86) KARCH=x86 ;;
  arm64|aarch64) KARCH=arm64 ;;
  *) KARCH=x86 ;;
esac

log "=== docker: cargo build acc (Linux binary) + tinyconfig + make CC=wrapper ==="
# Map acc -m flag
case "$KERNEL_ARCH" in
  arm64|aarch64) ACC_M=aarch64 ;;
  *) ACC_M=x86_64 ;;
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
  -e ACC_M="$ACC_M" \
  -e VER="$VER" \
  -e KSRC_REL="$KSRC_REL" \
  -e JOBS="$JOBS" \
  -e ACC_PARSE_ALL_BODIES=1 \
  -e ACC_SOFT_SKIP_BODIES=0 \
  -e ACC_ALLOW_SOFT_SYSCC=0 \
  -e ACC_SOFT_FREESTANDING=0 \
  -e ACC_KERNEL_FREESTANDING=1 \
  -e INITRD_PATH="$INITRD_PATH" \
  -e INITRD_REL="${INITRD_REL:-${INITRD_PATH#$ROOT/}}" \
  "$IMAGE" bash /work/harness/docker/c1_build_inner.sh
docker_ec=$?
set -e
log "docker_run_ec=$docker_ec"

# --- 5. Verdict ---
if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
  if [[ -f "$SCRATCH/c1_boot_marker_x86_64" ]] && [[ -f "$SCRATCH/qemu_boot_x86_64.log" ]]; then
    verdict "PASS"
    log "C1 x86_64 boot marker present — review qemu_boot_x86_64.log for evidence"
    exit 0
  fi
elif [[ -f "$SCRATCH/c1_boot_marker" ]] && [[ -f "$SCRATCH/qemu_boot.log" || -f "$SCRATCH/qemu_boot_a09.log" ]]; then
  verdict "PASS" # should not happen until language ready
  log "C1 boot marker present — review qemu_boot.log for evidence"
  exit 0
fi

# Default honest blocked path
last_fail="see kernel_make_full.log / stage_c_kernel.log tail"
if [[ -f "$SCRATCH/kernel_make_full.log" ]]; then
  last_fail="$(grep -E "acc_cc_wrapper:|ERROR:" "$SCRATCH/kernel_make_full.log" | head -3 | tr '\n' ' ' || true)"
fi
if [[ -z "${last_fail// }" ]]; then
  last_fail="make failed before producing bootable image; acc language coverage insufficient for kernel C"
fi

verdict "BLOCKED" "full Linux $VER QEMU boot not achieved. last_failure: $last_fail. Expected gaps: preprocessor (-E/-I/-D), GNU C extensions, attributes, inline asm, bitfields/complex types, kernel headers, freestanding builtins. Wrapper correctly refuses gcc fallback on .c. Docker image path exercised when docker available."
exit 3
