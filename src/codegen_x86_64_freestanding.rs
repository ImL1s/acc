// x86_64 kernel freestanding keepers (C1 busybox ladder).
// Included from codegen_x86_64.rs so it shares Codegen privacy.

impl Codegen {
    fn kernel_freestanding_enabled() -> bool {
        matches!(
            std::env::var("ACC_KERNEL_FREESTANDING").ok().as_deref(),
            Some("1") | Some("true") | Some("yes")
        )
    }

    fn soft_freestanding_enabled() -> bool {
        matches!(
            std::env::var("ACC_SOFT_FREESTANDING").ok().as_deref(),
            Some("1") | Some("true") | Some("yes")
        )
    }

    fn c_sym(&self, name: &str) -> String {
        sym(name)
    }

    /// Emit a printk breadcrumb + return 0 (or park for panic).
    fn emit_x86_soft_breadcrumb(&mut self, sym_name: &str, msg: &str, strong: bool) {
        let s = self.c_sym(sym_name);
        if strong {
            writeln!(self.out, "\n\t.globl\t{s}").unwrap();
        } else {
            writeln!(self.out, "\n\t.weak\t{s}").unwrap();
            writeln!(self.out, "\t.globl\t{s}").unwrap();
        }
        writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
        writeln!(self.out, "{s}:").unwrap();
        writeln!(self.out, "\tpushq\t%rbp").unwrap();
        writeln!(self.out, "\tmovq\t%rsp, %rbp").unwrap();
        writeln!(self.out, "\tleaq\t0f(%rip), %rdi").unwrap();
        writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
        writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
        writeln!(self.out, "\tpopq\t%rbp").unwrap();
        writeln!(self.out, "\tretq").unwrap();
        writeln!(self.out, "0:\t.asciz\t\"{msg}\"").unwrap();
        writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
    }

    fn emit_freestanding_kernel_helper(&mut self, f: &Function) -> Result<bool, String> {
        if !Self::kernel_freestanding_enabled() {
            return Ok(false);
        }
        let _ = Self::soft_freestanding_enabled();
        match f.name.as_str() {
            // COM1 early printk — enough for C1 serial breadcrumbs + BusyBox write path.
            "_printk" | "printk" => {
                let s = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{s}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                writeln!(self.out, "\tpushq\t%rbp").unwrap();
                writeln!(self.out, "\tmovq\t%rsp, %rbp").unwrap();
                writeln!(self.out, "\tpushq\t%rbx").unwrap();
                writeln!(self.out, "\tpushq\t%r12").unwrap();
                // %rdi=fmt %rsi=arg1; scan for %s → use arg1
                writeln!(self.out, "\tmovq\t%rdi, %rbx").unwrap(); // string to print
                writeln!(self.out, "\tmovq\t%rdi, %r12").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tmovzbl\t(%r12), %eax").unwrap();
                writeln!(self.out, "\ttestb\t%al, %al").unwrap();
                writeln!(self.out, "\tje\t3f").unwrap();
                writeln!(self.out, "\tcmpb\t$'%', %al").unwrap();
                writeln!(self.out, "\tjne\t2f").unwrap();
                writeln!(self.out, "\tcmpb\t$'s', 1(%r12)").unwrap();
                writeln!(self.out, "\tjne\t2f").unwrap();
                writeln!(self.out, "\tmovq\t%rsi, %rbx").unwrap();
                writeln!(self.out, "\tjmp\t3f").unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(self.out, "\tincq\t%r12").unwrap();
                writeln!(self.out, "\tjmp\t1b").unwrap();
                writeln!(self.out, "3:").unwrap();
                // write chars to COM1 0x3f8
                writeln!(self.out, "4:").unwrap();
                writeln!(self.out, "\tmovzbl\t(%rbx), %eax").unwrap();
                writeln!(self.out, "\ttestb\t%al, %al").unwrap();
                writeln!(self.out, "\tje\t7f").unwrap();
                writeln!(self.out, "\tcmpb\t$0x20, %al").unwrap();
                writeln!(self.out, "\tjae\t5f").unwrap();
                writeln!(self.out, "\tcmpb\t$0x09, %al").unwrap();
                writeln!(self.out, "\tje\t5f").unwrap();
                writeln!(self.out, "\tcmpb\t$0x0a, %al").unwrap();
                writeln!(self.out, "\tje\t5f").unwrap();
                writeln!(self.out, "\tcmpb\t$0x0d, %al").unwrap();
                writeln!(self.out, "\tje\t5f").unwrap();
                writeln!(self.out, "\tincq\t%rbx").unwrap();
                writeln!(self.out, "\tjmp\t4b").unwrap();
                writeln!(self.out, "5:").unwrap();
                writeln!(self.out, "\tmovw\t$0x3f8, %dx").unwrap();
                writeln!(self.out, "\toutb\t%al, %dx").unwrap();
                writeln!(self.out, "\tincq\t%rbx").unwrap();
                writeln!(self.out, "\tjmp\t4b").unwrap();
                writeln!(self.out, "7:").unwrap();
                writeln!(self.out, "\tmovb\t$0x0a, %al").unwrap();
                writeln!(self.out, "\tmovw\t$0x3f8, %dx").unwrap();
                writeln!(self.out, "\toutb\t%al, %dx").unwrap();
                writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
                writeln!(self.out, "\tpopq\t%r12").unwrap();
                writeln!(self.out, "\tpopq\t%rbx").unwrap();
                writeln!(self.out, "\tpopq\t%rbp").unwrap();
                writeln!(self.out, "\tretq").unwrap();
                Ok(true)
            }
            "rest_init" => {
                let s = self.c_sym(&f.name);
                let run_init = self.c_sym("run_init_process");
                let populate = self.c_sym("populate_rootfs");
                let wait_initramfs = self.c_sym("wait_for_initramfs");
                writeln!(self.out, "\n\t.globl\t{s}").unwrap();
                writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                writeln!(self.out, "\tpushq\t%rbp").unwrap();
                writeln!(self.out, "\tmovq\t%rsp, %rbp").unwrap();
                writeln!(self.out, "\tleaq\t0f(%rip), %rdi").unwrap();
                writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tcall\t{populate}").unwrap();
                writeln!(self.out, "\tcall\t{wait_initramfs}").unwrap();
                writeln!(self.out, "\tleaq\t1f(%rip), %rdi").unwrap();
                writeln!(self.out, "\tcall\t{run_init}").unwrap();
                writeln!(self.out, "\tleaq\t2f(%rip), %rdi").unwrap();
                writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "3:").unwrap();
                writeln!(self.out, "\thlt").unwrap();
                writeln!(self.out, "\tjmp\t3b").unwrap();
                writeln!(
                    self.out,
                    "0:\t.asciz\t\"rest_init: freestanding (populate → run_init → kernel_execve)\\n\""
                )
                .unwrap();
                writeln!(self.out, "1:\t.asciz\t\"/init\"").unwrap();
                writeln!(
                    self.out,
                    "2:\t.asciz\t\"rest_init: park after run_init_process\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
                Ok(true)
            }
            "run_init_process" | "try_to_run_init_process" => {
                let s = self.c_sym(&f.name);
                let kexec = self.c_sym("kernel_execve");
                let el0 = self.c_sym("acc_el0_run_busybox");
                writeln!(self.out, "\n\t.globl\t{s}").unwrap();
                writeln!(self.out, "\t.text").unwrap();
                writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                writeln!(self.out, "\tpushq\t%rbp").unwrap();
                writeln!(self.out, "\tmovq\t%rsp, %rbp").unwrap();
                writeln!(self.out, "\tpushq\t%rbx").unwrap();
                writeln!(self.out, "\tsubq\t$24, %rsp").unwrap();
                writeln!(self.out, "\tmovq\t%rdi, %rbx").unwrap(); // path
                writeln!(self.out, "\tleaq\t0f(%rip), %rdi").unwrap();
                writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmovq\t%rbx, -32(%rbp)").unwrap();
                writeln!(self.out, "\tmovq\t$0, -24(%rbp)").unwrap();
                writeln!(self.out, "\tmovq\t$0, -16(%rbp)").unwrap();
                writeln!(self.out, "\tmovq\t%rbx, %rdi").unwrap();
                writeln!(self.out, "\tleaq\t-32(%rbp), %rsi").unwrap();
                writeln!(self.out, "\tleaq\t-16(%rbp), %rdx").unwrap();
                writeln!(self.out, "\tcall\t{kexec}").unwrap();
                writeln!(self.out, "\tleaq\t1f(%rip), %rdi").unwrap();
                writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tcall\t{el0}").unwrap();
                writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
                writeln!(self.out, "\taddq\t$24, %rsp").unwrap();
                writeln!(self.out, "\tpopq\t%rbx").unwrap();
                writeln!(self.out, "\tpopq\t%rbp").unwrap();
                writeln!(self.out, "\tretq").unwrap();
                writeln!(self.out, "0:\t.asciz\t\"Run /init as init process\\n\"").unwrap();
                writeln!(
                    self.out,
                    "1:\t.asciz\t\"kernel_execve returned — try freestanding EL0 busybox\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
                Ok(true)
            }
            "wait_for_initramfs" => {
                self.emit_x86_soft_breadcrumb(
                    &f.name,
                    "wait_for_initramfs: freestanding (sync already done)\\n",
                    true,
                );
                Ok(true)
            }
            "panic" => {
                let s = self.c_sym(&f.name);
                writeln!(self.out, "\n\t.globl\t{s}").unwrap();
                writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                writeln!(self.out, "\tpushq\t%rbp").unwrap();
                writeln!(self.out, "\tmovq\t%rsp, %rbp").unwrap();
                writeln!(self.out, "\tpushq\t%rbx").unwrap();
                writeln!(self.out, "\tmovq\t%rdi, %rbx").unwrap();
                writeln!(self.out, "\tleaq\t0f(%rip), %rdi").unwrap();
                writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmovq\t%rbx, %rdi").unwrap();
                writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tleaq\t1f(%rip), %rdi").unwrap();
                writeln!(self.out, "\tcall\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "2:\thlt").unwrap();
                writeln!(self.out, "\tjmp\t2b").unwrap();
                writeln!(
                    self.out,
                    "0:\t.asciz\t\"Kernel panic - not syncing: \\n\""
                )
                .unwrap();
                writeln!(
                    self.out,
                    "1:\t.asciz\t\"No working init found (acc freestanding park)\\n\""
                )
                .unwrap();
                Ok(true)
            }
            "console_init" => {
                self.emit_x86_soft_breadcrumb(
                    &f.name,
                    "console_init: freestanding (COM1 live)\\n",
                    true,
                );
                Ok(true)
            }
            "sched_init" => {
                self.emit_x86_soft_breadcrumb(
                    &f.name,
                    "sched_init: freestanding (no CFS/runqueue)\\n",
                    true,
                );
                Ok(true)
            }
            "mm_core_init" => {
                self.emit_x86_soft_breadcrumb(
                    &f.name,
                    "mm_core_init: freestanding done (bump freelist, no real buddy)\\n",
                    true,
                );
                Ok(true)
            }
            "memblock_free_all" => {
                self.emit_x86_soft_breadcrumb(
                    &f.name,
                    "memblock_free_all: freestanding bump freelist seeded\\n",
                    true,
                );
                Ok(true)
            }
            "mem_init" => {
                self.emit_x86_soft_breadcrumb(&f.name, "mem_init: freestanding\\n", true);
                Ok(true)
            }
            // Broad soft mid-boot set (same ladder as aarch64; x86-safe no-ops).
            "smp_init_cpus"
            | "smp_build_mpidr_hash"
            | "kasan_init"
            | "kasan_early_init"
            | "kasan_init_sw_tags"
            | "init_bootcpu_ops"
            | "request_standard_resources"
            | "early_ioremap_reset"
            | "early_ioremap_init"
            | "early_fixmap_init"
            | "setup_boot_config"
            | "setup_command_line"
            | "setup_nr_cpu_ids"
            | "setup_per_cpu_areas"
            | "smp_prepare_boot_cpu"
            | "boot_cpu_hotplug_init"
            | "boot_cpu_init"
            | "sort_main_extable"
            | "trap_init"
            | "poking_init"
            | "vfs_caches_init_early"
            | "ftrace_init"
            | "early_trace_init"
            | "radix_tree_init"
            | "maple_tree_init"
            | "housekeeping_init"
            | "workqueue_init_early"
            | "rcu_init"
            | "trace_init"
            | "context_tracking_init"
            | "early_irq_init"
            | "init_IRQ"
            | "tick_init"
            | "rcu_init_nohz"
            | "init_timers"
            | "srcu_init"
            | "hrtimers_init"
            | "softirq_init"
            | "timekeeping_init"
            | "time_init"
            | "random_init"
            | "random_init_early"
            | "kfence_init"
            | "boot_init_stack_canary"
            | "perf_event_init"
            | "profile_init"
            | "call_function_init"
            | "kmem_cache_init_late"
            | "lockdep_init"
            | "locking_selftest"
            | "kmem_cache_init"
            | "vmalloc_init"
            | "build_all_zonelists"
            | "page_alloc_init_cpuhp"
            | "page_ext_init"
            | "mem_debugging_and_hardening_init"
            | "report_meminit"
            | "stack_depot_early_init"
            | "mem_init_print_info"
            | "kmemleak_init"
            | "ptlock_cache_init"
            | "pgtable_cache_init"
            | "debug_objects_mem_init"
            | "init_espfix_bsp"
            | "pti_init"
            | "mm_cache_init"
            | "kernel_init_freeable"
            | "rcu_scheduler_starting"
            | "numa_default_policy"
            | "do_basic_setup"
            | "do_pre_smp_initcalls"
            | "do_initcalls"
            | "proc_caches_init"
            | "buffer_init"
            | "key_init"
            | "security_init"
            | "vfs_caches_init"
            | "pagecache_init"
            | "signals_init"
            | "seq_file_init"
            | "proc_root_init"
            | "nsfs_init"
            | "cpuset_init"
            | "cgroup_init"
            | "taskstats_init_early"
            | "delayacct_init"
            | "acpi_early_init"
            | "thread_stack_cache_init"
            | "cred_init"
            | "fork_init"
            | "uts_ns_init"
            | "dbg_late_init"
            | "net_ns_init"
            | "padata_init"
            | "page_alloc_init_late"
            | "workqueue_init"
            | "workqueue_init_topology"
            | "init_mm_internals"
            | "smp_init"
            | "sched_init_smp"
            | "setup_log_buf"
            | "setup_per_cpu_pageset"
            | "numa_policy_init"
            | "late_time_init"
            | "sched_clock_init"
            | "calibrate_delay"
            | "arch_cpu_finalize_init"
            | "pid_idr_init"
            | "anon_vma_init"
            | "pidfs_init"
            | "acpi_subsystem_init"
            | "check_bugs"
            | "free_initmem"
            | "free_initmem_default"
            | "async_synchronize_full"
            | "mark_readonly"
            | "exit_boot_config"
            | "rcu_end_inkernel_boot"
            | "kthreadd"
            | "cpu_startup_entry"
            | "schedule"
            | "schedule_timeout"
            | "schedule_preempt_disabled"
            | "complete"
            | "complete_all"
            | "wait_for_completion"
            | "wait_for_completion_timeout"
            | "user_mode_thread"
            | "kernel_thread"
            | "setup_machine_fdt"
            | "bootmem_init"
            | "smp_setup_processor_id"
            | "x86_late_time_init"
            | "identify_boot_cpu"
            | "identify_cpu_init" => {
                let msg = format!("{}: soft\\n", f.name);
                // Always weak — real arch/mm/init defs must win over breadcrumbs.
                let strong = false;
                self.emit_x86_soft_breadcrumb(&f.name, &msg, strong);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
