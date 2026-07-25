/* Soft regression: field offset for typedef struct with ptr+int+bools.
 * Postgres setrefs indexed_tlist->num_vars must be at offset 8, not 0. */
typedef struct {
    void *tlist;
    int num_vars;
    char has_ph_vars;
    char has_non_vars;
} indexed_tlist;

typedef struct {
    indexed_tlist *outer_itlist;
    indexed_tlist *inner_itlist;
} fix_ctx;

static int get_nv(indexed_tlist *it) {
    return it ? it->num_vars : -1;
}

static int get_nv_via_ctx(fix_ctx *c) {
    return c->outer_itlist ? c->outer_itlist->num_vars : -1;
}

static void set_nv(indexed_tlist *it, int n) {
    it->num_vars = n;
}

int main(void) {
    indexed_tlist it;
    fix_ctx c;
    it.tlist = (void *)0x123456789ABCDEF0ULL;
    it.num_vars = 0;
    it.has_ph_vars = 0;
    it.has_non_vars = 0;
    set_nv(&it, 7);
    if (it.num_vars != 7)
        return 1;
    if (get_nv(&it) != 7)
        return 2;
    /* tlist pointer must survive set_nv */
    if (it.tlist != (void *)0x123456789ABCDEF0ULL)
        return 3;
    c.outer_itlist = &it;
    c.inner_itlist = &it;
    if (get_nv_via_ctx(&c) != 7)
        return 4;
    return 0;
}
