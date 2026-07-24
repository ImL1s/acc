#!/usr/bin/env bash
# Stage C2: attempt SQLite amalgamation with host-produced Linux asm (no external C on .c).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="${SCRATCH:-/tmp/acc-scratch}"
ACC="${ACC_BIN:-$ROOT/target/release/acc}"
SQL="$ROOT/third_party/stage_c/sqlite"
mkdir -p "$SCRATCH/sqlite"
# Compile shell + amalgamation is multi-file; for smoke: one-file program using sqlite3
cat > "$SCRATCH/sqlite/smain.c" <<'C'
#include "sqlite3.h"
#include <stdio.h>
int main(void){
  sqlite3 *db=0;
  char *err=0;
  if(sqlite3_open(":memory:", &db)!=SQLITE_OK){printf("open fail\n");return 1;}
  if(sqlite3_exec(db,"create table t(x); insert into t values(1);",0,0,&err)!=SQLITE_OK){
    printf("exec fail %s\n", err?err:""); return 2;
  }
  printf("sqlite_smoke_ok\n");
  sqlite3_close(db);
  return 0;
}
C
# Preprocess/include won't pull sqlite3.h path — copy
cp "$SQL/sqlite3.h" "$SCRATCH/sqlite/" 2>/dev/null || true
# Try compiling amalgamation alone first
set +e
"$ACC" --target-os linux -S -o "$SCRATCH/sqlite/sqlite3.s" "$SQL/sqlite3.c" 2>"$SCRATCH/sqlite/sqlite3_err.txt"
ec1=$?
"$ACC" --target-os linux -S -o "$SCRATCH/sqlite/smain.s" "$SCRATCH/sqlite/smain.c" 2>"$SCRATCH/sqlite/smain_err.txt"
ec2=$?
echo "acc_sqlite3_s_ec=$ec1"
echo "acc_smain_s_ec=$ec2"
head -20 "$SCRATCH/sqlite/sqlite3_err.txt"
head -20 "$SCRATCH/sqlite/smain_err.txt"
# If both .s exist, assemble in docker
if [[ $ec1 -eq 0 && $ec2 -eq 0 ]]; then
  docker run --rm -v "$SCRATCH/sqlite":/w -w /w acc-linux \
    bash -lc 'cc -o smoke smain.s sqlite3.s -lm && ./smoke'
fi
