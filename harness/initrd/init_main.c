/* Real userspace init body — compiled by acc, no libc.
 * Standalone ELF path (svc write) for native smoke; kernel path uses
 * acc_init_payload.c instead until EL0 binfmt works.
 */
long acc_sys_write(int fd, const void *buf, unsigned long n);

static unsigned long slen(const char *s)
{
	unsigned long n = 0;
	while (s[n])
		n++;
	return n;
}

int main(void)
{
	const char *msg = "acc-init: real userspace ELF running as pid1\n";
	acc_sys_write(1, msg, slen(msg));
	for (;;) {
	}
	return 0;
}
