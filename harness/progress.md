# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS (held; re-run after language churn) |
| C5 double-run | held |
| C4 clean-room | held (realmode -m16 soft exception) |
| **C2** | **SQLite amalgamation smoke PASS**; shell compiles on host; **no testfixture/Redis run yet** |
| **C1** | **bzImage linked + QEMU reaches 64-bit**; **page-fault cascade at ~0x100239** |

### C1 (updated 2026-07-21)
- **bzImage 296448 B** rebuilt with real `extract_kernel` / `decompress_kernel` bodies (keep-list, no PARSE_ALL hang)
- **vmlinux** re-linked after static-inline local-symbol fix (no multi-def)
- **compressed/vmlinux** links with `ggcc_comp_stubs.o` + builtin soft maps
- QEMU serial: `early console in setup code`
- QEMU enters long mode (`CR0=80050033`), then `#PF` at `pc=0x100239` / `CR2=0x100fce5f8` → `#DF`
- Evidence: `{SCRATCH}/stage_c_kernel.log`, `bzImage`, `qemu_full20.txt`, `qemu_int20.log`, `qemu_evidence.log`

### C2
- SQLite amalgamation + smoke main: **sqlite_smoke_ok** (prior)
- `shell.c` lex: multi-char soft; host `-S` OK (~126KB aarch64); x86_64 full shell path still heavy
- Full testfixture / Redis: open

### Language fixes this session
1. **static / static-inline → local only** (no `.globl`) — fixes kernel multi-def of header inlines
2. **`__builtin_unreachable` → `((void)0)`**; **`__builtin_memcpy/memset` → memcpy/memset**
3. Boot-critical keep list for decompress path
4. Compressed weak stubs object + Makefile hook
5. Soft multi-char char literals (SQLite shell)

### Soft exceptions (audited)
- `arch/x86/boot/*.c` + `-m16` → system gcc
- Soft stubs / `.weak` / link stubs / no-op objtool+sorttable
- `CONFIG_SECTION_MISMATCH_WARN_ONLY=y`

### blocked_reason
**C1:** 64-bit entry reached but early `#PF` before useful decompress serial; not userspace.  
**C2:** smoke only; need testfixture or Redis basic tests.  
**C5:** re-run after churn.  
**Goal NOT complete.**
