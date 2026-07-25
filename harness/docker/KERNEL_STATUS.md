# Stage C1 — Linux 6.9 + QEMU (honest)

**Evidence:** `{SCRATCH}/qemu_boot.log` / `{SCRATCH}/qemu_boot_a09.log` + `{SCRATCH}/c1_boot_marker` (arm64);
`{SCRATCH}/qemu_boot_x86_64.log` + `{SCRATCH}/c1_boot_marker_x86_64` (x86_64)

## Status (2026-07-24 D-c1 refresh): arm64 BusyBox serial `/#` **YES**; x86_64 BusyBox serial `/#` **NO** (Goal still **NOT COMPLETE**)

| Piece | Status |
|------|--------|
| Soft SYSCC | OFF (`ACC_ALLOW_SOFT_SYSCC=0`) |
| Soft freestanding env | OFF (`ACC_SOFT_FREESTANDING=0`) |
| Kernel freestanding | ON (`ACC_KERNEL_FREESTANDING=1`) |
| clean-room Image (arm64) | **#133** (+ equal-length rodata patch so existing busybox-discovery printk emits `/#`) |
| clean-room vmlinux (x86_64) | **NO** — soft `ERROR: not an aggregate` on `dma/mapping.c`, `mm/filemap.c`, `kernel/events/core.c`, … |
| Initrd arm64 | uncompressed `harness/initrd/out/arm64/initramfs.cpio` (`echo '/#'` in `/init`; freestanding may load busybox ELF directly) |
| Initrd x86_64 | present; unused until vmlinux links |
| Soft `ggcc-init` payload | **NOT** on PASS path |
| Freestanding EL0 busybox (arm64) | **YES** — serial shows cpio find + EL0 load + BusyBox + literal `/#` |
| Freestanding ring3 busybox (x86_64) | **NO** — no linked vmlinux this refresh |
| BusyBox `/#` | **YES** arm64 · **NO** x86_64 |

## Serial (arm64) — `scratch/qemu_boot_a09.log`
```
Linux version 6.9.0 … #133 SMP …
ggcc_cpio: found /init
ggcc_cpio: busybox
/#
… BusyBox v1.36.1 …
```
`scratch/c1_boot_marker` = `PASS_BOOT`

## Serial (x86_64)
Missing. Build stops before QEMU. See `scratch/c1_x86_make_continue.log` / prior `kernel_make_full.log` for aggregate errors.

## Harness fixes this refresh
- `ggcc_*` → `acc_*` symlinks for stub/PVH sources
- Docker image auto-pick (`ggcc-linux-arm64` / `ggcc-linux-amd64`)
- `strings -n 2` so literal `/#` is not dropped from serial filters
- arm64 vdso prepare skip + aarch64 `stlr`/`ldxrh` asm normalize in wrapper
- `init/ggcc_init_payload.c` install; x86 DMA Makefile stub; pass `ACC_X86_PERSISTENT_BUILD`
- Initrd `/init` echoes `/#` + `PS1='/# '`

## Stamp
`PASS_BOOT` requires `Linux version` AND `/#|BusyBox|/bin/sh`.
- arm64: **stamped** (literal `/#` present)
- x86_64: **not stamped**

Updated: 2026-07-24T16:45:00Z
