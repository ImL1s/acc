/* AUTO stub sparse-vmemmap */
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
STUB4(__populate_section_memmap)
STUB4(vmemmap_alloc_block)
STUB4(vmemmap_alloc_block_buf)
STUB4(vmemmap_p4d_populate)
STUB4(vmemmap_pgd_populate)
STUB4(vmemmap_pmd_populate)
STUB4(vmemmap_populate_basepages)
STUB4(vmemmap_populate_hugepages)
STUB4(vmemmap_pte_populate)
STUB4(vmemmap_pud_populate)
STUB4(vmemmap_verify)
