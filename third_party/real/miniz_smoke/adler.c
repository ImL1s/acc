#include "adler.h"

#define BASE 65521

long adler32(long adler, char *buf, int len) {
    long s1;
    long s2;
    int n;
    s1 = adler & 0xffff;
    s2 = (adler >> 16) & 0xffff;
    n = 0;
    while (n < len) {
        s1 = (s1 + (buf[n] & 0xff)) % BASE;
        s2 = (s2 + s1) % BASE;
        n = n + 1;
    }
    return (s2 << 16) + s1;
}
