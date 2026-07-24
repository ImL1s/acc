/* Regression: 12-byte struct by value (PgStat_HashKey shape) through a callee. */
#include <stdio.h>

typedef int PgStat_Kind;
typedef unsigned int Oid;

typedef struct PgStat_HashKey {
    PgStat_Kind kind;
    Oid dboid;
    Oid objoid;
} PgStat_HashKey;

static int check_key(PgStat_HashKey key)
{
    if (key.kind != 7)
        return 1;
    if (key.dboid != 0)
        return 2;
    if (key.objoid != 826)
        return 3;
    return 0;
}

int main(void)
{
    PgStat_HashKey key = {.kind = 7, .dboid = 0, .objoid = 826};
    int r = check_key(key);
    if (r) {
        printf("fail %d\n", r);
        return r;
    }
    puts("OK");
    return 0;
}
