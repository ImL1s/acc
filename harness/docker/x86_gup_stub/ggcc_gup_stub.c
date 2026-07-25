/* GGCC_SKIP_GUP — freestanding C1 x86_64 stub for mm/gup.c
 * Soft fails with ERROR: not an aggregate on the real TU.
 */
#ifdef __GNUC__
#define WEAK __attribute__((weak))
#else
#define WEAK
#endif

#define STUB0(n) WEAK long n(void) { return 0; }
#define STUB1(n) WEAK long n(long a) { (void)a; return 0; }
#define STUB2(n) WEAK long n(long a, long b) { (void)a; (void)b; return 0; }
#define STUB3(n) WEAK long n(long a, long b, long c) { (void)a; (void)b; (void)c; return 0; }
#define STUB4(n) WEAK long n(long a, long b, long c, long d) \
	{ (void)a; (void)b; (void)c; (void)d; return 0; }
#define STUB5(n) WEAK long n(long a, long b, long c, long d, long e) \
	{ (void)a; (void)b; (void)c; (void)d; (void)e; return 0; }
#define STUB6(n) WEAK long n(long a, long b, long c, long d, long e, long f) \
	{ (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; return 0; }

STUB2(__mm_populate)
STUB3(fault_in_readable)
STUB3(fault_in_safe_writeable)
STUB3(fault_in_subpage_writeable)
STUB3(fault_in_writeable)
STUB5(faultin_page_range)
STUB4(fixup_user_fault)
STUB2(folio_add_pin)
STUB3(follow_page)
STUB6(get_user_pages)
STUB4(get_user_pages_fast)
STUB3(get_user_pages_fast_only)
STUB6(get_user_pages_remote)
STUB5(get_user_pages_unlocked)
STUB6(pin_user_pages)
STUB4(pin_user_pages_fast)
STUB6(pin_user_pages_remote)
STUB5(pin_user_pages_unlocked)
STUB4(populate_vma_page_range)
STUB3(try_grab_folio)
STUB2(try_grab_page)
STUB1(unpin_user_page)
STUB3(unpin_user_page_range_dirty_lock)
STUB2(unpin_user_pages)
STUB3(unpin_user_pages_dirty_lock)
