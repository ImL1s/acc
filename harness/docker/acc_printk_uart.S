/* Strong early _printk → COM1 via ggcc_uart_nwrite (fmt only, no varargs). */
	.global	_printk
	.global	ggcc_uart_nwrite
	.text
	.p2align 4
_printk:
	pushq	%rbp
	movq	%rsp, %rbp
	pushq	%rbx
	movq	%rdi, %rbx
	xorq	%rsi, %rsi
1:	cmpb	$0, (%rbx,%rsi)
	je	2f
	incq	%rsi
	jmp	1b
2:	movq	%rbx, %rdi
	call	ggcc_uart_nwrite
	xorl	%eax, %eax
	popq	%rbx
	popq	%rbp
	retq
