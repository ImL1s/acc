#include <stdio.h>
int main(void) {
  int ok = ('\a'==7 && '\b'==8 && '\f'==12 && '\n'==10 && '\r'==13 && '\t'==9 && '\v'==11);
  printf(ok ? "OK\n" : "FAIL a=%d b=%d f=%d v=%d\n", (int)'\a', (int)'\b', (int)'\f', (int)'\v');
  return ok ? 0 : 1;
}
