/* Freestanding x86 link fillers — assembled by system as (not soft).
 * Soft drops C/asm defs for __head / func / var / empty initramfs bounds.
 */
	.weak __head
	.globl __head
	.data
	.align 8
__head:
	.quad 0

	.weak func
	.globl func
	.align 8
func:
	.quad 0

	.weak var
	.globl var
	.align 8
var:
	.quad 0

	.weak __initramfs_start
	.globl __initramfs_start
	.weak __initramfs_end
	.globl __initramfs_end
	.section .init.ramfs, "a"
__initramfs_start:
__initramfs_end:

	.weak __initramfs_size
	.globl __initramfs_size
	.section .init.ramfs.info, "a"
	.align 8
__initramfs_size:
	.quad 0
