/* Soft regression: bitwise & on unsigned long must keep ULong type so
 * `(w & mask) != 0` uses cmpq, not cmpl.
 * Postgres bitmapset.c bms_is_member:
 *   (a->words[wordnum] & ((bitmapword)1 << bitnum)) != 0
 * Attr proacl → bit 37 (30 - FirstLowInvalidHeapAttributeNumber). With
 * BitAnd typed as Int, cmpl saw the low 32 bits as zero → member missed →
 * incomplete scan tlists → FATAL variable not found in subplan target list.
 */
typedef unsigned long bitmapword;

#define FirstLowInvalidHeapAttributeNumber (-7)
#define BITS_PER_BITMAPWORD 64
#define WORDNUM(x) ((x) / BITS_PER_BITMAPWORD)
#define BITNUM(x) ((x) % BITS_PER_BITMAPWORD)

static int is_member(bitmapword word, int bit) {
    return (word & ((bitmapword)1 << bit)) != 0;
}

int main(void) {
    bitmapword w = 0;
    int bit_oid = 1 - FirstLowInvalidHeapAttributeNumber;  /* 8 */
    int bit_acl = 30 - FirstLowInvalidHeapAttributeNumber; /* 37 */

    w |= (bitmapword)1 << bit_oid;
    w |= (bitmapword)1 << bit_acl;

    if (!is_member(w, bit_oid))
        return 1;
    if (!is_member(w, bit_acl))
        return 2;
    if (is_member(w, 33))
        return 3;

    /* Direct form matching bms_is_member */
    if ((w & ((bitmapword)1 << 37)) == 0)
        return 4;
    if ((w & ((bitmapword)1 << 37)) != 0) {
        /* ok */
    } else
        return 5;

    return 0;
}
