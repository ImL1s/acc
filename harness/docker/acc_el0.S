/*
 * Freestanding EL0 enter + SVC vectors for C1 busybox banner path.
 * Assembled by system as (wrapper .S path) — not compiled by ggcc.
 *
 * ggcc_el0_enter(entry, sp_el0, ttbr0_phys, idmap_phys_ptr_unused)
 *   x0 = EL0 entry PC
 *   x1 = EL0 SP
 *   x2 = TTBR0 phys (user PGD)
 * Does not return on success (busybox exit_group parks).
 */
	.global	ggcc_el0_enter
	.global	ggcc_el0_vectors
	.global	ggcc_uart_nwrite
	.global	ggcc_el0_svc_c
	.global	ggcc_cache_flush_range
	.global	ggcc_read_far_el1
	.global	ggcc_set_tpidr_el0

	.text
	.align	11			/* 2KB for VBAR */

ggcc_read_far_el1:
	mrs	x0, far_el1
	ret

ggcc_set_tpidr_el0:
	msr	tpidr_el0, x0
	ret

/* ---- Clean D-cache to PoU + invalidate I-cache for [x0, x0+x1) ---- */
ggcc_cache_flush_range:
	/* x0=start VA (kernel), x1=size */
	mov	x2, x0
	add	x3, x0, x1
1:	cmp	x2, x3
	b.hs	2f
	dc	cvau, x2		/* clean D to PoU */
	add	x2, x2, #64
	b	1b
2:	dsb	ish
	mov	x2, x0
3:	cmp	x2, x3
	b.hs	4f
	ic	ivau, x2		/* invalidate I to PoU */
	add	x2, x2, #64
	b	3b
4:	dsb	ish
	isb
	ret

/* ---- UART write n bytes from kernel buffer (no trailing NL) ---- */
ggcc_uart_nwrite:
	/* x0=buf, x1=len */
	stp	x29, x30, [sp, #-48]!
	mov	x29, sp
	stp	x19, x20, [sp, #16]
	stp	x21, x22, [sp, #32]
	mov	x19, x0
	mov	x20, x1
	cbz	x20, 9f
	/* switch TTBR0 → idmap for PL011 DEVICE (phys 0x9000000) */
	mrs	x21, ttbr0_el1
	adrp	x9, ggcc_idmap_phys
	ldr	x9, [x9, :lo12:ggcc_idmap_phys]
	cbz	x9, 1f
	msr	ttbr0_el1, x9
	isb
	mov	x22, #1
	b	2f
1:	mov	x22, xzr
2:	movz	x0, #0x0000
	movk	x0, #0x900, lsl #16
3:	cbz	x20, 4f
	ldrb	w2, [x19], #1
	str	w2, [x0]
	sub	x20, x20, #1
	b	3b
4:	cbz	x22, 9f
	msr	ttbr0_el1, x21
	isb
9:	ldp	x21, x22, [sp, #32]
	ldp	x19, x20, [sp, #16]
	ldp	x29, x30, [sp], #48
	ret

/* ---- Enter EL0 ---- */
ggcc_el0_enter:
	/* Also set TPIDR in enter (in case C call was lost) — x3 optional */
	/* x0=entry, x1=sp_el0, x2=ttbr0_phys */
	msr	daifset, #0xf
	/* Install our vectors */
	adr	x9, ggcc_el0_vectors
	msr	vbar_el1, x9
	isb
	/* Early TLS canary base: tpidr=0x740 so tpidr-0x740==0 matches BSS */
	movz	x9, #0x740
	msr	tpidr_el0, x9
	/* Enable TTBR0 walks (clear EPD0) — uninstall_idmap may have set it */
	mrs	x9, tcr_el1
	bic	x9, x9, #(1 << 7)
	msr	tcr_el1, x9
	isb
	/* Clear WXN — our 2MB user blocks are RW; WXN would block EL0 exec */
	mrs	x9, sctlr_el1
	bic	x9, x9, #(1 << 19)
	msr	sctlr_el1, x9
	isb
	/* Allow EL0/EL1 FP/SIMD (busybox may touch) */
	mrs	x9, cpacr_el1
	orr	x9, x9, #(3 << 20)
	msr	cpacr_el1, x9
	isb
	/* User TTBR0 */
	msr	ttbr0_el1, x2
	isb
	tlbi	vmalle1
	dsb	nsh
	isb
	/* SPSR: EL0t, all exceptions unmasked once in EL0 (DAIF=0) */
	mov	x9, xzr
	msr	spsr_el1, x9
	msr	elr_el1, x0
	msr	sp_el0, x1
	/* Clear GPRs for clean userspace start (keep x0=argc later via stack) */
	mov	x0, xzr
	mov	x1, xzr
	mov	x2, xzr
	mov	x3, xzr
	mov	x4, xzr
	mov	x5, xzr
	mov	x6, xzr
	mov	x7, xzr
	mov	x8, xzr
	mov	x9, xzr
	mov	x10, xzr
	mov	x11, xzr
	mov	x12, xzr
	mov	x13, xzr
	mov	x14, xzr
	mov	x15, xzr
	mov	x16, xzr
	mov	x17, xzr
	mov	x18, xzr
	mov	x19, xzr
	mov	x20, xzr
	mov	x21, xzr
	mov	x22, xzr
	mov	x23, xzr
	mov	x24, xzr
	mov	x25, xzr
	mov	x26, xzr
	mov	x27, xzr
	mov	x28, xzr
	mov	x29, xzr
	mov	x30, xzr
	eret

/* ---- Exception vectors (EL1) ---- */
	.align	11
ggcc_el0_vectors:
	/* Current EL with SP0: 0x000-0x200 — unused */
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	/* Current EL with SPx: 0x200-0x400 */
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	/* Lower EL using AArch64: 0x400-0x600 */
	.align	7
	b	ggcc_el0_sync64		/* Synchronous */
	.align	7
	b	ggcc_el0_park		/* IRQ */
	.align	7
	b	ggcc_el0_park		/* FIQ */
	.align	7
	b	ggcc_el0_park		/* SError */
	/* Lower EL using AArch32: 0x600-0x800 */
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park
	.align	7
	b	ggcc_el0_park

ggcc_el0_park:
	wfe
	b	ggcc_el0_park

/* EL0 AArch64 sync: SVC or fault */
ggcc_el0_sync64:
	/* Save user x0-x30 + spsr/elr on EL1 stack */
	sub	sp, sp, #272
	stp	x0, x1, [sp, #0]
	stp	x2, x3, [sp, #16]
	stp	x4, x5, [sp, #32]
	stp	x6, x7, [sp, #48]
	stp	x8, x9, [sp, #64]
	stp	x10, x11, [sp, #80]
	stp	x12, x13, [sp, #96]
	stp	x14, x15, [sp, #112]
	stp	x16, x17, [sp, #128]
	stp	x18, x19, [sp, #144]
	stp	x20, x21, [sp, #160]
	stp	x22, x23, [sp, #176]
	stp	x24, x25, [sp, #192]
	stp	x26, x27, [sp, #208]
	stp	x28, x29, [sp, #224]
	str	x30, [sp, #240]
	mrs	x9, spsr_el1
	mrs	x10, elr_el1
	stp	x9, x10, [sp, #248]

	mrs	x9, esr_el1
	lsr	x10, x9, #26		/* EC */
	and	x10, x10, #0x3f
	cmp	x10, #0x15		/* EC_SVC64 */
	b.ne	ggcc_el0_fault

	/* SVC: args x0-x5, sysno x8 → C */
	ldp	x0, x1, [sp, #0]
	ldp	x2, x3, [sp, #16]
	ldp	x4, x5, [sp, #32]
	ldr	x6, [sp, #64]		/* x8 sysno */
	bl	ggcc_el0_svc_c
	/* x0 = return value → user x0 */
	str	x0, [sp, #0]

	/* advance ELR past SVC */
	ldr	x10, [sp, #256]
	add	x10, x10, #4
	str	x10, [sp, #256]

ggcc_el0_return:
	ldp	x9, x10, [sp, #248]
	msr	spsr_el1, x9
	msr	elr_el1, x10
	ldp	x0, x1, [sp, #0]
	ldp	x2, x3, [sp, #16]
	ldp	x4, x5, [sp, #32]
	ldp	x6, x7, [sp, #48]
	ldp	x8, x9, [sp, #64]
	ldp	x10, x11, [sp, #80]
	ldp	x12, x13, [sp, #96]
	ldp	x14, x15, [sp, #112]
	ldp	x16, x17, [sp, #128]
	ldp	x18, x19, [sp, #144]
	ldp	x20, x21, [sp, #160]
	ldp	x22, x23, [sp, #176]
	ldp	x24, x25, [sp, #192]
	ldp	x26, x27, [sp, #208]
	ldp	x28, x29, [sp, #224]
	ldr	x30, [sp, #240]
	add	sp, sp, #272
	eret

ggcc_el0_fault:
	/* Print ESR/ELR then park (breadcrumb) */
	mov	x0, x9			/* esr */
	ldr	x1, [sp, #256]		/* elr */
	bl	ggcc_el0_fault_c
1:	wfe
	b	1b
