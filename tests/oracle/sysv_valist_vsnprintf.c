#include <stdio.h>
#include <stdarg.h>
#include <string.h>

static int my_vsnprintf(char *buf, size_t size, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int res = vsnprintf(buf, size, fmt, ap);
    va_end(ap);
    return res;
}

int main(void) {
    char buf[128];
    int n = my_vsnprintf(buf, sizeof(buf), "Test %s %d %x", "valist", 42, 0xabcd);
    if (n < 0) return 1;
    if (strcmp(buf, "Test valist 42 abcd") != 0) {
        printf("Mismatch: %s\n", buf);
        return 2;
    }
    printf("SYSV_VALIST_VSNPRINTF_OK\n");
    return 0;
}
