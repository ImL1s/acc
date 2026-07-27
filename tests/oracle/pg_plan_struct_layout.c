#include <stdio.h>
#include <stddef.h>
#include <assert.h>

typedef int NodeTag;
typedef double Cost;
typedef double Cardinality;
typedef char bool;

typedef struct Plan {
    NodeTag type;
    Cost startup_cost;
    Cost total_cost;
    Cardinality plan_rows;
    int plan_width;
    bool parallel_aware;
    bool parallel_safe;
    bool async_capable;
    int plan_node_id;
    void *targetlist;
    void *qual;
    struct Plan *lefttree;
    struct Plan *righttree;
    void *initPlan;
    void *extParam;
    void *allParam;
} Plan;

int main(void) {
    assert(offsetof(Plan, parallel_aware) == 36);
    assert(offsetof(Plan, plan_node_id) == 40);
    assert(offsetof(Plan, targetlist) == 48);
    assert(offsetof(Plan, qual) == 56);
    assert(offsetof(Plan, lefttree) == 64);
    assert(offsetof(Plan, righttree) == 72);
    assert(sizeof(Plan) == 104);
    printf("PG_PLAN_STRUCT_LAYOUT_OK\n");
    return 0;
}
