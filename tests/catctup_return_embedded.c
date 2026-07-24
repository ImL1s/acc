/* Return &ct->tuple where tuple is embedded deep in heap-allocated struct (CatCTup pattern). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct HeapTupleData {
    unsigned t_len;
    unsigned t_self;
    unsigned t_tableOid;
    void *t_data;
} HeapTupleData;

typedef struct CatCTup {
    int ct_magic;
    unsigned hash_value;
    unsigned long keys[4];
    unsigned long cache_elem[2];
    int refcount;
    char dead;
    char negative;
    HeapTupleData tuple;
    void *c_list;
    void *my_cache;
    char payload[32];
} CatCTup;

static CatCTup *make_entry(void)
{
    CatCTup *ct = calloc(1, sizeof(CatCTup));
    ct->ct_magic = 0x57261502;
    ct->tuple.t_len = 24;
    ct->tuple.t_data = ct->payload;
    strcpy(ct->payload, "hello");
    return ct;
}

static HeapTupleData *miss_return(CatCTup *ct)
{
    return &ct->tuple;
}

int main(void)
{
    CatCTup *ct = make_entry();
    HeapTupleData *tp = miss_return(ct);
    if (!tp || tp->t_len != 24 || !tp->t_data)
        return 1;
    if (strcmp((char *) tp->t_data, "hello") != 0)
        return 2;
    puts("OK");
    return 0;
}
