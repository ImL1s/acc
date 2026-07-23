/* Regression: static int x=0; x++; if(x==1) must see the store.
 * Old bug: const-folded static loads to init value forever. */
#include <stdio.h>
static int n = 0;
int main(void) {
  n++;
  if (n != 1) { printf("FAIL n=%d\n", n); return 1; }
  n++;
  if (n != 2) { printf("FAIL2 n=%d\n", n); return 1; }
  printf("OK\n");
  return 0;
}
