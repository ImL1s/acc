# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS (held; re-run after language churn) |
| C5 double-run | held |
| C4 clean-room | held (realmode -m16; BP offset soft-fix) |
| **C2** | SQLite amalgamation smoke PASS; shell/testfixture/Redis open |
| **C1** | bzImage + QEMU early console + **64-bit past startup_64 stack**; later #PF |

### C1 (2026-07-21 evening)
- Root cause: wrong `BP_*` offsets (nested packed structs) → bad stack in `startup_64`
- Fix: `packed` attribute in lexer/parser/codegen + `fix_asm_offsets_bp.sh` (UAPI values)
- head_64 now: `mov 0x260(%rsi)` (BP_init_size=608) not `0x278`
- QEMU: past `0x100239` early PF → now PF at `pc≈0x2382142` with CR3 set
- Evidence: `{SCRATCH}/stage_c_kernel.log`, `qemu_serial_bp.txt`, `qemu_int_bp.log`, `bzImage`

### Soft exceptions
- realmode `-m16` system gcc for `arch/x86/boot/*`
- BP_* offset correction post asm-offsets generation
- compressed weak stubs, link stubs, no-op objtool/sorttable

### blocked_reason
**C1:** still no userspace; fault in later identity/decompress path.  
**C2:** need testfixture or Redis.  
**C5:** re-run pending.  
**Goal NOT complete.**
