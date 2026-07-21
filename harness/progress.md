# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | held |
| C5 double-run | held |
| C4 clean-room | held (realmode -m16; identity-map TU soft) |
| **C2** | amalgamation smoke path; testfixture/Redis open |
| **C1** | past stack + identity maps; **#UD @ 0x13cfc** after |

### C1 latest
- freestanding memops (rep movsb/stosb)
- SOFT `ident_map_64.c` → system freestanding gcc (logged)
- QEMU: early console; then #UD at low PC (not unmapped memcpy)
- Evidence: `{SCRATCH}/stage_c_kernel.log`, `qemu_serial_ident.txt`, `qemu_int_ident.log`

### Soft exceptions (audited)
1. `arch/x86/boot/*.c` + `-m16` → system gcc
2. `arch/x86/boot/compressed/ident_map_64.c` → system freestanding gcc
3. BP_* offset soft-fix script
4. link/compressed stubs, no-op objtool/sorttable

### blocked_reason
**C1:** no userspace; fault after identity maps (#UD @ 0x13cfc).  
**C2/C5:** open.  
**Goal NOT complete.**
