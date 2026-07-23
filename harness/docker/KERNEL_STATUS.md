# Stage C1 — Linux 6.9 + QEMU (honest)

**Evidence:** `{SCRATCH}/stage_c_kernel.log` + `qemu_c1_rebuild.log` / `qemu_c1_host.log`

## Status: **PARTIAL** (boot MET; not full PASS)

| Piece | Status |
|-------|--------|
| Soft SYSCC | OFF (`GGCC_ALLOW_SOFT_SYSCC=0`) |
| clean-room Image | YES (`arch/arm64/boot/Image` **#95**, rebuild make_ec=0) |
| QEMU boot (Docker) | YES — `Linux version 6.9.0` #95 + `ggcc-init` |
| QEMU boot (macOS host) | YES — same serial |
| EL0 binfmt_elf /init | NO (kernel-linked payload) |
| Freestanding soft mid-boot | 64 stubs (language gaps; **not** gcc-on-.c) |

## Serial (#95)
```
Linux version 6.9.0 … (ggcc-wrapper …) #95 SMP …
Run /init as init process
ggcc-init: real userspace ELF running as pid1
working init: ggcc payload returned (pid1 handoff; no EL0 binfmt yet)
```

## Not full C1 PASS until
Real EL0 VFS/binfmt_elf userspace init and/or fewer freestanding soft mid-boot helpers.
Bootable Image + QEMU is proven; goal-state "C1 BLOCKED / no Image" is **STALE**.
