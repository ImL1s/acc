/* Regression: local array of structs with brace init must use int field sizes
 * (not .byte). Broke SQLite window frame coercion (zName pointer table). */
#include <stdio.h>
static const char n1[] = "row_number";
static const char n2[] = "rank";
struct W {
  const char *z;
  int a, b, c;
};
int main(void) {
  struct W a[] = {
    { n1, 76, 90, 85 },
    { n2, 89, 90, 85 },
  };
  if (a[0].z != n1 || a[0].a != 76 || a[0].b != 90 || a[0].c != 85) {
    printf("FAIL0 z=%s a=%d b=%d c=%d\n", a[0].z ? a[0].z : "null", a[0].a, a[0].b, a[0].c);
    return 1;
  }
  if (a[1].z != n2 || a[1].a != 89 || a[1].b != 90 || a[1].c != 85) {
    printf("FAIL1\n");
    return 1;
  }
  printf("OK\n");
  return 0;
}
