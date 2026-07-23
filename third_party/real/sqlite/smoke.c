#include "sqlite3.h"
#include <stdio.h>
int main(void) {
  sqlite3 *db = 0;
  char *err = 0;
  int rc = sqlite3_open(":memory:", &db);
  if (rc != SQLITE_OK) {
    printf("open fail %d\n", rc);
    return 1;
  }
  rc = sqlite3_exec(db, "CREATE TABLE t(x INTEGER); INSERT INTO t VALUES(40); INSERT INTO t VALUES(2);", 0, 0, &err);
  if (rc != SQLITE_OK) {
    printf("exec fail %s\n", err ? err : "?");
    return 2;
  }
  sqlite3_stmt *st = 0;
  rc = sqlite3_prepare_v2(db, "SELECT SUM(x) FROM t;", -1, &st, 0);
  if (rc != SQLITE_OK) {
    printf("prepare fail %d\n", rc);
    return 3;
  }
  rc = sqlite3_step(st);
  if (rc != SQLITE_ROW) {
    printf("step fail %d\n", rc);
    return 4;
  }
  int sum = sqlite3_column_int(st, 0);
  sqlite3_finalize(st);
  sqlite3_close(db);
  printf("sqlite ok sum=%d\n", sum);
  return sum == 42 ? 0 : 5;
}
