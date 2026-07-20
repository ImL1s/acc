	.section	__TEXT,__text,regular,pure_instructions
	.p2align	2

	.globl	_adler32
_adler32:
	stp	x29, x30, [sp, #-16]!
	mov	x29, sp
	sub	sp, sp, #48
	str	x0, [x29, #-8]
	str	x1, [x29, #-16]
	str	x2, [x29, #-24]
	ldr	x9, [x29, #-8]
	str	x9, [sp, #-16]!
	movz	x10, #65535
	ldr	x9, [sp], #16
	and	x0, x9, x10
	str	x0, [sp, #-16]!
	add	x9, x29, #-32
	ldr	x0, [sp], #16
	str	x0, [x9]
	ldr	x9, [x29, #-8]
	str	x9, [sp, #-16]!
	movz	x10, #16
	ldr	x9, [sp], #16
	asr	x9, x9, x10
	str	x9, [sp, #-16]!
	movz	x10, #65535
	ldr	x9, [sp], #16
	and	x0, x9, x10
	str	x0, [sp, #-16]!
	add	x9, x29, #-40
	ldr	x0, [sp], #16
	str	x0, [x9]
	movz	x0, #0
	str	x0, [sp, #-16]!
	add	x9, x29, #-48
	ldr	x0, [sp], #16
	str	x0, [x9]
L_adler32_while_0:
	ldr	x9, [x29, #-48]
	str	x9, [sp, #-16]!
	ldr	x10, [x29, #-24]
	ldr	x9, [sp], #16
	cmp	w9, w10
	cset	x0, lt
	cbz	x0, L_adler32_endwhile_1
	ldr	x9, [x29, #-32]
	str	x9, [sp, #-16]!
	ldr	x9, [x29, #-16]
	str	x9, [sp, #-16]!
	ldr	x10, [x29, #-48]
	ldr	x9, [sp], #16
	mov	x11, #1
	mul	x10, x10, x11
	add	x9, x9, x10
	ldrb	w9, [x9]
	str	x9, [sp, #-16]!
	movz	x10, #255
	ldr	x9, [sp], #16
	and	x10, x9, x10
	ldr	x9, [sp], #16
	add	x9, x9, x10
	str	x9, [sp, #-16]!
	movz	x10, #65521
	ldr	x9, [sp], #16
	sdiv	x11, x9, x10
	msub	x0, x11, x10, x9
	str	x0, [sp, #-16]!
	add	x9, x29, #-32
	ldr	x0, [sp], #16
	str	x0, [x9]
	ldr	x9, [x29, #-40]
	str	x9, [sp, #-16]!
	ldr	x10, [x29, #-32]
	ldr	x9, [sp], #16
	add	x9, x9, x10
	str	x9, [sp, #-16]!
	movz	x10, #65521
	ldr	x9, [sp], #16
	sdiv	x11, x9, x10
	msub	x0, x11, x10, x9
	str	x0, [sp, #-16]!
	add	x9, x29, #-40
	ldr	x0, [sp], #16
	str	x0, [x9]
	ldr	x9, [x29, #-48]
	str	x9, [sp, #-16]!
	movz	x10, #1
	ldr	x9, [sp], #16
	add	x0, x9, x10
	str	x0, [sp, #-16]!
	add	x9, x29, #-48
	ldr	x0, [sp], #16
	str	x0, [x9]
	b	L_adler32_while_0
L_adler32_endwhile_1:
	ldr	x9, [x29, #-40]
	str	x9, [sp, #-16]!
	movz	x10, #16
	ldr	x9, [sp], #16
	lsl	x9, x9, x10
	str	x9, [sp, #-16]!
	ldr	x10, [x29, #-32]
	ldr	x9, [sp], #16
	add	x0, x9, x10
	b	L_adler32_epilogue
	mov	w0, #0
L_adler32_epilogue:
	mov	sp, x29
	ldp	x29, x30, [sp], #16
	ret
