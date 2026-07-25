/* Regression: 12-byte struct return (ObjectAddress / SysV %rax:%rdx). */
#include <stdio.h>

typedef unsigned int Oid;

typedef struct ObjectAddress {
    Oid classId;
    Oid objectId;
    int objectSubId;
} ObjectAddress;

static ObjectAddress make_addr(Oid c, Oid o, int s)
{
    ObjectAddress a;
    a.classId = c;
    a.objectId = o;
    a.objectSubId = s;
    return a;
}

int main(void)
{
    ObjectAddress a = make_addr(1255, 42, 7);
    if (a.classId != 1255)
        return 1;
    if (a.objectId != 42)
        return 2;
    if (a.objectSubId != 7)
        return 3;
    puts("OK");
    return 0;
}
