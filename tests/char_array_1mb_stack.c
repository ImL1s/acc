/* Regression: char etalon[1024*1024] must be 1MiB stack, not 8*N (SEGV). */
#include <stdio.h>

int f(void) {
  char etalon[1024 * 1024];
  etalon[0] = 1;
  etalon[1024 * 1024 - 1] = 2;
  return etalon[0] + etalon[1024 * 1024 - 1];
}

int main(void) {
  int r = f();
  if (r != 3) {
    printf("FAIL r=%d\n", r);
    return 1;
  }
  printf("PASS\n");
  return 0;
}
