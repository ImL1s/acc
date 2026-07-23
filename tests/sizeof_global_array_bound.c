/* Regression: local array bound sizeof(global_array)+N must not collapse to 0.
 *
 * Bug: file-scope types were invisible to parse-time sizeof folding, so
 *   u8 zHeader[sizeof(aJournalMagic)+4];
 * became a zero-length array; syncJournal then called
 *   sqlite3OsWrite(jfd, zHeader, sizeof(zHeader)=0, ...);
 * and never patched journal magic — SQLite SAVEPOINT ROLLBACK failed
 * under cache_size=10 + large records + index (savepoint-6.3).
 */
#include <stdio.h>
static const unsigned char aJournalMagic[] = {
  0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7,
};
static int sync_like_write_amt(void) {
  unsigned char zHeader[sizeof(aJournalMagic) + 4];
  /* mimic the OsWrite size argument */
  return (int)sizeof(zHeader);
}
int main(void) {
  int a = (int)sizeof(aJournalMagic);
  int b = sync_like_write_amt();
  printf("magic=%d zHeader=%d\n", a, b);
  return (a == 8 && b == 12) ? 0 : 1;
}
