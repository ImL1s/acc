#include <stdio.h>

typedef long (*fn7_t)(long a1, long a2, long a3, long a4, long a5, long a6, long a7);

static long target_fn(long a1, long a2, long a3, long a4, long a5, long a6, long a7) {
    if (a1 != 1) return 101;
    if (a2 != 2) return 102;
    if (a3 != 3) return 103;
    if (a4 != 4) return 104;
    if (a5 != 5) return 105;
    if (a6 != 6) return 106;
    if (a7 != 7) return 107;
    return 0;
}

int main(void) {
    fn7_t fp = target_fn;
    long res = fp(1, 2, 3, 4, 5, 6, 7);
    if (res != 0) {
        printf("Failed: %ld\n", res);
        return (int)res;
    }
    printf("INDIRECT_CALL_7ARGS_OK\n");
    return 0;
}
