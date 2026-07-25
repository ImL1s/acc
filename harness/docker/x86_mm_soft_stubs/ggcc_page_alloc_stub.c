/* AUTO-generated freestanding soft stub — do not hand-edit.
 * Soft fails ERROR: not an aggregate on upstream TU; PVH busybox path.
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
#define STUB4(n) WEAK long n(long a, long b, long c, long d) { (void)a;(void)b;(void)c;(void)d; return 0; }
#define STUB5(n) WEAK long n(long a, long b, long c, long d, long e) { (void)a;(void)b;(void)c;(void)d;(void)e; return 0; }
#define STUB6(n) WEAK long n(long a, long b, long c, long d, long e, long f) { (void)a;(void)b;(void)c;(void)d;(void)e;(void)f; return 0; }
#define STUB7(n) WEAK long n(long a, long b, long c, long d, long e, long f, long g) { (void)a;(void)b;(void)c;(void)d;(void)e;(void)f;(void)g; return 0; }
#define STUB8(n) WEAK long n(long a, long b, long c, long d, long e, long f, long g, long h) { (void)a;(void)b;(void)c;(void)d;(void)e;(void)f;(void)g;(void)h; return 0; }
/* stub for mm/page_alloc.c */
WEAK long gfp_allowed_mask;
WEAK long migratetype_names;
WEAK long min_free_kbytes;
WEAK long movable_zone;
WEAK long node_states;
WEAK long page_group_by_mobility_disabled;
WEAK long user_min_free_kbytes;
WEAK long vm_numa_stat_key;
WEAK long zone_names;
STUB2(__alloc_pages)
STUB4(__alloc_pages_bulk)
STUB2(__folio_alloc)
STUB4(__get_free_pages)
STUB4(__isolate_free_page)
STUB4(__page_frag_alloc_align)
STUB4(__page_frag_cache_drain)
STUB4(__pageblock_pfn_to_page)
STUB4(__putback_isolated_page)
STUB4(__zone_watermark_ok)
STUB4(adjust_managed_page_count)
STUB4(alloc_pages_exact)
STUB4(alloc_pages_exact_nid)
STUB4(build_all_zonelists)
STUB4(calculate_min_free_kbytes)
STUB4(decay_pcp_high)
STUB4(destroy_large_folio)
STUB4(drain_all_pages)
STUB4(drain_local_pages)
STUB4(find_suitable_fallback)
STUB4(free_contig_range)
STUB4(free_pages_exact)
STUB4(free_pages_prepare)
STUB4(free_reserved_area)
STUB4(free_unref_folios)
STUB4(get_pfnblock_flags_mask)
STUB4(get_zeroed_page)
STUB4(gfp_pfmemalloc_allowed)
STUB4(init_per_zone_wmark_min)
STUB4(is_free_buddy_page)
STUB4(move_freepages_block)
STUB4(nr_free_buffer_pages)
STUB0(page_alloc_sysctl_init)
STUB4(page_frag_cache_drain)
STUB4(page_frag_free)
STUB4(post_alloc_hook)
STUB4(prep_compound_page)
STUB4(set_pageblock_migratetype)
STUB4(set_pfnblock_flags_mask)
STUB4(setup_pcp_cacheinfo)
STUB4(setup_per_zone_wmarks)
STUB4(setup_zone_pageset)
STUB4(should_fail_alloc_page)
STUB4(split_free_page)
STUB4(split_page)
STUB4(warn_alloc)
STUB4(zone_pcp_disable)
STUB4(zone_pcp_enable)
STUB0(zone_pcp_init)
STUB4(zone_pcp_reset)
STUB4(zone_watermark_ok)
STUB4(zone_watermark_ok_safe)
