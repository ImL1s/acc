/* GGCC_SKIP_FILEMAP — freestanding C1 x86_64 stub for mm/filemap.c
 * Soft fails with ERROR: not an aggregate on the real TU.
 * PVH → busybox path does not need page-cache filemap; provide linkable
 * no-op exports so the rest of mm/ can build.
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

/* Data exports referenced by mm/fs */
WEAK int sysctl_page_lock_unfairness;
/* vm_operations_struct stand-in — never called on freestanding busybox path */
WEAK char generic_file_vm_ops[128];

STUB0(pagecache_init)

STUB2(__filemap_add_folio)
STUB3(__filemap_fdatawrite_range)
STUB3(__filemap_get_folio)
STUB2(__filemap_remove_folio)
STUB2(__filemap_set_wb_err)
STUB1(__folio_lock)
STUB1(__folio_lock_killable)
STUB2(__folio_lock_or_retry)
STUB2(__generic_file_write_iter)

STUB2(delete_from_page_cache_batch)
STUB1(file_check_and_advance_wb_err)
STUB3(file_fdatawait_range)
STUB3(file_write_and_wait_range)

STUB2(filemap_add_folio)
STUB1(filemap_check_errors)
STUB1(filemap_fault)
STUB1(filemap_fdatawait_keep_errors)
STUB3(filemap_fdatawait_range)
STUB3(filemap_fdatawait_range_keep_errors)
STUB1(filemap_fdatawrite)
STUB3(filemap_fdatawrite_range)
STUB2(filemap_fdatawrite_wbc)
STUB1(filemap_flush)
STUB2(filemap_free_folio)
STUB2(filemap_get_entry)
STUB4(filemap_get_folios)
STUB4(filemap_get_folios_contig)
STUB5(filemap_get_folios_tag)
STUB2(filemap_invalidate_lock_two)
STUB2(filemap_invalidate_unlock_two)
STUB1(filemap_map_pages)
STUB1(filemap_page_mkwrite)
STUB3(filemap_range_has_page)
STUB3(filemap_range_has_writeback)
STUB2(filemap_read)
STUB1(filemap_release_folio)
STUB1(filemap_remove_folio)
STUB2(filemap_splice_read)
STUB3(filemap_write_and_wait_range)

STUB4(find_get_entries)
STUB4(find_lock_entries)

STUB2(folio_add_wait_queue)
STUB1(folio_end_private_2)
STUB2(folio_end_read)
STUB1(folio_end_writeback)
STUB1(folio_unlock)
STUB2(folio_wait_bit)
STUB2(folio_wait_bit_killable)
STUB1(folio_wait_private_2)
STUB1(folio_wait_private_2_killable)

STUB2(generic_file_direct_write)
STUB1(generic_file_mmap)
STUB2(generic_file_read_iter)
STUB1(generic_file_readonly_mmap)
STUB2(generic_file_write_iter)
STUB2(generic_perform_write)

STUB1(kiocb_invalidate_pages)
STUB1(kiocb_invalidate_post_direct_write)
STUB1(kiocb_write_and_wait)

STUB3(mapping_read_folio_gfp)
STUB3(mapping_seek_hole_data)

STUB2(page_cache_next_miss)
STUB2(page_cache_prev_miss)

STUB2(read_cache_folio)
STUB2(read_cache_page)
STUB3(read_cache_page_gfp)
STUB2(replace_page_cache_folio)
STUB2(splice_folio_into_pipe)
