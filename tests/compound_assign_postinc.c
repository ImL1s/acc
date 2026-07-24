/* Regression: `x += *p++` must not clobber LHS address while evaluating RHS. */
#include <stdio.h>

int main(void) {
    unsigned char buf[] = {10, 20, 30};
    unsigned char *sp = buf;
    int len = 18;
    /* Same shape as pglz_decompress extended match tag. */
    if (len == 18)
        len += *sp++;
    if (len != 28) {
        printf("len=%d want 28\n", len);
        return 1;
    }
    if (sp != buf + 1) {
        printf("sp advanced wrong\n");
        return 2;
    }
    puts("OK");
    return 0;
}
