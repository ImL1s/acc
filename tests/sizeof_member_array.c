/* Regression: char buf[sizeof(p->arr)] must be 16, not 0.
 * Bug caused SQLite pager version-check OsRead(amt=0) and stale multi-conn cache.
 */
#include <stdio.h>
struct P { char dbFileVers[16]; int x; };
int readlike(void *fd, void *buf, int amt, long long off) {
  (void)fd;
  (void)buf;
  (void)off;
  printf("readlike amt=%d off=%lld\n", amt, off);
  return amt == 16 ? 0 : 1;
}
int check(struct P *p) {
  char dbFileVers[sizeof(p->dbFileVers)];
  int rc = readlike(0, &dbFileVers, sizeof(dbFileVers), 24);
  int a = sizeof(p->dbFileVers);
  int b = sizeof(dbFileVers);
  printf("member=%d local=%d rc=%d\n", a, b, rc);
  return (a == 16 && b == 16 && rc == 0) ? 0 : 1;
}
int main(void) {
  struct P p = {{0}};
  return check(&p);
}
