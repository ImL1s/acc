/* Soft unit: bool *found out-param from "hash insert" returning existing entry.
 * If caller reads found wrong, pgstat_get_entry_ref_cached can return false
 * for a live pending ref and later release(..., false) FATAL.
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
	uint32_t kind, dboid, objoid;
} Key12;

typedef struct {
	Key12 key;
	char status;
	void *entry_ref;
} HashEnt;

static HashEnt g_slot;
static int g_has;

static HashEnt *
fake_insert(Key12 key, int *found)
{
	if (g_has &&
		g_slot.key.kind == key.kind &&
		g_slot.key.dboid == key.dboid &&
		g_slot.key.objoid == key.objoid) {
		*found = 1;
		return &g_slot;
	}
	g_slot.key = key;
	g_slot.status = 1;
	g_slot.entry_ref = (void *) (uintptr_t) 0x1111;
	g_has = 1;
	*found = 0;
	return &g_slot;
}

static int
cached_lookup(Key12 key, void **out_ref)
{
	int found = 0;
	HashEnt *ent = fake_insert(key, &found);

	if (!found || !ent->entry_ref) {
		ent->entry_ref = (void *) (uintptr_t) 0x2222;
		found = 0;
	} else {
		/* existing live entry — found must stay true */
	}
	*out_ref = ent->entry_ref;
	return found;
}

int
main(void)
{
	Key12 key = {.kind = 1, .dboid = 1, .objoid = 0};
	void *ref = NULL;
	int f1, f2;

	f1 = cached_lookup(key, &ref);
	if (f1 != 0 || ref != (void *) (uintptr_t) 0x2222) {
		fprintf(stderr, "first: found=%d ref=%p\n", f1, ref);
		return 1;
	}

	/* Second insert must report found=1 and keep live ref */
	ref = NULL;
	f2 = cached_lookup(key, &ref);
	if (f2 != 1 || ref != (void *) (uintptr_t) 0x2222) {
		fprintf(stderr, "second: found=%d ref=%p (want found=1 live)\n", f2, ref);
		return 2;
	}
	return 0;
}
