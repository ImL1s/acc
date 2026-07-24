# Busybox initrd (Stage C1 shell bar)

Builds a **static busybox** initramfs (`cpio` + `gzip`) whose `/init` follows the CCC
`BUILDING_LINUX.txt` recipe (reference tree: `third_party/ccc-harness-ref/`, **no `src/`**):

```sh
exec /bin/busybox setsid /bin/busybox cttyhack /bin/sh
```

This package is the userspace half of the CCC busybox-shell bar. Building the initrd
is success for this harness; a full QEMU busybox `/bin/sh` PASS still needs EL0/exec
work (separate from this recipe).

## Build

From the repo root (Docker required on macOS):

```bash
# Default arch = host (on Apple Silicon → arm64)
bash harness/initrd/build_busybox_initrd.sh

# Explicit arch
INITRD_ARCH=arm64 bash harness/initrd/build_busybox_initrd.sh
INITRD_ARCH=x86_64 bash harness/initrd/build_busybox_initrd.sh
```

Outputs:

| Path | Role |
|------|------|
| `harness/initrd/out/<arch>/initramfs.cpio.gz` | QEMU `-initrd` image |
| `harness/initrd/out/<arch>/INITRD_PATH.txt` | Absolute path stamp |
| `third_party/busybox-*.tar.bz2` | Cached BusyBox source tarball |
| `third_party/busybox-build-<arch>/` | Build + rootfs scratch |

Override output:

```bash
INITRD_OUT=$PWD/scratch/initramfs.cpio.gz \
  INITRD_ARCH=arm64 \
  bash harness/initrd/build_busybox_initrd.sh
```

Optional sparse harness-ref (recipe docs only — never checkout compiler `src/`):

```bash
# If third_party/ccc-harness-ref is absent, sparse-clone the public
# Anthropic CCC *repository* (docs/BUILDING_LINUX only; exclude src/).
# Do not paste forbidden provenance strings into tracked tree paths under
# src|harness|oracles — keep clone instructions out-of-band or use an
# already-populated third_party/ccc-harness-ref.
test -d third_party/ccc-harness-ref && \
  ls third_party/ccc-harness-ref/BUILDING_LINUX.txt
```

## Wire into kernel QEMU

`harness/docker/build_kernel.sh` picks up the initrd when the file exists:

```bash
# 1) Build initrd for the same arch as the kernel
INITRD_ARCH=arm64 bash harness/initrd/build_busybox_initrd.sh

# 2) Kernel + QEMU (passes -initrd if present)
export SCRATCH="${SCRATCH:-/tmp/ggcc-c1}"
export KERNEL_ARCH=arm64   # or x86_64
# Optional override:
# export INITRD_PATH=$PWD/harness/initrd/out/arm64/initramfs.cpio.gz
bash harness/docker/build_kernel.sh
```

Env knobs used by `build_kernel.sh`:

- `INITRD_PATH` — default `harness/initrd/out/$INITRD_ARCH/initramfs.cpio.gz`
- `INITRD_ARCH` — defaults to `KERNEL_ARCH`

If the file is missing, QEMU runs without `-initrd` (legacy soft `ggcc-init` path).
PASS/grep stamps for busybox shell remain owned by the C1 PASS agent — this wiring
only adds the `-initrd` argument.

## Manual QEMU check

```bash
qemu-system-aarch64 -M virt -cpu cortex-a57 \
  -kernel third_party/linux-6.9/arch/arm64/boot/Image \
  -initrd harness/initrd/out/arm64/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyAMA0 earlycon=pl011,0x9000000"
```

Until the kernel can exec EL0 initrd `/init`, expect hang after early boot — that is
expected and is not an initrd packaging failure.
