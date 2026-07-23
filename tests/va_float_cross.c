/* AAPCS64: variadic double in d0; str_vappendf-style helper takes va_list by value. */
#include <stdio.h>
#include <stdarg.h>
#include <string.h>

static void append_double(char *out, va_list ap) {
  double d = va_arg(ap, double);
  sprintf(out, "%.3e", d);
}

char *mprintf_d(const char *fmt, ...) {
  static char buf[64];
  va_list ap;
  (void)fmt;
  va_start(ap, fmt);
  append_double(buf, ap);
  va_end(ap);
  return buf;
}

int main(void) {
  char *s = mprintf_d("%f", 0.001);
  printf("RESULT=%s\n", s);
  if (strcmp(s, "1.000e-03") != 0 && strcmp(s, "1.0e-03") != 0) {
    /* accept either 1.000e-03 or 1.0e-03 */
    if (s[0] != '1' || strstr(s, "e-03") == 0) {
      printf("FAIL got '%s'\n", s);
      return 1;
    }
  }
  printf("PASS\n");
  return 0;
}
