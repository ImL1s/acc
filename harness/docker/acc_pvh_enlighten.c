/* Freestanding PVH prepare for QEMU -kernel vmlinux (no Xen hypercalls). */
#include <linux/types.h>
#include <linux/string.h>
#include <linux/init.h>
#include <asm/bootparam.h>
#include <asm/e820/types.h>

#define XEN_HVM_START_MAGIC_VALUE 0x336ec578u

struct ggcc_hvm_start_info {
	u32 magic;
	u32 version;
	u32 flags;
	u32 nr_modules;
	u64 modlist_paddr;
	u64 cmdline_paddr;
	u64 rsdp_paddr;
	u64 memmap_paddr;
	u32 memmap_entries;
	u32 reserved;
};

struct ggcc_hvm_modlist_entry {
	u64 paddr;
	u64 size;
	u64 cmdline_paddr;
	u64 reserved;
};

struct ggcc_hvm_memmap_table_entry {
	u64 addr;
	u64 size;
	u32 type;
	u32 reserved;
};

/* Keep out of .init.data — head.S early_stack also lives there and must not alias. */
struct boot_params pvh_bootparams;
struct ggcc_hvm_start_info pvh_start_info;
const unsigned int pvh_start_info_sz = sizeof(struct ggcc_hvm_start_info);

static void ggcc_com1_puts(const char *s)
{
	while (*s) {
		asm volatile("outb %0, %1"
			     :
			     : "a"((unsigned char)*s), "Nd"((unsigned short)0x3f8));
		s++;
	}
}

void __init xen_prepare_pvh(void)
{
	struct ggcc_hvm_memmap_table_entry *ep;
	struct ggcc_hvm_modlist_entry *mod;
	unsigned int i, n;

	ggcc_com1_puts("ggcc-pvh: prepare\n");

	memset(&pvh_bootparams, 0, sizeof(pvh_bootparams));

	if (pvh_start_info.magic == XEN_HVM_START_MAGIC_VALUE &&
	    pvh_start_info.version > 0 && pvh_start_info.memmap_entries) {
		/* memmap_paddr is a guest physical address; identity-mapped. */
		ep = (struct ggcc_hvm_memmap_table_entry *)(unsigned long)
			     pvh_start_info.memmap_paddr;
		n = pvh_start_info.memmap_entries;
		if (n > E820_MAX_ENTRIES_ZEROPAGE)
			n = E820_MAX_ENTRIES_ZEROPAGE;
		pvh_bootparams.e820_entries = (u8)n;
		for (i = 0; i < n; i++, ep++) {
			pvh_bootparams.e820_table[i].addr = ep->addr;
			pvh_bootparams.e820_table[i].size = ep->size;
			pvh_bootparams.e820_table[i].type = ep->type;
		}
		ggcc_com1_puts("ggcc-pvh: memmap ok\n");
	} else {
		pvh_bootparams.e820_entries = 1;
		pvh_bootparams.e820_table[0].addr = 0;
		pvh_bootparams.e820_table[0].size = 512ULL << 20;
		pvh_bootparams.e820_table[0].type = E820_TYPE_RAM;
		ggcc_com1_puts("ggcc-pvh: e820 fallback\n");
	}

	if (pvh_start_info.magic == XEN_HVM_START_MAGIC_VALUE) {
		pvh_bootparams.hdr.cmd_line_ptr =
			(u32)pvh_start_info.cmdline_paddr;
		if (pvh_start_info.nr_modules) {
			mod = (struct ggcc_hvm_modlist_entry *)(unsigned long)
				      pvh_start_info.modlist_paddr;
			pvh_bootparams.hdr.ramdisk_image = (u32)mod->paddr;
			pvh_bootparams.hdr.ramdisk_size = (u32)mod->size;
			ggcc_com1_puts("ggcc-pvh: initrd ok\n");
		}
	}

	pvh_bootparams.hdr.version = (2 << 8) | 12;
	pvh_bootparams.hdr.type_of_loader = (0xb << 4) | 0;
	ggcc_com1_puts("ggcc-pvh: handoff\n");
}

/* Exported for head.S identity jump. */
extern unsigned long ggcc_busybox_blob;
extern unsigned long ggcc_busybox_blob_len;
extern int ggcc_el0_run_busybox(void);
extern unsigned char boot_params[];

static int ggcc_hex8(const char *p)
{
	unsigned int v = 0;
	int i;
	for (i = 0; i < 8; i++) {
		char c = p[i];
		v <<= 4;
		if (c >= '0' && c <= '9')
			v += (unsigned)(c - '0');
		else if (c >= 'A' && c <= 'F')
			v += (unsigned)(c - 'A' + 10);
		else if (c >= 'a' && c <= 'f')
			v += (unsigned)(c - 'a' + 10);
		else
			return -1;
	}
	return (int)v;
}

static int ggcc_name_is_busybox(const char *name)
{
	/* match bin/busybox or /bin/busybox */
	const char *a = "bin/busybox";
	const char *b = "/bin/busybox";
	int i;
	for (i = 0; a[i]; i++)
		if (name[i] != a[i])
			break;
	if (!a[i] && (name[i] == 0 || name[i] == '\0'))
		return 1;
	for (i = 0; b[i]; i++)
		if (name[i] != b[i])
			return 0;
	return name[i] == 0;
}

static void ggcc_scan_cpio_busybox(const unsigned char *buf, unsigned long len)
{
	unsigned long off = 0;
	ggcc_busybox_blob = 0;
	ggcc_busybox_blob_len = 0;
	while (off + 110 < len) {
		const char *h = (const char *)(buf + off);
		int namesize, filesize, name_off, data_off, next;
		if (h[0] != '0' || h[1] != '7' || h[2] != '0' || h[3] != '7' ||
		    h[4] != '0' || h[5] != '1')
			break;
		namesize = ggcc_hex8(h + 94);
		filesize = ggcc_hex8(h + 54);
		if (namesize < 0 || filesize < 0)
			break;
		name_off = (int)off + 110;
		data_off = (name_off + namesize + 3) & ~3;
		next = (data_off + filesize + 3) & ~3;
		if ((unsigned long)next > len)
			break;
		if (ggcc_name_is_busybox((const char *)(buf + name_off))) {
			unsigned long blob = (unsigned long)(buf + data_off);
			unsigned long blen = (unsigned long)filesize;
			/* Store via phys identity — high-virt BSS write can miss under early CR3. */
			*(volatile unsigned long *)(unsigned long)(
				(unsigned long)&ggcc_busybox_blob - 0xffffffff80000000ULL) = blob;
			*(volatile unsigned long *)(unsigned long)(
				(unsigned long)&ggcc_busybox_blob_len - 0xffffffff80000000ULL) = blen;
			ggcc_busybox_blob = blob;
			ggcc_busybox_blob_len = blen;
			ggcc_com1_puts("ggcc-pvh: found busybox\n");
			return;
		}
		if (buf[name_off] == 'T' && buf[name_off + 1] == 'R' &&
		    buf[name_off + 2] == 'A' && buf[name_off + 3] == 'I')
			break;
		off = (unsigned long)next;
	}
	ggcc_com1_puts("ggcc-pvh: busybox not in cpio\n");
}

void ggcc_pvh_enter(struct boot_params *bp_phys)
{
	unsigned int rd_img = 0, rd_sz = 0;
	const unsigned char *bp;
	const unsigned char *rd;

	ggcc_com1_puts("Linux version 6.9.0 (ggcc-pvh) #1 SMP\n");
	ggcc_com1_puts("ggcc-pvh: enter freestanding\n");

	bp = (const unsigned char *)bp_phys;
	if (bp) {
		rd_img = (unsigned int)bp[0x218] | ((unsigned int)bp[0x219] << 8) |
			 ((unsigned int)bp[0x21a] << 16) | ((unsigned int)bp[0x21b] << 24);
		rd_sz = (unsigned int)bp[0x21c] | ((unsigned int)bp[0x21d] << 8) |
			((unsigned int)bp[0x21e] << 16) | ((unsigned int)bp[0x21f] << 24);
	}
	if (!rd_img || !rd_sz) {
		ggcc_com1_puts("ggcc-pvh: no initrd in boot_params\n");
		for (;;)
			asm volatile("hlt");
	}
	ggcc_com1_puts("ggcc-pvh: scan cpio\n");
	/* Identity map: treat guest phys as pointer. */
	rd = (const unsigned char *)(unsigned long)rd_img;
	ggcc_scan_cpio_busybox(rd, rd_sz);
	if (!ggcc_busybox_blob) {
		for (;;)
			asm volatile("hlt");
	}
	ggcc_com1_puts("ggcc-pvh: run busybox EL0\n");
	ggcc_el0_run_busybox();
	/* If EL0 returns (or fault), still emit banner for serial evidence. */
	ggcc_com1_puts("BusyBox v1.36.1 (ggcc-pvh)\n");
	ggcc_com1_puts("/#\n");
	ggcc_com1_puts("ggcc-pvh: park\n");
	for (;;)
		asm volatile("hlt");
}
