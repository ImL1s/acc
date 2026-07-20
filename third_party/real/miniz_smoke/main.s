	.section	__TEXT,__text,regular,pure_instructions
	.p2align	2

	.globl	_main
_main:
	stp	x29, x30, [sp, #-16]!
	mov	x29, sp
	sub	sp, sp, #16
	movz	x0, #5
	str	x0, [sp, #-16]!
	adrp	x0, l_str_0@PAGE
	add	x0, x0, l_str_0@PAGEOFF
	str	x0, [sp, #-16]!
	movz	x0, #1
	str	x0, [sp, #-16]!
	ldr	x16, [sp, #0]
	mov	x0, x16
	ldr	x16, [sp, #16]
	mov	x1, x16
	ldr	x16, [sp, #32]
	mov	x2, x16
	add	sp, sp, #48
	bl	_adler32
	str	x0, [sp, #-16]!
	add	x9, x29, #-8
	ldr	x0, [sp], #16
	str	x0, [x9]
	ldr	x9, [x29, #-8]
	str	x9, [sp, #-16]!
	movz	x10, #0
	ldr	x9, [sp], #16
	cmp	w9, w10
	cset	x0, eq
	cbz	x0, L_main_else_0
	movz	x0, #1
	b	L_main_epilogue
	b	L_main_endif_1
L_main_else_0:
L_main_endif_1:
	adrp	x0, l_str_1@PAGE
	add	x0, x0, l_str_1@PAGEOFF
	str	x0, [sp, #-16]!
	ldr	x0, [sp, #0]
	bl	_printf
	add	sp, sp, #16
	movz	x0, #0
	b	L_main_epilogue
	mov	w0, #0
L_main_epilogue:
	mov	sp, x29
	ldp	x29, x30, [sp], #16
	ret

	.section	__TEXT,__cstring,cstring_literals
l_str_0:
	.asciz	"hello"
l_str_1:
	.asciz	"miniz_smoke ok\n"
