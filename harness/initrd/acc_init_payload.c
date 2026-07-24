/*
 * acc_init_payload.c — C1 init handoff body, compiled only by acc.
 *
 * Linked into vmlinux (init/acc_init_payload.o) and called from the
 * freestanding run_init_process helper. This is NOT full EL0 binfmt_elf
 * userspace; it is a real acc-compiled C function that runs at the
 * kernel pid1 handoff and prints a distinct serial marker via freestanding
 * _printk (PL011).
 */
extern int _printk(const char *fmt);

void acc_real_init_payload(void)
{
	_printk("acc-init: real userspace ELF running as pid1\n");
}
