/* Nested 12-byte struct-by-value (pgstat_get_entry_ref → cached → hash_insert). */
#include <stdio.h>
#include <string.h>

typedef int PgStat_Kind;
typedef unsigned int Oid;

typedef struct PgStat_HashKey {
    PgStat_Kind kind;
    Oid dboid;
    Oid objoid;
} PgStat_HashKey;

typedef struct Entry {
    PgStat_HashKey key;
    char status;
    void *entry_ref;
} Entry;

static int hash_cmp(PgStat_HashKey a, PgStat_HashKey b)
{
    return memcmp(&a, &b, sizeof(PgStat_HashKey));
}

static Entry *hash_insert(Entry *tb, PgStat_HashKey key, int *found)
{
    if (hash_cmp(tb[0].key, key) == 0 && tb[0].status) {
        *found = 1;
        return &tb[0];
    }
    tb[0].key = key;
    tb[0].status = 1;
    *found = 0;
    return &tb[0];
}

static Entry *cached(PgStat_HashKey key, int *found)
{
    static Entry table[2];
    return hash_insert(table, key, found);
}

int main(void)
{
    PgStat_HashKey key = {.kind = 7, .dboid = 1, .objoid = 826};
    int found = 0;
    Entry *e = cached(key, &found);
    if (!e || e->key.kind != 7 || e->key.dboid != 1 || e->key.objoid != 826)
        return 1;
    puts("OK");
    return 0;
}
