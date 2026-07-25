/* Soft unit: PQExpBuffer enlarge + appendShellString (initdb Success path).
 * Mirrors initdb's createPQExpBuffer / appendShellString after trust warning.
 * Hang mode: maxlen/len not sticky → enlarge doubles forever (exponential mmap).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>

#define INITIAL_EXPBUFFER_SIZE 256

typedef struct PQExpBufferData {
	char	   *data;
	size_t		len;
	size_t		maxlen;
} PQExpBufferData;

typedef PQExpBufferData *PQExpBuffer;

#define PQExpBufferBroken(str) ((str) == NULL || (str)->maxlen == 0)

static const char oom_buffer[1] = "";
static const char *oom_buffer_ptr = oom_buffer;

static void
mark_broken(PQExpBuffer str)
{
	if (str->data != oom_buffer)
		free(str->data);
	str->data = (char *) oom_buffer_ptr;
	str->len = 0;
	str->maxlen = 0;
}

static int
enlarge(PQExpBuffer str, size_t needed)
{
	size_t		newlen;
	char	   *newdata;
	int			guard = 0;

	if (PQExpBufferBroken(str))
		return 0;
	if (needed >= ((size_t) INT_MAX - str->len)) {
		mark_broken(str);
		return 0;
	}
	needed += str->len + 1;
	if (needed <= str->maxlen)
		return 1;

	newlen = (str->maxlen > 0) ? (2 * str->maxlen) : 64;
	while (needed > newlen) {
		newlen = 2 * newlen;
		if (++guard > 64) {
			fprintf(stderr, "FAIL enlarge loop (maxlen sticky?)\n");
			exit(2);
		}
	}
	if (newlen > (size_t) INT_MAX)
		newlen = (size_t) INT_MAX;

	newdata = (char *) realloc(str->data, newlen);
	if (!newdata) {
		mark_broken(str);
		return 0;
	}
	str->data = newdata;
	str->maxlen = newlen;
	return 1;
}

static void
append_char(PQExpBuffer str, char ch)
{
	if (PQExpBufferBroken(str))
		return;
	if (str->len + 1 >= str->maxlen)
		enlarge(str, 1);
	str->data[str->len] = ch;
	str->len++;
	str->data[str->len] = '\0';
}

static void
append_str(PQExpBuffer str, const char *s)
{
	size_t		n = strlen(s);

	if (PQExpBufferBroken(str))
		return;
	if (!enlarge(str, n))
		return;
	memcpy(str->data + str->len, s, n);
	str->len += n;
	str->data[str->len] = '\0';
}

/* Minimal appendShellStringNoError (Unix) */
static void
append_shell(PQExpBuffer buf, const char *str)
{
	const char *p;

	if (*str != '\0' &&
		strspn(str, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_./:") == strlen(str)) {
		append_str(buf, str);
		return;
	}
	append_char(buf, '\'');
	for (p = str; *p; p++) {
		if (*p == '\'')
			append_str(buf, "'\"'\"'");
		else
			append_char(buf, *p);
	}
	append_char(buf, '\'');
}

static PQExpBuffer
create_buf(void)
{
	PQExpBuffer res = (PQExpBuffer) malloc(sizeof(PQExpBufferData));

	if (!res)
		return NULL;
	res->data = (char *) malloc(INITIAL_EXPBUFFER_SIZE);
	if (!res->data) {
		free(res);
		return NULL;
	}
	res->maxlen = INITIAL_EXPBUFFER_SIZE;
	res->len = 0;
	res->data[0] = '\0';
	return res;
}

int
main(void)
{
	PQExpBuffer buf;
	char		path[1024];
	int			i;

	buf = create_buf();
	if (!buf) {
		fprintf(stderr, "FAIL create\n");
		return 1;
	}

	/* Many char appends force enlarge; len/maxlen must stick. */
	for (i = 0; i < 1000; i++)
		append_char(buf, 'a' + (i % 26));
	if (buf->len != 1000 || buf->maxlen <= 1000) {
		fprintf(stderr, "FAIL char-append len=%zu maxlen=%zu\n", buf->len, buf->maxlen);
		return 1;
	}

	/* Safe path → fast append_str */
	snprintf(path, sizeof(path),
			 "/work/scratch/postgres-build-15.7/src/bin/initdb/pg_ctl");
	append_shell(buf, path);
	append_shell(buf, " -D ");
	append_shell(buf, "/tmp/pgdata_test_with spaces"); /* forces quoting */

	if (buf->len < 100 || strchr(buf->data, '\'') == NULL) {
		fprintf(stderr, "FAIL shell quote len=%zu data=%s\n", buf->len, buf->data);
		return 1;
	}

	printf("UNIT_OK pqexpbuffer_append_shell len=%zu maxlen=%zu\n", buf->len, buf->maxlen);
	free(buf->data);
	free(buf);
	return 0;
}
