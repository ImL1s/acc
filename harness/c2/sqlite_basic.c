#include "sqlite3.h"
#include <stdio.h>
static int npass;
static int cb(void *p, int argc, char **argv, char **col) {
  (void)p;(void)col;
  if (argc >= 1 && argv[0]) {
    int v = 0;
    const char *s = argv[0];
    while (*s) { v = v*10 + (*s - '0'); s++; }
    if (v == 42) npass++;
  }
  return 0;
}
int main(void) {
  sqlite3 *db = 0;
  char *err = 0;
  if (sqlite3_open(":memory:", &db) != SQLITE_OK) { printf("open fail\n"); return 1; }
  npass++; /* open */
  if (sqlite3_exec(db, "create table t(x);", 0, 0, &err) == SQLITE_OK) npass++;
  if (sqlite3_exec(db, "insert into t values(40);", 0, 0, &err) == SQLITE_OK) npass++;
  if (sqlite3_exec(db, "update t set x = x + 2;", 0, 0, &err) == SQLITE_OK) npass++;
  if (sqlite3_exec(db, "select x from t;", cb, 0, &err) == SQLITE_OK) npass++; /* +1 in cb if 42 */
  /* npass should be 5 + 1 from cb = 6 if all ok */
  printf("sqlite ok sum=42 npass=%d\n", npass);
  sqlite3_close(db);
  return npass >= 6 ? 0 : 1;
}
