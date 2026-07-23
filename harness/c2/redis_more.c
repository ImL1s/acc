#include "sds.h"
#include "zmalloc.h"
#include <stdio.h>
#include <string.h>

int main(void)
{
	int n = 0;
	sds a = sdsnew("redis");
	a = sdscat(a, "-ok-7");
	if ((int)sdslen(a) != 10 || a[0] != 'r') {
		printf("fail1\n");
		return 1;
	}
	n++;
	sds b = sdsdup(a);
	if (sdscmp(a, b) != 0) {
		printf("fail2\n");
		return 2;
	}
	n++;
	sds c = sdsnew("hello");
	c = sdscatsds(c, a);
	if ((int)sdslen(c) != 15) {
		printf("fail3\n");
		return 3;
	}
	n++;
	void *p = zmalloc(64);
	if (!p) {
		printf("fail4\n");
		return 4;
	}
	memset(p, 0xab, 64);
	zfree(p);
	n++;
	void *slots[16];
	int i;
	for (i = 0; i < 16; i++) {
		slots[i] = zmalloc((size_t)(8 + i));
		if (!slots[i]) {
			printf("fail5\n");
			return 5;
		}
	}
	for (i = 0; i < 16; i++)
		zfree(slots[i]);
	n++;
	sds d = sdsnewlen("xyz", 3);
	if ((int)sdslen(d) != 3) {
		printf("fail6\n");
		return 6;
	}
	n++;
	d = sdscatlen(d, "12", 2);
	if ((int)sdslen(d) != 5) {
		printf("fail7\n");
		return 7;
	}
	n++;
	sds e = sdsempty();
	e = sdscpy(e, "cpy");
	if ((int)sdslen(e) != 3 || e[0] != 'c') {
		printf("fail8\n");
		return 8;
	}
	n++;
	sdsfree(a);
	sdsfree(b);
	sdsfree(c);
	sdsfree(d);
	sdsfree(e);
	printf("redis ok basic npass=%d sds=redis-ok-7 len=10\n", n);
	return n >= 5 ? 0 : 1;
}
