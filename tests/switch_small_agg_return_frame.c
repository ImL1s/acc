/* Regression: switch-case `return small_agg_fn()` must not under-allocate
 * the frame. Soft used to skip small-agg materialize slots in measure while
 * emit spilled each `return AlterXxx(...)` to a new stack temp — callers with
 * deep case locals (ObjectAddress) overlapped the callee frame and lost
 * classId (postgres ALTER LANGUAGE → table_open(OID 0)). */
#include <stdio.h>

typedef unsigned int Oid;

typedef struct ObjectAddress {
    Oid classId;
    Oid objectId;
    int objectSubId;
} ObjectAddress;

/* Large callee frame — clobbers caller locals if they sit below measured rsp. */
static ObjectAddress make_oa(Oid c, Oid o, int s)
{
    volatile char pad[192];
    ObjectAddress a;
    int i;
    for (i = 0; i < 192; i++)
        pad[i] = (char)(i * 3);
    a.classId = c;
    a.objectId = o;
    a.objectSubId = s;
    if (pad[191] == 1)
        a.classId = 0;
    return a;
}

static ObjectAddress early0(void) { return make_oa(100, 1, 0); }
static ObjectAddress early1(void) { return make_oa(101, 2, 0); }
static ObjectAddress early2(void) { return make_oa(102, 3, 0); }
static ObjectAddress early3(void) { return make_oa(103, 4, 0); }
static ObjectAddress early4(void) { return make_oa(104, 5, 0); }
static ObjectAddress early5(void) { return make_oa(105, 6, 0); }
static ObjectAddress early6(void) { return make_oa(106, 7, 0); }
static ObjectAddress early7(void) { return make_oa(107, 8, 0); }

static ObjectAddress run(int kind)
{
    Oid top = 1;
    switch (kind) {
    case 0:
        return early0();
    case 1:
        return early1();
    case 2:
        return early2();
    case 3:
        return early3();
    case 4:
        return early4();
    case 5:
        return early5();
    case 6:
        return early6();
    case 7:
        return early7();
    case 21: {
        Oid classId;
        ObjectAddress address;
        void *relation;
        char sink[8];
        (void)top;
        (void)sink;
        address = make_oa(2612, 42, 0);
        classId = address.classId;
        relation = 0;
        if (classId != 2612)
            return address;
        if (relation)
            address.objectId = 0;
        return address;
    }
    default:
        break;
    }
    {
        ObjectAddress z;
        z.classId = 0;
        z.objectId = 0;
        z.objectSubId = 0;
        return z;
    }
}

int main(void)
{
    ObjectAddress a;
    int i;
    for (i = 0; i < 8; i++) {
        a = run(i);
        if (a.classId != (Oid)(100 + i) || a.objectId != (Oid)(i + 1))
            return 10 + i;
    }
    a = run(21);
    if (a.classId != 2612) {
        printf("class=%u\n", a.classId);
        return 1;
    }
    if (a.objectId != 42) {
        printf("obj=%u\n", a.objectId);
        return 2;
    }
    puts("OK");
    return 0;
}
