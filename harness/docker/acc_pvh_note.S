/* Minimal Xen PVH ELF note so QEMU accepts uncompressed ELF -kernel.
 *
 * XEN_ELFNOTE_PHYS32_ENTRY (type 18 / 0x12): 32-bit physical entry address.
 * Value = CONFIG_PHYSICAL_START default (0x1000000 / 16MiB) — same load
 * address as startup_64 on tinyconfig x86_64. This clears QEMU's Xen-note
 * check; it is NOT a real pvh_start_xen trampoline (32-bit PE handoff still
 * needs more for a clean boot).
 *
 * Layout matches include/linux/elfnote.h ELFNOTE(Xen, …) shape:
 *   namesz=4 ("Xen\0"), descsz=4 (.long), type=18.
 */
	.section .note.Xen, "a", @note
	.balign 4
	.long 1f - 0f			/* namesz */
	.long 3f - 2f			/* descsz */
	.long 18			/* XEN_ELFNOTE_PHYS32_ENTRY */
0:	.asciz "Xen"
1:	.balign 4
2:	.long 0x1000000			/* phys entry = CONFIG_PHYSICAL_START */
3:	.balign 4
