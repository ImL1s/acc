#include "sds.h"
#include "zmalloc.h"
#include <stdio.h>
#include <string.h>
int main(void) {
  int n=0;
  sds a = sdsnew("redis");
  a = sdscat(a, "-ok-7");
  if ((int)sdslen(a) != 10 || a[0] != 'r') { printf("fail1\n"); return 1; }
  n++;
  sds b = sdsdup(a);
  if (sdscmp(a, b) != 0) { printf("fail2\n"); return 2; }
  n++;
  sds c = sdsnew("hello");
  c = sdscatsds(c, a);
  if ((int)sdslen(c) != 15) { printf("fail3 len=%d\n",(int)sdslen(c)); return 3; }
  n++;
  void *p = zmalloc(64);
  if (!p) { printf("fail4\n"); return 4; }
  memset(p, 0xab, 64);
  zfree(p);
  n++;
  void *slots[16];
  for (int i=0;i<16;i++) {
    slots[i]=zmalloc((size_t)(8+i));
    if (!slots[i]) { printf("fail5\n"); return 5; }
  }
  for (int i=0;i<16;i++) zfree(slots[i]);
  n++;
  printf("redis ok basic npass=%d sds=%s len=%d\n", n, a, (int)sdslen(a));
  sdsfree(a); sdsfree(b); sdsfree(c);
  return 0;
}
