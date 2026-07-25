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

/* Soft asm operand leaks / broken immediates */
WEAK long b0;
WEAK long buf;
WEAK long filp;
WEAK long func;
WEAK long in_mask;
WEAK long user_mask;
WEAK long var;
WEAK long units_10;
WEAK long units_2;
WEAK long __head;
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
