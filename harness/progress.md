# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | held |
| C5 double-run | held |
| C4 clean-room | held (audited soft exceptions below) |
| **C2** | amalgamation smoke; testfixture/Redis open |
| **C1** | **#PF at 0x10000ca** (decompressed entry region) — past identity maps |

### C1 trajectory
1. #PF @ 0x100239 — bad BP_init_size → fixed (packed + BP_*)
2. #PF in memcpy → freestanding memops (rep movsb)
3. #PF unmapped pgtable → SOFT identity-map TU (system freestanding)
4. #UD @ 0x13cfc — soft parse_elf → SOFT misc.c extract/decompress
5. **now #PF @ 0x10000ca** — jumped toward LOAD_PHYSICAL_ADDR; map/entry still wrong

### Soft exceptions (audited, logged)
1. `arch/x86/boot/*` + `-m16` → system gcc (realmode)
2. `compressed/ident_map_64.c` → system freestanding gcc
3. `compressed/misc.c` → system freestanding gcc (parse_elf/extract)
4. BP_* offset soft-fix script
5. ggcc freestanding memops for string.c

**Still ggcc:** vast majority of vmlinux .c, string.c memops, codegen for rest.

### blocked_reason
C1 not userspace. Next: map/entry at 0x1000000 after decompress.  
C2/C5 open. **Goal NOT complete.**
