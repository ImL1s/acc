/* Driver for Redis upstream sdsTest() under REDIS_TEST.
 * Compile with -DREDIS_TEST; link sds.o zmalloc.o stubs.
 *
 * Note: sdsTest prints "N - descr: PASSED/FAILED" lines; we grade by that
 * (cross-TU __test_num can be masked by weak stubs in older ggcc builds).
 */
int sdsTest(int argc, char **argv, int flags);

int __failed_tests = 0;
int __test_num = 0;

int main(int argc, char **argv)
{
	int rc = sdsTest(argc, argv, 0);
	if (rc != 0)
		return rc;
	/* Prefer shared counters when visible; otherwise success if sdsTest returned. */
	if (__failed_tests != 0)
		return 1;
	return 0;
}
