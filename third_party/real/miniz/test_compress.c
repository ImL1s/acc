/* Stage B miniz real project self-test: compress + uncompress roundtrip */
#include "miniz.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *msg =
    "Good morning Dr. Chandra. This is Hal. I am ready for my first lesson. "
    "ggcc Stage B real miniz project compress-uncompress roundtrip.";

int main(void) {
    uLong src_len = (uLong)strlen(msg);
    uLong cmp_len = compressBound(src_len);
    uLong uncomp_len = src_len;
    unsigned char *pCmp;
    unsigned char *pUncomp;
    int st;

    pCmp = (unsigned char *)malloc((size_t)cmp_len);
    pUncomp = (unsigned char *)malloc((size_t)src_len + 1);
    if (!pCmp || !pUncomp) {
        printf("oom\n");
        return 1;
    }

    st = compress(pCmp, &cmp_len, (const unsigned char *)msg, src_len);
    if (st != Z_OK) {
        printf("compress fail %d\n", st);
        return 2;
    }
    if (cmp_len >= src_len) {
        /* still ok if not smaller; just ensure non-zero */
        if (cmp_len == 0) {
            printf("empty compressed\n");
            return 3;
        }
    }

    st = uncompress(pUncomp, &uncomp_len, pCmp, cmp_len);
    if (st != Z_OK) {
        printf("uncompress fail %d\n", st);
        return 4;
    }
    if (uncomp_len != src_len) {
        printf("len mismatch %lu vs %lu\n", (unsigned long)uncomp_len, (unsigned long)src_len);
        return 5;
    }
    pUncomp[uncomp_len] = 0;
    if (strcmp((char *)pUncomp, msg) != 0) {
        printf("data mismatch\n");
        return 6;
    }

    printf("miniz ok cmp=%lu src=%lu\n", (unsigned long)cmp_len, (unsigned long)src_len);
    free(pCmp);
    free(pUncomp);
    return 0;
}
