/*
 * Freestanding ring3 enter + SYSCALL stubs for C1 busybox banner (x86_64).
 * Assembled by system as — not compiled by ggcc.
 *
 * ggcc_el0_enter(entry, sp_user, cr3_phys)
 *   %rdi = user entry PC
 *   %rsi = user SP
 *   %rdx = CR3 phys (PML4: kernel high half copied + user maps)
 * Does not return on success (busybox exit_group parks).
 *
 * GDT layout (Linux-ish for SYSRETQ):
 *   0x00 null | 0x08 unused | 0x10 kern64-code | 0x18 kern-data
 *   0x20 user32-code | 0x28 user-data | 0x30 user64-code
 * STAR = (USER32_CS<<48)|(KERNEL_CS<<32) → SYSRETQ CS=0x33 SS=0x2b
 */
	.global	ggcc_el0_enter
	.global	ggcc_uart_nwrite
	.global	ggcc_el0_svc_c
	.global	ggcc_cache_flush_range
	.global	ggcc_read_far_el1
	.global	ggcc_set_tpidr_el0
	.global	ggcc_el0_syscall
	.global	ggcc_read_cr3

	.text
	.p2align	4

ggcc_read_cr3:
	movq	%cr3, %rax
	ret

ggcc_read_far_el1:
	xorq	%rax, %rax
	ret

ggcc_set_tpidr_el0:
	/* %rdi → IA32_FS_BASE (early TLS canary base) */
	movl	$0xC0000100, %ecx
	movq	%rdi, %rax
	movq	%rdi, %rdx
	shrq	$32, %rdx
	wrmsr
	ret

ggcc_cache_flush_range:
	ret

/* COM1 (0x3f8): %rdi=buf %rsi=len */
	.p2align	4
ggcc_uart_nwrite:
	pushq	%rbx
	movq	%rdi, %rbx
	movq	%rsi, %rcx
	testq	%rcx, %rcx
	jz	.Luart_done
.Luart_loop:
	movzbl	(%rbx), %eax
	movw	$0x3f8, %dx
	outb	%al, %dx
	incq	%rbx
	decq	%rcx
	jnz	.Luart_loop
.Luart_done:
	popq	%rbx
	ret

/* SYSCALL entry: rax=sysno; rdi/rsi/rdx/r10/r8/r9=args; rcx=RIP; r11=RFLAGS */
	.p2align	4
ggcc_el0_syscall:
	movq	%rsp, ggcc_user_rsp(%rip)
	movq	ggcc_kernel_rsp(%rip), %rsp
	pushq	%rcx				/* user RIP */
	pushq	%r11				/* user RFLAGS */
	pushq	%r9
	pushq	%r8
	pushq	%r10
	pushq	%rdx
	pushq	%rsi
	pushq	%rdi
	pushq	%rax				/* sysno */
	/* Call ggcc_el0_svc_c(a0..a5, sysno) — 7th arg on stack */
	movq	8(%rsp), %rdi
	movq	16(%rsp), %rsi
	movq	24(%rsp), %rdx
	movq	32(%rsp), %rcx			/* r10 → 4th arg */
	movq	40(%rsp), %r8
	movq	48(%rsp), %r9
	pushq	(%rsp)				/* sysno as 7th */
	call	ggcc_el0_svc_c
	addq	$8, %rsp
	/* %rax = retval; unwind to sysretq */
	addq	$8, %rsp			/* drop sysno */
	popq	%rdi
	popq	%rsi
	popq	%rdx
	popq	%r10
	popq	%r8
	popq	%r9
	popq	%r11
	popq	%rcx
	movq	ggcc_user_rsp(%rip), %rsp
	sysretq

/* Enter ring3 via iretq */
	.p2align	4
ggcc_el0_enter:
	cli
	movq	%rdi, ggcc_user_entry(%rip)
	movq	%rsi, ggcc_user_sp(%rip)
	movq	%rdx, ggcc_user_cr3(%rip)
	movq	%rsp, ggcc_kernel_rsp(%rip)

	leaq	ggcc_gdt(%rip), %rax
	movq	%rax, ggcc_gdt_ptr+2(%rip)
	lgdt	ggcc_gdt_ptr(%rip)

	pushq	$0x10
	leaq	.Lreload_cs(%rip), %rax
	pushq	%rax
	lretq
.Lreload_cs:
	movw	$0x18, %ax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %ss

	/* IA32_EFER.SCE */
	movl	$0xC0000080, %ecx
	rdmsr
	orl	$1, %eax
	wrmsr

	/* IA32_LSTAR */
	leaq	ggcc_el0_syscall(%rip), %rax
	movl	$0xC0000082, %ecx
	movq	%rax, %rdx
	shrq	$32, %rdx
	wrmsr

	/* IA32_STAR: kernel CS=0x10, user32 CS=0x20 */
	movl	$0xC0000081, %ecx
	xorl	%eax, %eax
	movl	$0x00200010, %edx
	wrmsr

	/* IA32_FMASK: clear IF on syscall */
	movl	$0xC0000084, %ecx
	movl	$0x200, %eax
	xorl	%edx, %edx
	wrmsr

	/* FS_BASE=0x740 */
	movl	$0xC0000100, %ecx
	movl	$0x740, %eax
	xorl	%edx, %edx
	wrmsr

	movq	ggcc_user_cr3(%rip), %rax
	movq	%rax, %cr3

	movq	ggcc_user_sp(%rip), %rax
	pushq	$0x2b				/* user SS */
	pushq	%rax
	pushq	$0x202				/* IF */
	pushq	$0x33				/* user CS */
	movq	ggcc_user_entry(%rip), %rax
	pushq	%rax

	xorq	%rax, %rax
	xorq	%rbx, %rbx
	xorq	%rcx, %rcx
	xorq	%rdx, %rdx
	xorq	%rsi, %rsi
	xorq	%rdi, %rdi
	xorq	%rbp, %rbp
	xorq	%r8, %r8
	xorq	%r9, %r9
	xorq	%r10, %r10
	xorq	%r11, %r11
	xorq	%r12, %r12
	xorq	%r13, %r13
	xorq	%r14, %r14
	xorq	%r15, %r15
	iretq

	.data
	.p2align	3
ggcc_kernel_rsp:	.quad	0
ggcc_user_rsp:		.quad	0
ggcc_user_entry:	.quad	0
ggcc_user_sp:		.quad	0
ggcc_user_cr3:		.quad	0

	.p2align	3
ggcc_gdt:
	.quad	0x0000000000000000		/* 0x00 null */
	.quad	0x0000000000000000		/* 0x08 unused */
	.quad	0x00af9a000000ffff		/* 0x10 kern 64-code */
	.quad	0x00cf92000000ffff		/* 0x18 kern data */
	.quad	0x00cffa000000ffff		/* 0x20 user 32-code */
	.quad	0x00cff2000000ffff		/* 0x28 user data */
	.quad	0x00affa000000ffff		/* 0x30 user 64-code */
ggcc_gdt_end:

	.p2align	3
ggcc_gdt_ptr:
	.word	ggcc_gdt_end - ggcc_gdt - 1
	.quad	0

/* Link stub: ggcc may emit calls to __builtin_choose_expr instead of folding. */
	.weak	__builtin_choose_expr
	.globl	__builtin_choose_expr
	.p2align	4, 0x90
__builtin_choose_expr:
	movq	%rsi, %rax
	retq
