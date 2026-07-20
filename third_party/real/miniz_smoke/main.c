/* miniz_smoke: tiny checksum "project" (adler-like) — multi-file style via includes */
#include "adler.h"

int printf(char *, ...);

int main(void) {
    long c;
    c = adler32(1, "hello", 5);
    if (c == 0)
        return 1;
    printf("miniz_smoke ok\n");
    return 0;
}
