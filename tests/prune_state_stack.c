/* Regression: PruneState-sized stack local + marked[] memset/index (postgres heap_page_prune). */
#include <stdio.h>
#include <string.h>
#include <stddef.h>

typedef unsigned short OffsetNumber;
typedef unsigned int TransactionId;
typedef long long TimestampTz;
typedef void *Relation;
typedef void *GlobalVisState;

#define MaxHeapTuplesPerPage 291
#define FirstOffsetNumber 1

typedef struct {
    Relation rel;
    GlobalVisState *vistest;
    TimestampTz old_snap_ts;
    TransactionId old_snap_xmin;
    char old_snap_used;
    TransactionId new_prune_xid;
    TransactionId latestRemovedXid;
    int nredirected;
    int ndead;
    int nunused;
    OffsetNumber redirected[MaxHeapTuplesPerPage * 2];
    OffsetNumber nowdead[MaxHeapTuplesPerPage];
    OffsetNumber nowunused[MaxHeapTuplesPerPage];
    char marked[MaxHeapTuplesPerPage + 1];
    signed char htsv[MaxHeapTuplesPerPage + 1];
} PruneState;

typedef struct {
    unsigned int t_len;
    unsigned short t_self[2];
    unsigned int t_tableOid;
    void *t_data;
} HeapTupleData;

int use_prune(void) {
    PruneState prstate;
    HeapTupleData tup;
    OffsetNumber offnum;
    int i;

    prstate.new_prune_xid = 0;
    prstate.rel = (Relation)0x1000;
    prstate.vistest = (GlobalVisState *)0x2000;
    prstate.old_snap_xmin = 0;
    prstate.old_snap_ts = 0;
    prstate.old_snap_used = 0;
    prstate.latestRemovedXid = 0;
    prstate.nredirected = prstate.ndead = prstate.nunused = 0;
    memset(prstate.marked, 0, sizeof(prstate.marked));
    tup.t_len = 0;
    tup.t_tableOid = 0;
    tup.t_data = 0;

    for (offnum = FirstOffsetNumber; offnum <= MaxHeapTuplesPerPage; offnum = (OffsetNumber)(offnum + 1)) {
        prstate.htsv[offnum] = -1;
        if (prstate.marked[offnum] != 0)
            return 100 + offnum;
    }
    for (i = 0; i < 64; i++)
        prstate.redirected[i] = (OffsetNumber)i;
    prstate.marked[5] = 1;
    if (prstate.marked[5] != 1)
        return 200;
    return 0;
}

int main(void) {
    printf("sizeof(PruneState)=%zu\n", sizeof(PruneState));
    printf("offsetof marked=%zu htsv=%zu\n",
           offsetof(PruneState, marked), offsetof(PruneState, htsv));
    int r = use_prune();
    if (r) {
        printf("FAIL %d\n", r);
        return 1;
    }
    printf("PASS\n");
    return 0;
}
