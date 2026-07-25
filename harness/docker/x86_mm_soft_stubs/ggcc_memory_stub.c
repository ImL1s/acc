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
/* stub for mm/memory.c */
WEAK long high_memory;
WEAK long highest_memmap_pfn;
WEAK long max_mapnr;
WEAK long mem_map;
WEAK long randomize_va_space;
WEAK long zero_pfn;
STUB4(__get_locked_pte)
STUB2(__pmd_alloc)
STUB2(__pte_alloc)
STUB4(__pte_alloc_kernel)
STUB2(__pud_alloc)
STUB4(access_process_vm)
STUB4(access_remote_vm)
STUB4(apply_to_existing_page_range)
STUB4(apply_to_page_range)
STUB4(copy_page_range)
STUB4(do_set_pmd)
STUB4(do_swap_page)
STUB4(finish_fault)
STUB4(follow_pfn)
STUB4(follow_phys)
STUB4(follow_pte)
STUB4(free_pgd_range)
STUB4(free_pgtables)
STUB4(generic_access_phys)
STUB4(handle_mm_fault)
STUB4(lock_mm_and_find_vma)
STUB4(lock_vma_under_rcu)
STUB4(mm_trace_rss_stat)
STUB4(numa_migrate_prep)
STUB4(pmd_install)
STUB4(print_vma_addr)
STUB4(remap_pfn_range)
STUB4(remap_pfn_range_notrack)
STUB4(set_pte_range)
STUB4(unmap_mapping_folio)
STUB4(unmap_mapping_pages)
STUB4(unmap_mapping_range)
STUB4(unmap_page_range)
STUB4(unmap_vmas)
STUB4(vm_insert_page)
STUB4(vm_insert_pages)
STUB4(vm_iomap_memory)
STUB4(vm_map_pages)
STUB4(vm_map_pages_zero)
STUB4(vm_normal_folio)
STUB4(vm_normal_page)
STUB4(vmf_anon_prepare)
STUB4(vmf_insert_mixed)
STUB4(vmf_insert_mixed_mkwrite)
STUB4(vmf_insert_pfn)
STUB4(vmf_insert_pfn_prot)
STUB4(zap_page_range_single)
STUB4(zap_vma_ptes)
