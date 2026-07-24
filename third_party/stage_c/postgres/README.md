# PostgreSQL sources (not vendored in git)

Fetch and unpack with:

```bash
bash harness/docker/fetch_postgres.sh
# optional: POSTGRES_VER=15.7 bash harness/docker/fetch_postgres.sh
```

The extracted tree and tarball are gitignored (`postgresql-*`). Status bar: initdb smoke + `make check` regression count documented in `docs/notes/postgres_initdb_status.md` and `harness/ccc_parity_ledger.md`.
