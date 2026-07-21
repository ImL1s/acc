# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS (held; re-run after language churn) |
| C5 double-run | held |
| C4 clean-room | held (realmode -m16 soft exception) |
| **C2** | **SQLite amalgamation smoke PASS**; **no testfixture** |
| **C1** | **bzImage + QEMU early console**; **triple-fault in 64-bit** |

### C1
- bzImage ~239KB, vmlinux ~38MB, System.map
- QEMU serial: `early console in setup code` then triple fault
- Evidence: `{SCRATCH}/stage_c_kernel.log`, `bzImage`, `qemu_boot_serial.log`

### C2
- SQLite amalgamation + smoke main: **sqlite_smoke_ok** (ggcc compile, system as/ld)
- Evidence: `{SCRATCH}/stage_c_projects.log`
- Full testfixture / Redis still open

### Soft exceptions
- arch/x86/boot/*.c + -m16 → system gcc (16-bit realmode only)
- Soft stubs / .weak / link stubs for kernel soft path

### blocked_reason
**C1:** not full boot (setup ok, 64-bit faults).  
**C2:** smoke only; need testfixture or Redis.  
**C5:** re-run after churn.  
**Goal NOT complete.**
