/* Soft unit: sizeof(sockaddr_un.sun_path) must be large (Linux ~108).
 * Mirrors postgres UNIXSOCK_PATH_BUFLEN used in pqcomm StreamServerPort.
 */
#include <stdio.h>
#include <sys/un.h>
#include <stddef.h>
#include <string.h>

#ifndef UNIXSOCK_PATH_BUFLEN
#define UNIXSOCK_PATH_BUFLEN sizeof(((struct sockaddr_un *) NULL)->sun_path)
#endif

int
main(void)
{
	size_t		buflen = UNIXSOCK_PATH_BUFLEN;
	size_t		sun_sz = sizeof(struct sockaddr_un);
	size_t		path_off = offsetof(struct sockaddr_un, sun_path);
	char		path[256];

	snprintf(path, sizeof(path), "/tmp/pg_regress-XXXXXX/.s.PGSQL.5432");
	printf("sun_path_buflen=%zu sockaddr_un=%zu sun_path_off=%zu pathlen=%zu\n",
		   buflen, sun_sz, path_off, strlen(path));

	if (buflen < 64) {
		fprintf(stderr, "FAIL buflen too small (%zu)\n", buflen);
		return 1;
	}
	if (strlen(path) >= buflen) {
		fprintf(stderr, "FAIL path longer than soft buflen\n");
		return 1;
	}
	printf("UNIT_OK sockaddr_un_sun_path\n");
	return 0;
}
