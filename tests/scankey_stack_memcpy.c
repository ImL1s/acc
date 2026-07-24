/* ScanKeyData stack layout: cur_skey[4] must not be scribbled past by memcpy/beginscan. */
#include <stdio.h>
#include <string.h>

typedef unsigned int Oid;
typedef short StrategyNumber;
typedef short AttrNumber;
typedef void *Datum;
typedef void *PGFunction;

typedef struct FmgrInfo {
    PGFunction fn_addr;
    Oid fn_oid;
    short fn_nargs;
    char fn_strict;
    char fn_retset;
    unsigned char fn_stats;
    void *fn_extra;
    void *fn_mcxt;
    void *fn_expr;
} FmgrInfo;

typedef struct ScanKeyData {
    int sk_flags;
    AttrNumber sk_attno;
    StrategyNumber sk_strategy;
    Oid sk_subtype;
    Oid sk_collation;
    FmgrInfo sk_func;
    Datum sk_argument;
} ScanKeyData;

#define CATCACHE_MAXKEYS 4

static ScanKeyData template_key = {
    .sk_flags = 1,
    .sk_attno = 1,
    .sk_strategy = 1,
    .sk_subtype = 0,
    .sk_collation = 0,
    .sk_func = {.fn_addr = (PGFunction) 0x1000},
    .sk_argument = (Datum) (uintptr_t) 0x2000,
};

static int miss_like_copy(int nkeys, ScanKeyData *cur_skey, Datum v1)
{
    char guard = 0x5a;
    memcpy(cur_skey, &template_key, sizeof(ScanKeyData) * (size_t) nkeys);
    cur_skey[0].sk_argument = v1;
    if (guard != (char) 0x5a)
        return 1;
    if (cur_skey[0].sk_attno != 1)
        return 2;
    if (cur_skey[0].sk_func.fn_addr != (PGFunction) 0x1000)
        return 3;
    return 0;
}

int main(void)
{
    ScanKeyData cur_skey[CATCACHE_MAXKEYS];
    int r = miss_like_copy(1, cur_skey, (Datum) (uintptr_t) 0x3000);
    if (r != 0)
        return r;
    if (sizeof(ScanKeyData) < 32)
        return 4;
    puts("OK");
    return 0;
}
