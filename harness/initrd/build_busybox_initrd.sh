#!/usr/bin/env bash
# Build a static busybox initrd (cpio/gz) for Stage C1 busybox-shell bar.
#
# Recipe mirrors CCC BUILDING_LINUX.txt (third_party/ccc-harness-ref, no src/):
#   /init → busybox setsid cttyhack /bin/sh
#
# Success for this script = initrd artifact built. Full busybox PASS is separate.
#
# Usage (from repo root, Docker recommended on macOS):
#   bash harness/initrd/build_busybox_initrd.sh
#   INITRD_ARCH=arm64 bash harness/initrd/build_busybox_initrd.sh
#   INITRD_OUT=/path/to/initramfs.cpio.gz bash harness/initrd/build_busybox_initrd.sh
#
# Env:
#   INITRD_ARCH   arm64|aarch64|x86_64|x86 (default: host / KERNEL_ARCH)
#   INITRD_OUT    output path (default: harness/initrd/out/<arch>/initramfs.cpio.gz)
#   BUSYBOX_VER   default 1.36.1
#   BUSYBOX_SRC   override unpacked busybox tree
#   KERNEL_SRC    optional; if usr/gen_init_cpio exists, prefer it
#   ACC_DOCKER_IMAGE / GGCC_DOCKER_IMAGE  default acc-linux
#   FORCE_DOCKER  1 = always build inside Docker
#   JOBS          parallel make jobs
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BUSYBOX_VER="${BUSYBOX_VER:-1.36.1}"
JOBS="${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
IMAGE="${ACC_DOCKER_IMAGE:-${GGCC_DOCKER_IMAGE:-acc-linux}}"
FORCE_DOCKER="${FORCE_DOCKER:-0}"

# Arch selection
if [[ -z "${INITRD_ARCH:-}" ]]; then
  if [[ -n "${KERNEL_ARCH:-}" ]]; then
    INITRD_ARCH="$KERNEL_ARCH"
  else
    case "$(uname -m)" in
      arm64|aarch64) INITRD_ARCH=arm64 ;;
      x86_64|amd64) INITRD_ARCH=x86_64 ;;
      *) INITRD_ARCH=x86_64 ;;
    esac
  fi
fi
case "$INITRD_ARCH" in
  arm64|aarch64) INITRD_ARCH=arm64; BB_ARCH=arm64; HOST_TRIPLE_HINT=aarch64 ;;
  x86_64|x86|amd64) INITRD_ARCH=x86_64; BB_ARCH=x86_64; HOST_TRIPLE_HINT=x86_64 ;;
  *)
    echo "error: unsupported INITRD_ARCH=$INITRD_ARCH (use arm64 or x86_64)" >&2
    exit 2
    ;;
esac

OUT_DIR="${INITRD_OUT_DIR:-$ROOT/harness/initrd/out/$INITRD_ARCH}"
INITRD_OUT="${INITRD_OUT:-$OUT_DIR/initramfs.cpio.gz}"
WORKDIR="${INITRD_WORKDIR:-$ROOT/third_party/busybox-build-$INITRD_ARCH}"
BUSYBOX_TAR_DIR="${BUSYBOX_CACHE:-$ROOT/third_party}"
BUSYBOX_SRC="${BUSYBOX_SRC:-$WORKDIR/busybox-$BUSYBOX_VER}"

mkdir -p "$OUT_DIR" "$WORKDIR" "$BUSYBOX_TAR_DIR"

log() { echo "[build_busybox_initrd] $*"; }

need_docker() {
  if [[ "$FORCE_DOCKER" == "1" ]]; then
    return 0
  fi
  # macOS / non-Linux host cannot produce a Linux static busybox natively.
  if [[ "$(uname -s)" != "Linux" ]]; then
    return 0
  fi
  # Wrong arch inside Linux → use Docker with matching platform if available.
  local m
  m="$(uname -m)"
  case "$INITRD_ARCH" in
    arm64)
      [[ "$m" == "aarch64" || "$m" == "arm64" ]] && return 1
      return 0
      ;;
    x86_64)
      [[ "$m" == "x86_64" || "$m" == "amd64" ]] && return 1
      return 0
      ;;
  esac
  return 0
}

DOCKER_PLATFORM_ARGS=()
case "$INITRD_ARCH" in
  arm64) DOCKER_PLATFORM_ARGS=(--platform linux/arm64) ;;
  x86_64) DOCKER_PLATFORM_ARGS=(--platform linux/amd64) ;;
esac

# --- Docker re-exec path ---
if need_docker; then
  if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    echo "error: Docker required to build Linux busybox initrd on this host" >&2
    exit 2
  fi
  log "building Docker image $IMAGE (if needed)"
  docker build ${DOCKER_BUILD_FLAGS:-} ${DOCKER_PLATFORM_ARGS[@]+"${DOCKER_PLATFORM_ARGS[@]}"} \
    -t "$IMAGE" -f "$ROOT/harness/docker/Dockerfile.linux" "$ROOT/harness/docker"
  log "re-exec inside $IMAGE (INITRD_ARCH=$INITRD_ARCH)"
  # Avoid recursive Docker: force native path inside container.
  exec docker run --rm \
    ${DOCKER_PLATFORM_ARGS[@]+"${DOCKER_PLATFORM_ARGS[@]}"} \
    -v "$ROOT":/work \
    -w /work \
    -e FORCE_DOCKER=0 \
    -e INITRD_ARCH="$INITRD_ARCH" \
    -e INITRD_OUT="/work/${INITRD_OUT#$ROOT/}" \
    -e INITRD_OUT_DIR="/work/${OUT_DIR#$ROOT/}" \
    -e INITRD_WORKDIR="/work/${WORKDIR#$ROOT/}" \
    -e BUSYBOX_VER="$BUSYBOX_VER" \
    -e BUSYBOX_CACHE="/work/${BUSYBOX_TAR_DIR#$ROOT/}" \
    -e BUSYBOX_SRC="/work/${BUSYBOX_SRC#$ROOT/}" \
    -e KERNEL_SRC="${KERNEL_SRC:+/work/${KERNEL_SRC#$ROOT/}}" \
    -e JOBS="$JOBS" \
    "$IMAGE" bash /work/harness/initrd/build_busybox_initrd.sh
fi

# --- Native Linux build (inside container or Linux host) ---
log "native build INITRD_ARCH=$INITRD_ARCH JOBS=$JOBS"
command -v gcc >/dev/null
command -v make >/dev/null
command -v cpio >/dev/null
command -v gzip >/dev/null

TAR="$BUSYBOX_TAR_DIR/busybox-${BUSYBOX_VER}.tar.bz2"
if [[ ! -f "$TAR" ]]; then
  log "fetching busybox-$BUSYBOX_VER ..."
  curl -fL --retry 3 -o "$TAR" \
    "https://busybox.net/downloads/busybox-${BUSYBOX_VER}.tar.bz2"
fi

if [[ ! -d "$BUSYBOX_SRC" ]]; then
  log "extracting $TAR → $WORKDIR"
  mkdir -p "$WORKDIR"
  tar -xjf "$TAR" -C "$WORKDIR"
fi
test -d "$BUSYBOX_SRC"

cd "$BUSYBOX_SRC"
if [[ ! -f .config ]]; then
  log "make ARCH=$BB_ARCH defconfig"
  make ARCH="$BB_ARCH" defconfig
fi
# Static link (CCC BUILDING_LINUX.txt)
if grep -q '^# CONFIG_STATIC is not set' .config 2>/dev/null; then
  sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config
elif ! grep -q '^CONFIG_STATIC=y' .config 2>/dev/null; then
  echo 'CONFIG_STATIC=y' >> .config
fi
# CONFIG_TC often breaks static busybox builds (CCC recipe)
if grep -q '^CONFIG_TC=y' .config 2>/dev/null; then
  sed -i 's/CONFIG_TC=y/# CONFIG_TC is not set/' .config
fi
# Ensure shell + cttyhack + setsid applets are present (defconfig usually has them)
for opt in CONFIG_SH CONFIG_ASH CONFIG_CTTYHACK CONFIG_SETSID CONFIG_FEATURE_SH_IS_ASH; do
  if grep -q "^# ${opt} is not set" .config 2>/dev/null; then
    sed -i "s/# ${opt} is not set/${opt}=y/" .config
  elif ! grep -q "^${opt}=y" .config 2>/dev/null; then
    echo "${opt}=y" >> .config
  fi
done
make ARCH="$BB_ARCH" oldconfig </dev/null 2>/dev/null || true

log "make ARCH=$BB_ARCH -j$JOBS (static busybox)"
make ARCH="$BB_ARCH" -j"$JOBS"
test -x "$BUSYBOX_SRC/busybox"

# --- Rootfs + /init (CCC basic-init) ---
ROOTFS="$WORKDIR/rootfs"
rm -rf "$ROOTFS"
mkdir -p "$ROOTFS"/{dev,proc,sys,root,bin,sbin,usr/bin,usr/sbin,usr/bin/games,etc}

cat >"$ROOTFS/init" <<'EOF'
#!/bin/busybox sh
/bin/busybox mkdir -p /dev /etc /proc /sys /bin /sbin /usr/bin/games /usr/sbin
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sys /sys
/bin/busybox mount -t devtmpfs dev /dev
/bin/busybox --install -s 2>/dev/null
export TERM=linux
export PATH=/bin:/sbin:/usr/bin:/usr/sbin:/usr/bin/games
# Ensure PS1 prints on serial even when cttyhack cannot grab a TTY
# (freestanding EL0 / virtio-less QEMU). Status bar greps literal /#.
export PS1='/# '
echo '/#'
echo 1 > /proc/sys/kernel/printk
sleep 0.2
exec /bin/busybox setsid /bin/busybox cttyhack /bin/sh
EOF
chmod 755 "$ROOTFS/init"

cp -f "$BUSYBOX_SRC/busybox" "$ROOTFS/bin/busybox"
chmod 755 "$ROOTFS/bin/busybox"
# Early aliases before busybox --install (kernel may look for /bin/sh)
ln -sf busybox "$ROOTFS/bin/sh"
ln -sf busybox "$ROOTFS/bin/ash"

# Device nodes (when not using gen_init_cpio nod entries)
if [[ "$(id -u)" -eq 0 ]]; then
  mknod -m 600 "$ROOTFS/dev/console" c 5 1 2>/dev/null || true
  mknod -m 666 "$ROOTFS/dev/tty" c 5 0 2>/dev/null || true
  mknod -m 666 "$ROOTFS/dev/null" c 1 3 2>/dev/null || true
fi

mkdir -p "$(dirname "$INITRD_OUT")"

# Prefer kernel gen_init_cpio when available AND runnable on this arch
# (arm64 host may leave an aarch64 binary that fails under linux/amd64).
GEN_INIT_CPIO=""
ggcc_gen_init_ok() {
  local p="$1"
  [[ -x "$p" ]] || return 1
  # Reject wrong-ISA binaries (e.g. aarch64 gen_init_cpio in amd64 container).
  if command -v file >/dev/null 2>&1; then
    case "$(uname -m)" in
      x86_64|amd64)
        file "$p" 2>/dev/null | grep -qiE 'x86-64|x86_64|Intel 80386' || return 1
        ;;
      aarch64|arm64)
        file "$p" 2>/dev/null | grep -qiE 'aarch64|ARM aarch64' || return 1
        ;;
    esac
  fi
  return 0
}
if [[ -n "${KERNEL_SRC:-}" ]] && ggcc_gen_init_ok "${KERNEL_SRC}/usr/gen_init_cpio"; then
  GEN_INIT_CPIO="${KERNEL_SRC}/usr/gen_init_cpio"
elif ggcc_gen_init_ok "$ROOT/third_party/linux-6.9/usr/gen_init_cpio"; then
  GEN_INIT_CPIO="$ROOT/third_party/linux-6.9/usr/gen_init_cpio"
fi

CPIO_RAW="${INITRD_OUT%.gz}"
[[ "$CPIO_RAW" == "$INITRD_OUT" ]] && CPIO_RAW="$OUT_DIR/initramfs.cpio"

if [[ -n "$GEN_INIT_CPIO" ]]; then
  log "packing with gen_init_cpio ($GEN_INIT_CPIO)"
  LIST="$WORKDIR/initramfs.list"
  cat >"$LIST" <<EOF
dir /dev 0755 0 0
nod /dev/console 0600 0 0 c 5 1
nod /dev/tty 0666 0 0 c 5 0
nod /dev/null 0666 0 0 c 1 3
dir /proc 0755 0 0
dir /sys 0755 0 0
dir /root 0700 0 0
dir /bin 0755 0 0
dir /sbin 0755 0 0
dir /usr 0755 0 0
dir /usr/bin 0755 0 0
dir /usr/bin/games 0755 0 0
dir /usr/sbin 0755 0 0
dir /etc 0755 0 0
file /init $ROOTFS/init 0755 0 0
file /bin/busybox $ROOTFS/bin/busybox 0755 0 0
slink /bin/sh busybox 0755 0 0
slink /bin/ash busybox 0755 0 0
EOF
  "$GEN_INIT_CPIO" "$LIST" >"$CPIO_RAW"
  gzip -9 -c "$CPIO_RAW" >"$INITRD_OUT"
else
  log "packing with find|cpio (no runnable gen_init_cpio)"
  (
    cd "$ROOTFS"
    # Strip leading ./ so stubs match init / bin/busybox (gen_init_cpio style).
    find . | sed 's|^\./||' | sed '/^\.$/d' | cpio -ov --format=newc 2>/dev/null
  ) >"$CPIO_RAW"
  gzip -9 -c "$CPIO_RAW" >"$INITRD_OUT"
fi

test -s "$INITRD_OUT"
test -s "$CPIO_RAW"
log "OK initrd: $INITRD_OUT ($(wc -c <"$INITRD_OUT") bytes) + uncompressed $CPIO_RAW ($(wc -c <"$CPIO_RAW") bytes)"
# Marker for harness consumers
echo "$INITRD_OUT" >"$OUT_DIR/INITRD_PATH.txt"
ls -la "$INITRD_OUT" "$CPIO_RAW"
file "$INITRD_OUT" "$CPIO_RAW" 2>/dev/null || true
