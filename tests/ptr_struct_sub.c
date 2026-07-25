/* Soft: pointer subtraction must divide by sizeof(struct), not 1.
 * Mirrors postgres setrefs.c: itlist->num_vars = (vinfo - itlist->vars); */
struct V {
    int a;
    short b;
    short c;
};

static int count(struct V *base, struct V *end) {
    return (int)(end - base);
}

int main(void) {
    struct V a[5];
    if (sizeof(struct V) != 8)
        return 10;
    if (count(a, a + 4) != 4)
        return 1;
    if (count(a, a + 1) != 1)
        return 2;
    if (count(a, a) != 0)
        return 3;
    return 0;
}
