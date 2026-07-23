# Progress (NO-DOWNGRADE) — honest status

## Goal: **NOT complete**

| Gate | Status | Notes |
|------|--------|-------|
| **A** | PASS | |
| **B** | PASS | |
| **C1** | **PARTIAL** | Linux version on QEMU with hard `_printk`; mid-boot + early setup_arch soft stubs now **opt-in only** (`GGCC_SOFT_FREESTANDING=0`). Full rebuild + QEMU pending Docker. |
| **C2** | **STRONG** | SQLite full testfixture ~99.94% (17914 tests); Redis basic MET |
| **C3/C5** | PASS | |
| **C4** | PASS policy | soft SYSCC off; soft freestanding gated (expanded list) |

### Latest code change
- Expanded `is_soft_freestanding_name` to cover always-on early stubs:
  `map_mem`, `bootmem_init`, `create_idmap`, `setup_machine_fdt`,
  `kasan_init_sw_tags`, `smp_setup_processor_id`, schedule/completion, …
- Unit test asserts `map_mem` real body when soft freestanding off
- Hard keepers: `_printk`, `create_init_idmap`, tpidr helpers, unaligned load

### blocked_reason
```
C1: Docker daemon unavailable this turn — cannot rebuild Image / re-run QEMU.
    Code gates early soft stubs; need Docker remake of setup_arch path objects
    then QEMU until init/pid1.
```

### Next
1. Docker up → remake kernel objects under SOFT_FREESTANDING=0 → QEMU
2. Fix real-body failures exposed when soft stubs are off
3. Stamp PASS_BOOT only with init/pid1 evidence

Updated: 2026-07-23T01:48:00Z
