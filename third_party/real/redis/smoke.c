/* Stage C2: real Redis 7.2.5 sds.c + zmalloc.c under acc. */
#include "sds.h"
#include "zmalloc.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

/* sds sdscatfmt path may call these (util.c); provide libc-based copies. */
int ll2string(char *dst, size_t dstlen, long long svalue) {
  return snprintf(dst, dstlen, "%lld", svalue);
}
int ull2string(char *dst, size_t dstlen, unsigned long long value) {
  return snprintf(dst, dstlen, "%llu", value);
}

/* Soft Mach symbols for zmalloc_get_rss when system headers are soft. */
unsigned int current_task(void) { return 0; }
int task_for_pid(unsigned int a, int b, unsigned int *c) {
  (void)a;
  (void)b;
  (void)c;
  return 1;
}
int task_info(unsigned int a, int b, int *c, int *d) {
  (void)a;
  (void)b;
  (void)c;
  (void)d;
  return 1;
}
int proc_pidinfo(int a, int b, unsigned long long c, void *d, int e) {
  (void)a;
  (void)b;
  (void)c;
  (void)d;
  (void)e;
  return 0;
}

int main(void) {
  /* Avoid sdscatprintf (varargs into redis path); use sdsnew/sdscat only. */
  sds a = sdsnew("redis");
  a = sdscat(a, "-ok-7");
  void *p = zmalloc(64);
  if (!p) {
    printf("zmalloc fail\n");
    return 2;
  }
  memset(p, 0, 64);
  zfree(p);
  int n = (int)sdslen(a);
  int ok = (n == 10 && a[0] == 'r');
  printf("redis ok sds=%s len=%d\n", a, n);
  sdsfree(a);
  return ok ? 0 : 1;
}
