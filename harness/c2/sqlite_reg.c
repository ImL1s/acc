#include "sqlite3.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static int npass, nfail;
static char last_val[256];

static int scalar_cb(void *p, int argc, char **argv, char **col)
{
	(void)p;
	(void)col;
	if (argc >= 1 && argv[0]) {
		strncpy(last_val, argv[0], sizeof(last_val) - 1);
		last_val[sizeof(last_val) - 1] = 0;
	} else {
		last_val[0] = 0;
	}
	return 0;
}

static void check_sql(sqlite3 *db, const char *sql, const char *expect,
		      const char *name)
{
	char *err = 0;
	last_val[0] = 0;
	if (sqlite3_exec(db, sql, scalar_cb, 0, &err) != SQLITE_OK) {
		printf("FAIL %s exec: %s\n", name, err ? err : "?");
		sqlite3_free(err);
		nfail++;
		return;
	}
	if (expect) {
		if (strcmp(last_val, expect) == 0) {
			printf("PASS %s\n", name);
			npass++;
		} else {
			printf("FAIL %s got=%s expect=%s\n", name, last_val,
			       expect);
			nfail++;
		}
	} else {
		printf("PASS %s\n", name);
		npass++;
	}
}

static void check_ok(sqlite3 *db, const char *sql, const char *name)
{
	check_sql(db, sql, 0, name);
}

int main(void)
{
	sqlite3 *db = 0;

	if (sqlite3_open(":memory:", &db) != SQLITE_OK) {
		printf("FAIL open\n");
		return 1;
	}
	npass++;
	printf("PASS open\n");

	check_ok(db,
		 "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT, val REAL);",
		 "create");
	check_ok(db, "INSERT INTO t VALUES(1,'alice',1.5);", "insert1");
	check_ok(db, "INSERT INTO t VALUES(2,'bob',2.5);", "insert2");
	check_ok(db, "INSERT INTO t VALUES(3,'carol',3.25);", "insert3");
	check_sql(db, "SELECT COUNT(*) FROM t;", "3", "count");
	check_sql(db, "SELECT name FROM t WHERE id=2;", "bob", "where");
	check_sql(db, "SELECT SUM(val) FROM t;", "7.25", "sum");
	check_sql(db, "SELECT name FROM t ORDER BY name DESC LIMIT 1;", "carol",
		  "order_limit");
	check_ok(db, "UPDATE t SET val=val+1 WHERE id=1;", "update");
	check_sql(db, "SELECT val FROM t WHERE id=1;", "2.5", "update_check");
	check_ok(db, "DELETE FROM t WHERE id=3;", "delete");
	check_sql(db, "SELECT COUNT(*) FROM t;", "2", "count_after_del");
	check_ok(db, "CREATE INDEX idx_name ON t(name);", "index");
	check_sql(db, "SELECT id FROM t WHERE name='alice';", "1",
		  "index_lookup");
	check_ok(db, "BEGIN;", "begin");
	check_ok(db, "INSERT INTO t VALUES(4,'dave',9.0);", "tx_insert");
	check_ok(db, "ROLLBACK;", "rollback");
	check_sql(db, "SELECT COUNT(*) FROM t;", "2", "count_after_rb");
	check_ok(db, "BEGIN;", "begin2");
	check_ok(db, "INSERT INTO t VALUES(5,'eve',8.0);", "tx_insert2");
	check_ok(db, "COMMIT;", "commit");
	check_sql(db, "SELECT COUNT(*) FROM t;", "3", "count_after_commit");
	check_ok(db, "CREATE TABLE u(x TEXT);", "create_u");
	check_ok(db, "INSERT INTO u SELECT name FROM t;", "insert_select");
	check_sql(db, "SELECT COUNT(*) FROM u;", "3", "u_count");
	check_sql(db, "SELECT LENGTH('hello');", "5", "length");
	check_sql(db, "SELECT UPPER('AbC');", "ABC", "upper");
	check_sql(db, "SELECT 40+2;", "42", "arith");
	check_sql(db, "SELECT CASE WHEN 1 THEN 'yes' ELSE 'no' END;", "yes",
		  "case");
	/* After commit: alice=2.5, bob=2.5, eve=8.0 → all three match val>2 */
	check_ok(db, "CREATE VIEW v AS SELECT name FROM t WHERE val > 2;",
		 "view");
	check_sql(db, "SELECT COUNT(*) FROM v;", "3", "view_count");
	check_ok(db, "CREATE TABLE j(a INT, b INT);", "join_t");
	check_ok(db, "INSERT INTO j VALUES(1,10),(2,20);", "join_ins");
	check_sql(db, "SELECT t.name FROM t JOIN j ON t.id=j.a WHERE j.b=20;",
		  "bob", "join");
	check_ok(db, "SELECT GROUP_CONCAT(name) FROM t;", "group_concat");
	check_ok(db, "PRAGMA table_info(t);", "pragma");
	check_sql(db, "SELECT sqlite_version() IS NOT NULL;", "1", "version");

	printf("sqlite_reg npass=%d nfail=%d\n", npass, nfail);
	sqlite3_close(db);
	return nfail == 0 && npass >= 30 ? 0 : 1;
}
