/* Soft unit: PG-style circular dlist (prev before next) iteration.
 * Mirrors pgstat_flush_pending_entries walking with has_next/next.
 */
#include <stdio.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

typedef struct dlist_node dlist_node;
struct dlist_node {
	dlist_node *prev;
	dlist_node *next;
};

typedef struct dlist_head {
	dlist_node head;
} dlist_head;

typedef struct {
	int id;
	dlist_node node;
} Item;

static void
dlist_init(dlist_head *h)
{
	h->head.next = h->head.prev = &h->head;
}

static int
dlist_is_empty(dlist_head *h)
{
	return h->head.next == NULL || h->head.next == &h->head;
}

static void
dlist_push_tail(dlist_head *h, dlist_node *n)
{
	if (h->head.next == NULL)
		dlist_init(h);
	n->next = &h->head;
	n->prev = h->head.prev;
	n->prev->next = n;
	h->head.prev = n;
}

static int
dlist_has_next(dlist_head *h, dlist_node *n)
{
	return n->next != &h->head;
}

static dlist_node *
dlist_next_node(dlist_head *h, dlist_node *n)
{
	(void) h;
	return n->next;
}

static dlist_node *
dlist_head_node(dlist_head *h)
{
	return h->head.next;
}

static void
dlist_delete(dlist_node *n)
{
	n->prev->next = n->next;
	n->next->prev = n->prev;
	n->next = n->prev = NULL;
}

int
main(void)
{
	dlist_head pending;
	Item a, b, c;
	dlist_node *cur;
	int steps = 0;
	int sum = 0;

	memset(&pending, 0, sizeof(pending));
	a.id = 1;
	b.id = 2;
	c.id = 3;
	dlist_init(&pending);
	dlist_push_tail(&pending, &a.node);
	dlist_push_tail(&pending, &b.node);
	dlist_push_tail(&pending, &c.node);

	if (dlist_is_empty(&pending)) {
		fprintf(stderr, "empty after push\n");
		return 1;
	}

	cur = dlist_head_node(&pending);
	while (cur) {
		Item *it = (Item *) ((char *) cur - offsetof(Item, node));
		dlist_node *next;

		sum += it->id;
		steps++;
		if (steps > 10) {
			fprintf(stderr, "infinite walk\n");
			return 2;
		}
		if (dlist_has_next(&pending, cur))
			next = dlist_next_node(&pending, cur);
		else
			next = NULL;
		dlist_delete(cur);
		cur = next;
	}

	if (steps != 3 || sum != 6 || !dlist_is_empty(&pending)) {
		fprintf(stderr, "fail steps=%d sum=%d empty=%d\n",
			steps, sum, dlist_is_empty(&pending));
		return 3;
	}

	/* Address identity: &head->head must equal (dlist_node *)head */
	if (&pending.head != (dlist_node *) &pending) {
		fprintf(stderr, "head address identity broken\n");
		return 4;
	}
	return 0;
}
