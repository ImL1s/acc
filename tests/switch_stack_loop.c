/* Regression: switch must not leak stack per iteration.
 * Old bug: break jumped to swend before add sp,#16 → 16B/iter → SEGV after ~100k.
 * Run many switch dispatches in a tight loop; should finish with small stack.
 */
#include <stdio.h>

static int classify(int x) {
  switch (x & 7) {
  case 0: return 10;
  case 1: return 11;
  case 2: return 12;
  case 3: return 13;
  case 4: return 14;
  case 5: return 15;
  case 6: return 16;
  default: return 17;
  }
}

int main(void) {
  long long s = 0;
  /* 500k iterations × 16B leak would need ~8MB; must not SEGV */
  for (int i = 0; i < 500000; i++) {
    s += classify(i);
  }
  printf("sum=%lld\n", s);
  return s == 0 ? 1 : 0;
}
