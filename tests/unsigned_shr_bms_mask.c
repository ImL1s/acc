/* Soft regression: unsigned >> must be logical (shr), not arithmetic (sar).
 * Mirrors postgres bitmapset.c bms_add_range same-word mask:
 *   words[w] |= ~(bitmapword)(((bitmapword)1 << lbitnum) - 1)
 *               & (~(bitmapword)0) >> ushiftbits;
 * For lower=0, upper=1 with 64-bit words, result must be 3 (bits 0 and 1).
 * Arithmetic >> of all-ones leaves all-ones → bogus extra members (e.g. bit 2)
 * → Append OOB list_nth → FATAL unrecognized node type: 0.
 */
typedef unsigned long bitmapword;

#define BITS_PER_BITMAPWORD 64
#define BITNUM(x) ((x) % BITS_PER_BITMAPWORD)

static bitmapword add_range_mask(int lower, int upper) {
    int lbitnum = BITNUM(lower);
    int ushiftbits = BITS_PER_BITMAPWORD - (BITNUM(upper) + 1);
    bitmapword w = 0;
    w |= ~(bitmapword)(((bitmapword)1 << lbitnum) - 1)
         & (~(bitmapword)0) >> ushiftbits;
    return w;
}

static int next_member(bitmapword w, int prev) {
    int b;
    prev++;
    for (b = prev; b < BITS_PER_BITMAPWORD; b++) {
        if (w & ((bitmapword)1 << b))
            return b;
    }
    return -2;
}

int main(void) {
    bitmapword w;
    int x;

    /* Core: (~0UL) >> n must be logical. Use runtime n so soft cannot
     * hide behind a bad const-fold of a signed shift. */
    {
        volatile int n = 62;
        if (((~(bitmapword)0) >> n) != (bitmapword)3)
            return 1;
    }

    w = add_range_mask(0, 1);
    if (w != (bitmapword)3)
        return 2;

    x = -1;
    if ((x = next_member(w, x)) != 0)
        return 3;
    if ((x = next_member(w, x)) != 1)
        return 4;
    if ((x = next_member(w, x)) != -2)
        return 5; /* must not yield 2 */

    w = add_range_mask(0, 0);
    if (w != (bitmapword)1)
        return 6;

    w = add_range_mask(2, 4);
    if (w != (bitmapword)0x1c) /* bits 2,3,4 */
        return 7;

    return 0;
}
