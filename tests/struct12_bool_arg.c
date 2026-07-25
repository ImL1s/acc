/* Soft unit: 12-byte struct by value + trailing bool (SysV rdi/rsi + rdx + rcx).
 *
 * Mirrors pgstat_release_entry_ref(PgStat_HashKey, void *, bool discard_pending):
 * if the bool lands in the wrong register, discard_pending=true is read as false
 * and a "pending" release path errors.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

typedef struct {
	uint32_t kind;
	uint32_t dboid;
	uint32_t objoid;
} HashKey12;

typedef struct {
	void *shared_entry;
	void *shared_stats;
	void *pending;
	void *node_next;
	void *node_prev;
} EntryRef;

static int g_errors;
static int g_discards;

static void
release_entry_ref(HashKey12 key, EntryRef *entry_ref, int discard_pending)
{
	(void) key;
	if (entry_ref && entry_ref->pending) {
		if (discard_pending)
			g_discards++;
		else
			g_errors++;
	}
}

static void
drop_like(uint32_t kind, uint32_t dboid, uint32_t objoid, EntryRef *ref)
{
	HashKey12 key = {.kind = kind, .dboid = dboid, .objoid = objoid};
	/* Must pass discard_pending=1 in the bool slot after the 12B key. */
	release_entry_ref(key, ref, 1);
}

int
main(void)
{
	EntryRef ref;
	int pending_space = 0x51;

	memset(&ref, 0, sizeof(ref));
	ref.pending = &pending_space;

	drop_like(1, 1, 0, &ref);

	if (g_errors != 0 || g_discards != 1) {
		fprintf(stderr, "fail: errors=%d discards=%d (want 0,1)\n",
			g_errors, g_discards);
		return 1;
	}
	return 0;
}
