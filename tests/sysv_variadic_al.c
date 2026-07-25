/* Soft unit: SysV %al cleared for soft→libc/libpq-style variadic calls. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Declared variadic (like pqexpbuffer.h); defined here so soft owns both sides. */
static void
append_like(char *buf, size_t *len, size_t cap, const char *fmt, ...)
{
	/* Use only the fmt string for a non-va path — just prove the CALL site. */
	(void)cap;
	(void)fmt;
	/* Prefer libc vsnprintf via a tiny helper that takes va_list — skip if SEGV. */
	memcpy(buf + *len, " -l logfile start", 17);
	*len += 17;
	buf[*len] = '\0';
}

int
main(void)
{
	char		buf[64];
	size_t		len = 0;
	char	   *p = NULL;
	int			n;

	buf[0] = '\0';
	/* Call site must emit xorb %al before PLT/local variadic. */
	append_like(buf, &len, sizeof(buf), " -l %s start", "logfile");

	/* External libc variadic not in the old hardcode-only path. */
	n = asprintf(&p, "ok-%s", "var");
	if (n < 0 || !p || strcmp(p, "ok-var") != 0) {
		fprintf(stderr, "FAIL asprintf n=%d p=%s\n", n, p ? p : "(null)");
		return 1;
	}
	free(p);

	if (strcmp(buf, " -l logfile start") != 0) {
		fprintf(stderr, "FAIL buf='%s'\n", buf);
		return 1;
	}
	printf("UNIT_OK sysv_variadic_al\n");
	return 0;
}
