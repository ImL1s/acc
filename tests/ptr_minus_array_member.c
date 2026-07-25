/* Soft regression: ptr - flexible/array member must divide by sizeof,
 * not scale the array address as an integer index.
 * Mirrors postgres setrefs.c:
 *   itlist->num_vars = (vinfo - itlist->vars);
 */
typedef struct {
    int varno;
    short varattno;
    short resno;
} tlist_vinfo;

typedef struct {
    void *tlist;
    int num_vars;
    char has_ph_vars;
    char has_non_vars;
    tlist_vinfo vars[1];
} indexed_tlist;

static int count_vars(indexed_tlist *itlist, tlist_vinfo *vinfo) {
    return (int)(vinfo - itlist->vars);
}

int main(void) {
    char buf[128];
    indexed_tlist *it = (indexed_tlist *)buf;
    tlist_vinfo *v = it->vars;
    it->tlist = 0;
    it->num_vars = 0;
    if (count_vars(it, v) != 0)
        return 1;
    if (count_vars(it, v + 1) != 1)
        return 2;
    if (count_vars(it, v + 3) != 3)
        return 3;
    if (count_vars(it, v + 7) != 7)
        return 4;
    return 0;
}
