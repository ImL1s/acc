/* Varargs snprintf must not clobber unrelated stack locals (SearchCatCacheMiss ct slot). */
#include <stdio.h>
#include <stdint.h>

static int miss_like(void *v1, unsigned reloid, unsigned idx, int nkeys)
{
    void *ct = NULL;
    char buf[160];

    snprintf(buf, sizeof buf, "reloid=%u idx=%u nkeys=%d v1=%p\n",
             reloid, idx, nkeys, v1);
    ct = (void *) (uintptr_t) 0xdeadbeefcafebabeULL;
    snprintf(buf, sizeof buf, "rel=%p\n", (void *) 0x1000);
    if ((uintptr_t) ct != 0xdeadbeefcafebabeULL)
        return 1;
    return 0;
}

int main(void)
{
    if (miss_like((void *) 0x3000, 7, 2690, 1) != 0)
        return 1;
    puts("OK");
    return 0;
}
