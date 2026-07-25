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

/* === C1_X86_LINK_EXTRAS === */
/* Auto extras for freestanding x86 link — weak; real defs win. */
#ifdef __GNUC__
#define WEAK __attribute__((weak))
#else
#define WEAK
#endif
#define STUB0(n) WEAK long n(void) { return 0; }
#define STUB1(n) WEAK long n(long a) { (void)a; return 0; }

/* Soft freestanding calls acc_*; x86 stubs export ggcc_*. */
extern int ggcc_el0_run_busybox(void);
WEAK int acc_el0_run_busybox(void) { return ggcc_el0_run_busybox(); }

/* Initrd globals when BLK_DEV_INITRD off or TU not linked yet */
WEAK unsigned long initrd_start;
WEAK unsigned long initrd_end;
WEAK unsigned long phys_initrd_start;
WEAK unsigned long phys_initrd_size;

/* Built-in initramfs image bounds (empty image OK for -initrd QEMU path). */
__asm__(
	".weak __initramfs_start\n"
	".globl __initramfs_start\n"
	"__initramfs_start:\n"
	".weak __initramfs_end\n"
	".globl __initramfs_end\n"
	"__initramfs_end:\n"
	".weak __initramfs_size\n"
	".globl __initramfs_size\n"
	".section .rodata\n"
	"__initramfs_size:\n"
	"\t.quad 0\n"
	".previous\n"
);

/* Soft may drop C defs named __head / func / var — force via asm. */
__asm__(
	".weak __head\n"
	".globl __head\n"
	".data\n"
	"__head:\n"
	"\t.quad 0\n"
	".weak func\n"
	".globl func\n"
	"func:\n"
	"\t.quad 0\n"
	".weak var\n"
	".globl var\n"
	"var:\n"
	"\t.quad 0\n"
	".previous\n"
);

/* Soft asm operand leaks / broken immediates */
WEAK long b0;
WEAK long buf;
WEAK long filp;
WEAK long in_mask;
WEAK long user_mask;
WEAK long units_10;
WEAK long units_2;
WEAK long nop_func;
WEAK long vclocks_used;
WEAK long x2apic_max_apicid;
WEAK int sysctl_perf_event_paranoid;
WEAK long perf_swevent_enabled;
WEAK long perf_sched_events;
WEAK long __perf_regs;

STUB0(__SCK__perf_snapshot_branch_stack)
STUB0(__SCT__x86_pmu_add)
STUB0(__SCT__x86_pmu_assign)
STUB0(__SCT__x86_pmu_commit_scheduling)
STUB0(__SCT__x86_pmu_del)
STUB0(__SCT__x86_pmu_disable)
STUB0(__SCT__x86_pmu_disable_all)
STUB0(__SCT__x86_pmu_drain_pebs)
STUB0(__SCT__x86_pmu_enable)
STUB0(__SCT__x86_pmu_enable_all)
STUB0(__SCT__x86_pmu_filter)
STUB0(__SCT__x86_pmu_get_event_constraints)
STUB0(__SCT__x86_pmu_guest_get_msrs)
STUB0(__SCT__x86_pmu_handle_irq)
STUB0(__SCT__x86_pmu_limit_period)
STUB0(__SCT__x86_pmu_pebs_aliases)
STUB0(__SCT__x86_pmu_put_event_constraints)
STUB0(__SCT__x86_pmu_read)
STUB0(__SCT__x86_pmu_sched_task)
STUB0(__SCT__x86_pmu_schedule_events)
STUB0(__SCT__x86_pmu_set_period)
STUB0(__SCT__x86_pmu_start_scheduling)
STUB0(__SCT__x86_pmu_stop_scheduling)
STUB0(__SCT__x86_pmu_swap_task_ctx)
STUB0(__SCT__x86_pmu_update)
STUB0(___perf_sw_event)
STUB0(__p4d_alloc)
STUB0(__perf_event_task_sched_in)
STUB0(__perf_event_task_sched_out)
STUB0(__perf_sw_event)
STUB0(__x64_sys_brk)
STUB0(__x64_sys_munmap)
STUB0(cache_get_priv_group)
STUB0(dma_alloc_attrs)
STUB0(dma_free_attrs)
STUB0(fpregs_soft_get)
STUB0(fpregs_soft_set)
STUB0(is_swiotlb_allocated)
STUB0(modify_user_hw_breakpoint)
STUB0(perf_aux_output_begin)
STUB0(perf_aux_output_end)
STUB0(perf_aux_output_flag)
STUB0(perf_aux_output_skip)
STUB0(perf_bp_event)
STUB0(perf_callchain)
STUB0(perf_event_account_interrupt)
STUB0(perf_event_addr_filters_sync)
STUB0(perf_event_comm)
STUB0(perf_event_delayed_put)
STUB0(perf_event_exec)
STUB0(perf_event_exit_task)
STUB0(perf_event_fork)
STUB0(perf_event_free_task)
STUB0(perf_event_init)
STUB0(perf_event_init_task)
STUB0(perf_event_itrace_started)
STUB0(perf_event_namespaces)
STUB0(perf_event_output)
STUB0(perf_event_overflow)
STUB0(perf_event_sysfs_show)
STUB0(perf_event_task_disable)
STUB0(perf_event_task_enable)
STUB0(perf_event_task_tick)
STUB0(perf_event_text_poke)
STUB0(perf_event_update_userpage)
STUB0(perf_get_aux)
STUB0(perf_log_lost_samples)
STUB0(perf_output_begin)
STUB0(perf_output_copy_aux)
STUB0(perf_output_end)
STUB0(perf_output_sample)
STUB0(perf_pmu_disable)
STUB0(perf_pmu_enable)
STUB0(perf_pmu_register)
STUB0(perf_pmu_resched)
STUB0(perf_pmu_unregister)
STUB0(perf_prepare_header)
STUB0(perf_prepare_sample)
STUB0(perf_report_aux_output_id)
STUB0(perf_sample_event_took)
STUB0(perf_sched_cb_dec)
STUB0(perf_sched_cb_inc)
STUB0(register_user_hw_breakpoint)
STUB0(swiotlb_dev_init)
STUB0(swiotlb_exit)
STUB0(swiotlb_init)
STUB0(swiotlb_print_info)
STUB0(unregister_hw_breakpoint)
