/* Soft regression: aggregate `?:` as arg / return (C1 "not an aggregate").
 * Class: typeof(Cond) was Int; emit_materialize_agg_addr rejected the expr.
 * 12-byte SysV small-agg arg/return (RelFileNode-sized). */
struct S {
    int a;
    int b;
    int c;
};

static struct S g = {1, 2, 3};
static struct S h = {4, 5, 6};

static int sink_a, sink_b, sink_c;

__attribute__((noinline)) static void take(struct S s) {
    sink_a = s.a;
    sink_b = s.b;
    sink_c = s.c;
}

__attribute__((noinline)) static struct S pick(int c) {
    return c ? g : h;
}

int main(void) {
    take(1 ? g : h);
    if (sink_a != 1 || sink_b != 2 || sink_c != 3)
        return 1;
    take(0 ? g : h);
    if (sink_a != 4 || sink_b != 5 || sink_c != 6)
        return 2;
    struct S r = pick(1);
    if (r.a != 1 || r.b != 2 || r.c != 3)
        return 3;
    r = pick(0);
    if (r.a != 4 || r.b != 5 || r.c != 6)
        return 4;
    return 0;
}
