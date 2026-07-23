# Stage C1 — Linux 6.9 + QEMU (honest)

**Evidence:** `{SCRATCH}/stage_c_kernel.log` + `{SCRATCH}/qemu_boot.log` + `{SCRATCH}/c1_boot_marker`

## Status: **PASS** (2026-07-23 Image #110)

| Piece | Status |
|------|--------|
| Soft SYSCC | OFF (`GGCC_ALLOW_SOFT_SYSCC=0`) |
| Soft freestanding env | OFF (`GGCC_SOFT_FREESTANDING=0`) |
| Kernel freestanding | ON for kernel only (`GGCC_KERNEL_FREESTANDING=1`) |
| clean-room Image | YES (`arch/arm64/boot/Image` **#110**) |
| QEMU boot (Docker) | YES — `Linux version 6.9.0` + init/pid1 |
| EL0 binfmt_elf /init | NO (kernel-linked `ggcc_real_init_payload`) |
| Freestanding mid-boot helpers | language-gap hard keepers (not gcc-on-.c) |

## Serial (#110)
```
Linux version 6.9.0 … (ggcc-wrapper …) #110 SMP …
rest_init: freestanding (direct run_init_process)
Run /init as init process
ggcc-init: real userspace ELF running as pid1
working init: ggcc payload returned (pid1 handoff; no EL0 binfmt yet)
rest_init: park after run_init_process
```

## Stamp
`PASS_BOOT` in `{SCRATCH}/c1_boot_marker` (requires Linux version **and** init/pid1).
