/* Weak link stubs for ggcc x86_64 C1 — real defs win. Auto-extended for undefs. */
#ifdef __GNUC__
#define WEAK __attribute__((weak))
#else
#define WEAK
#endif

/* Soft-asm operand leak from pushf;pop %0 → pop x0 */
WEAK unsigned long x0;
WEAK unsigned long x1;

/* __builtin_choose_expr: ggcc drops C defs named __builtin_*; provide via asm. */
__asm__(
	".weak __builtin_choose_expr\n"
	".globl __builtin_choose_expr\n"
	"__builtin_choose_expr:\n"
	"\tmovq\t%rsi, %rax\n"
	"\tretq\n"
);

/* Builtins that ggcc sometimes emits as calls instead of folding */
WEAK long __builtin_constant_p(long x) { (void)x; return 0; }
WEAK long __builtin_object_size(const void *p, int t) { (void)p; (void)t; return -1; }
WEAK void __builtin_prefetch(const void *p, ...) { (void)p; }
WEAK void *__builtin_return_address(unsigned int n) { (void)n; return 0; }
WEAK void *__builtin_frame_address(unsigned int n) { (void)n; return 0; }
WEAK void *__builtin_extract_return_addr(void *p) { return p; }
WEAK void __builtin_va_start(void *a, ...) { (void)a; }
WEAK void __builtin_va_end(void *a) { (void)a; }
WEAK void __builtin_va_copy(void *d, void *s) { (void)d; (void)s; }

/* Build-bug / size sentinels — should be compile-time; stub so link proceeds */
WEAK void __bad_size_call_parameter(void) {}
WEAK void __xadd_wrong_size(void) {}
WEAK void __xchg_wrong_size(void) {}
WEAK void __cmpxchg_wrong_size(void) {}
WEAK void __bad_copy_from(void) {}
WEAK void __bad_copy_to(void) {}
WEAK void __bad_udelay(void) {}
WEAK void __get_user_bad(void) {}
WEAK void __put_user_bad(void) {}
WEAK void ____wrong_branch_error(void) {}
WEAK void GEN_UNARY_RMWcc_1(void) {}
WEAK void GEN_BINARY_RMWcc_1(void) {}

/* Early printk if kernel printk TU soft-skipped the freestanding keeper */
extern void ggcc_uart_nwrite(const void *buf, unsigned long n);
WEAK int _printk(const char *fmt, ...)
{
	const char *p = fmt ? fmt : "";
	unsigned long n = 0;
	while (p[n])
		n++;
	ggcc_uart_nwrite(p, n);
	return 0;
}

/* Syscall table holes — return -ENOSYS */
#define STUB_SYS(n) WEAK long n(void) { return -38; }
STUB_SYS(__x64_sys_ni_syscall)
STUB_SYS(__x64_sys_fork)
STUB_SYS(__x64_sys_vfork)
STUB_SYS(__x64_sys_getpid)
STUB_SYS(__x64_sys_getppid)
STUB_SYS(__x64_sys_gettid)
STUB_SYS(__x64_sys_getuid)
STUB_SYS(__x64_sys_geteuid)
STUB_SYS(__x64_sys_getgid)
STUB_SYS(__x64_sys_getegid)
STUB_SYS(__x64_sys_getpgrp)
STUB_SYS(__x64_sys_setsid)
STUB_SYS(__x64_sys_pause)
STUB_SYS(__x64_sys_sched_yield)
STUB_SYS(__x64_sys_sync)
STUB_SYS(__x64_sys_vhangup)
STUB_SYS(__x64_sys_rt_sigreturn)
STUB_SYS(__x64_sys_restart_syscall)
STUB_SYS(x32_sys_call)

#define STUB0(n) WEAK long n(void) { return 0; }
STUB0(arch_syscall_is_vdso_sigreturn)
STUB0(convert_from_fxsr)
STUB0(convert_to_fxsr)
STUB0(copy_mc_fragile)
STUB0(cpu_show_meltdown)
STUB0(cpu_show_spec_store_bypass)
STUB0(cpu_show_spectre_v1)
STUB0(cpu_show_spectre_v2)
STUB0(dma_alloc_from_pool)
STUB0(dma_free_from_pool)
STUB0(do_set_thread_area)
STUB0(efi_crash_gracefully_on_page_fault)
STUB0(efi_init)
STUB0(efi_mem_attributes)
STUB0(efi_mem_type)
STUB0(efi_memblock_x86_reserve_range)
STUB0(fixup_vdso_exception)
STUB0(handle_vc_boot_ghcb)
STUB0(huge_pud_set_accessed)
STUB0(ia32_setup_frame)
STUB0(ia32_setup_rt_frame)
STUB0(x32_setup_rt_frame)
STUB0(int3_magic)
STUB0(intel_pmu_nhm_workaround)
STUB0(irq_do_set_affinity)
STUB0(lock_is_held)
STUB0(lockdep_is_held)
STUB0(microcode_nmi_handler)
STUB0(microcode_offline_nmi_handler)
STUB0(pmdp_clear_flush_young)
STUB0(ptdump_walk_pgd_level_checkwx)
STUB0(sched_clock_noinstr)
STUB0(vc_boot_ghcb)
STUB0(vc_no_ghcb)
STUB0(x86_pmu_del)
STUB0(x86_schedule_events)
STUB0(amd_pmu_add_event)
STUB0(amd_pmu_del_event)
STUB0(__handle_irq)
STUB0(__static_call_return)
STUB0(rust_fmt_argument)

/* Static-call / alternative sites — empty ranges */
WEAK char __cfi_sites;
WEAK char __cfi_sites_end;
WEAK char __retpoline_sites;
WEAK char __retpoline_sites_end;
WEAK char __return_sites;
WEAK char __return_sites_end;

/* Static-call trampoline slots (PERF/APIC leftovers) */
#define STUB_SCT(n) WEAK long n(void) { return 0; }
STUB_SCT(__SCT__amd_pmu_branch_add)
STUB_SCT(__SCT__amd_pmu_branch_del)
STUB_SCT(__SCT__amd_pmu_branch_hw_config)
STUB_SCT(__SCT__amd_pmu_branch_reset)
STUB_SCT(__SCT__amd_pmu_test_overflow)
STUB_SCT(__SCT__apic_call_eoi)
STUB_SCT(__SCT__apic_call_icr_read)
STUB_SCT(__SCT__apic_call_icr_write)
STUB_SCT(__SCT__apic_call_native_eoi)
STUB_SCT(__SCT__apic_call_read)
STUB_SCT(__SCT__apic_call_send_IPI)
STUB_SCT(__SCT__apic_call_send_IPI_all)
STUB_SCT(__SCT__apic_call_send_IPI_allbutself)
STUB_SCT(__SCT__apic_call_send_IPI_mask)
STUB_SCT(__SCT__apic_call_send_IPI_mask_allbutself)
STUB_SCT(__SCT__apic_call_send_IPI_self)
STUB_SCT(__SCT__apic_call_wait_icr_idle)
STUB_SCT(__SCT__apic_call_wakeup_secondary_cpu)
STUB_SCT(__SCT__apic_call_wakeup_secondary_cpu_64)
STUB_SCT(__SCT__apic_call_write)
STUB_SCT(__SCT__intel_pmu_set_topdown_event_period)
STUB_SCT(__SCT__intel_pmu_update_topdown_event)
STUB_SCT(__SCT__perf_snapshot_branch_stack)
STUB_SCT(__SCT__x86_idle)

/* module_param ops */
WEAK char param_ops__Bool;
WEAK char param_ops_ulong;

/* RCU lockdep maps */
WEAK char rcu_bh_lock_map;
WEAK char rcu_callback_map;
WEAK char rcu_lock_map;
WEAK char rcu_sched_lock_map;
