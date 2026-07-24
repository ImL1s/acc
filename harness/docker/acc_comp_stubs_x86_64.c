/* Stubs for arch/x86/boot/compressed link under ggcc. */
#ifdef __GNUC__
#define WEAK __attribute__((weak))
#else
#define WEAK
#endif

WEAK unsigned long x0;
WEAK unsigned long x1;
WEAK long __builtin_constant_p(long x) { (void)x; return 0; }
WEAK void accept_memory(unsigned long start, unsigned long end)
{
	(void)start;
	(void)end;
}
WEAK void boot_stage1_vc(void) {}
WEAK void boot_stage2_vc(void) {}
WEAK int _printk(const char *fmt, ...)
{
	(void)fmt;
	return 0;
}

__asm__(
	".weak __builtin_choose_expr\n"
	".globl __builtin_choose_expr\n"
	"__builtin_choose_expr:\n"
	"\tmovq\t%rsi, %rax\n"
	"\tretq\n"
);
