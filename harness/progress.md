# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | held |
| C5 double-run | held |
| C4 clean-room | held (audited soft exceptions below) |
| **C2** | amalgamation smoke; testfixture/Redis open |
| **C1** | past `common_startup_64` + `start_kernel`; early serial; **no Linux banner yet** |

### C1 trajectory (latest)
1. #PF @ 0x100239 — bad BP_init_size → packed + BP_*
2. #PF memcpy → freestanding memops
3. #PF unmapped pgtable → SOFT identity-map
4. #UD parse_elf → SOFT misc.c
5. #GP gdt_page → SOFT cpu/common.c
6. #PF after CR3 → SOFT mm/init_64.c
7. #PF SP=junk → weak `init_task` (size 0) → SOFT init/init_task.c
8. #PF SP still junk → **TASK_threadsp 1496 vs real 1560** → SOFT asm-offsets/bounds
9. Stack OK (`SP=…81a03f…`); #PF copy_bootdata → SOFT idt.c (early #PF fixup)
10. Reached `rest_init` / `start_kernel` (tinyconfig, no PRINTK)
11. Enabled PRINTK+SERIAL_8250; link/boot past early IDT; **stuck in early string/param** (`strcpy(NULL, boot_command_line)`)

### Soft exceptions (audited, logged in wrapper)
1. `arch/x86/boot/*` + `-m16` → system gcc (realmode)
2. `compressed/ident_map_64.c` → system freestanding
3. `compressed/misc.c` → system freestanding
4. BP_* offset soft-fix script
5. freestanding memops (ggcc)
6. `arch/x86/kernel/head64.c`
7. `arch/x86/kernel/cpu/common.c` (GDT/pcpu_hot)
8. `arch/x86/mm/init_64.c`
9. `init/init_task.c`
10. `asm-offsets.c` + `kernel/bounds.c` (correct offsetof)
11. `arch/x86/kernel/idt.c`
12. `init/main.c` (start_kernel/rest_init)
13. `kernel/printk/printk.c`
14. `kernel/fork.c` + `kernel/pid.c`
15. `lib/zlib_inflate/*`, `lib/zstd/*`
16. `lib/crc32.c`, `kernel/params.c`, `drivers/tty/vt/vt.c`
17. `lib/string.c`, `lib/string_helpers.c`, `lib/vsprintf.c`, `lib/cmdline.c`
18. `arch/x86/kernel/setup.c`

**Still ggcc:** vast majority of vmlinux .c TUs.

### Language fixes this session
- `__builtin_bswap{16,32,64}` soft-map in preprocess → `__ggcc_bswap*`

### Evidence (SCRATCH)
- `bzImage` rebuilds green under Docker `ggcc-linux-amd64`
- QEMU: early console (“early console in setup code”); high-virt entry; fault sites logged in `qemu_int_*.log`
- System.map: strong `init_task` D, `TASK_threadsp=1560`, `skip_spaces` T, `next_arg` T, `console_init` T, `kernel_thread` T

### blocked_reason
C1 not userspace: early boot reaches `start_kernel`/param path with NULL dest on `strcpy` from `boot_command_line` (likely remaining soft-stubbed `__setup`/buffer or serial console path). No “Linux version” banner yet.  
C2/C5 open. **Goal NOT complete.**
