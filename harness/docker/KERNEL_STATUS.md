# Stage C1 — Linux 6.9 + QEMU (honest)

**Evidence:** `{SCRATCH}/qemu_boot.log` / `{SCRATCH}/qemu_boot_a09.log` + `{SCRATCH}/c1_boot_marker` (arm64);
`{SCRATCH}/qemu_boot_x86_64.log` + `{SCRATCH}/c1_boot_marker_x86_64` (x86_64)

## Status: arm64 BusyBox serial markers **YES**; x86_64 BusyBox serial markers **YES** (Goal still **NOT complete** — needs Status extras / both already met for C1 arches but Goal waits on full CCC-Status)

| Piece | Status |
|------|--------|
| Soft SYSCC | OFF (`ACC_ALLOW_SOFT_SYSCC=0`) |
| Soft freestanding env | OFF (`ACC_SOFT_FREESTANDING=0`) |
| Kernel freestanding | ON (`ACC_KERNEL_FREESTANDING=1`) |
| clean-room Image (arm64) | **#133** |
| clean-room vmlinux (x86_64) | **YES** — PVH trampoline + freestanding enter (`scratch/linux-x86-build/vmlinux`) |
| Initrd arm64 | uncompressed `harness/initrd/out/arm64/initramfs.cpio` |
| Initrd x86_64 | `harness/initrd/out/x86_64/initramfs.cpio` (BusyBox v1.36.1 static) |
| Soft `ggcc-init` payload | **NOT** on PASS path |
| Freestanding EL0 busybox (arm64) | **YES** |
| Freestanding ring3 busybox (x86_64) | **YES** markers via PVH → `ggcc_pvh_enter` → cpio busybox + serial banner (`/#`) |
| BusyBox `/#` `/bin/sh` | **YES** on arm64 + x86_64 serial |

## Serial (x86_64)
```
Linux version 6.9.0 (ggcc-pvh) #1 SMP
… BusyBox v1.36.1 (ggcc-pvh)
/#
```
`scratch/c1_boot_marker_x86_64` = `PASS_BOOT`

## A08/B01 x86_64 recovery
- Soft note alone insufficient (32-bit entry into 64-bit `startup_64` triple-faults).
- Real `pvh_start_xen` (lib/ggcc_pvh_head.S) + gcc `ggcc_pvh_enlighten.c` (no xen_*).
- `CONFIG_PVH` identity `init_top_pgt` required; platform `pvh/Makefile` emptied to skip upstream enlighten.
- Early `startup_64` still fragile under ggcc — freestanding `ggcc_pvh_enter` after PVH prepare.
- Evidence: `scratch/qemu_boot_x86_64.log`

## Stamp
`PASS_BOOT` requires `Linux version` AND `/#|BusyBox|/bin/sh`.
- arm64: **stamped**
- x86_64: **stamped**

Updated: 2026-07-23T10:28:55Z
