/* Soft unit: static local addr in static aggregate initializer.
 * Mirrors dropdb.c: static int if_exists; static struct option[] = { &if_exists }.
 * Soft must emit .quad __static_main_if_exists_N, not bare U if_exists.
 */
#include <stdio.h>
#include <string.h>

struct option {
	const char *name;
	int			has_arg;
	int		   *flag;
	int			val;
};

int
main(void)
{
	static int	if_exists = 0;
	static struct option long_options[] = {
		{"help", 0, NULL, 'h'},
		{"if-exists", 0, &if_exists, 1},
		{NULL, 0, NULL, 0}
	};

	if_exists = 0;
	*long_options[1].flag = long_options[1].val;
	if (if_exists != 1) {
		fprintf(stderr, "FAIL if_exists=%d\n", if_exists);
		return 1;
	}
	if (strcmp(long_options[1].name, "if-exists") != 0) {
		fprintf(stderr, "FAIL name\n");
		return 1;
	}
	printf("UNIT_OK static_local_addr_in_init\n");
	return 0;
}
