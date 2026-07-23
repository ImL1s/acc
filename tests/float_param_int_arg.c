/* Regression: fmov x0,d0 must not clobber following integer args (AAPCS64). */
#include <stdio.h>
#include <string.h>
#include <stdint.h>
static int same_as_int(double r1, int64_t i) {
  double r2 = (double)i;
  return r1 == 0.0
      || (memcmp(&r1, &r2, sizeof(r1)) == 0
          && i >= -2251799813685248LL && i < 2251799813685248LL);
}
int main(void) {
  if (!same_as_int(1.0, 1)) { printf("FAIL 1\n"); return 1; }
  if (!same_as_int(-1.0, -1)) { printf("FAIL -1\n"); return 1; }
  if (!same_as_int(12300000.0, 12300000)) { printf("FAIL 123e5\n"); return 1; }
  if (same_as_int(1.5, 1)) { printf("FAIL 1.5\n"); return 1; }
  printf("PASS\n");
  return 0;
}
