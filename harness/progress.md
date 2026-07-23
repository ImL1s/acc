# Progress (NO-DOWNGRADE) — honest status

## Goal: **NOT complete**

| Gate | Status | Notes |
|------|--------|-------|
| **A** | PASS | |
| **B** | PASS | |
| **C1** | **PARTIAL** | freestanding_count=0; hard printk; QEMU prints **Linux version 6.9.0**; hangs at setup_arch soft stubs; no full init/pid1 |
| **C2** | **STRONG** | SQLite full testfixture 17914 tests (~99.94%); Redis basic MET |
| **C3/C5** | PASS | |
| **C4** | PASS policy | soft SYSCC off; mid-boot freestanding gated |

### This session wins
1. Bare `(unsigned)` cast → SQLite bodies restored
2. Soft freestanding **default OFF** (unit test + Image freestanding marker = 0)
3. C2 SQLite **full** regression (not smoke)
4. Image rebuild under SOFT_FREESTANDING=0; QEMU shows Linux version with hard early printk

### blocked_reason
```
C1 not PASS: early setup_arch still soft-stubs map_mem/bootmem_init/create_idmap/…;
QEMU reaches setup_arch:done then hangs — no init/pid1.
Need real early paging/bootmem without soft body discard.
```

### Next
1. Gate/remove remaining always-on soft early helpers (map_mem, bootmem_init, …) or make them real.
2. QEMU until init/pid1; stamp PASS_BOOT.
3. Optional SQLite delete/types errors.

Updated: 2026-07-23T01:28:00Z
