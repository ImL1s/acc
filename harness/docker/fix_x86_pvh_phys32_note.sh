#!/usr/bin/env bash
# Patch Xen PHYS32_ENTRY note in vmlinux to pvh_start_xen physical address.
# soft/asm note expression truncates badly as .long; fix after link.
set -euo pipefail
VMLINUX="$(cd "$(dirname "$1")"; pwd)/$(basename "$1")"
[[ -f "$VMLINUX" ]] || { echo "missing $VMLINUX" >&2; exit 2; }

virt_hex=$(nm "$VMLINUX" | awk '/ T pvh_start_xen$/{print $1; exit}')
[[ -n "$virt_hex" ]] || { echo "pvh_start_xen not found" >&2; exit 2; }
python3 - "$VMLINUX" "$virt_hex" <<'PY'
import sys, struct
from pathlib import Path
path = Path(sys.argv[1])
virt = int(sys.argv[2], 16)
# __START_KERNEL_map on x86_64 tinyconfig
START_KERNEL_MAP = 0xFFFFFFFF80000000
phys = (virt - START_KERNEL_MAP) & 0xFFFFFFFF
data = bytearray(path.read_bytes())
# Find ELF note name "Xen\0" with type 18 (PHYS32_ENTRY)
# Note hdr: namesz, descsz, type (all u32 LE)
patched = 0
i = 0
needle = b"Xen\x00"
while True:
    j = data.find(needle, i)
    if j < 0:
        break
    # namesz field is 4 bytes before name (aligned); scan back for hdr
    # Standard: hdr at j-4 if namesz==4
    for back in (4, 8, 12, 0):
        off = j - back
        if off < 0:
            continue
        namesz, descsz, typ = struct.unpack_from("<III", data, off)
        if namesz == 4 and typ == 18 and 1 <= descsz <= 8:
            # desc follows name, padded to 4
            name_off = off + 12
            desc_off = name_off + ((namesz + 3) & ~3)
            if descsz >= 4:
                struct.pack_into("<I", data, desc_off, phys)
                # If descsz was 8, also clear high word
                if descsz >= 8:
                    struct.pack_into("<I", data, desc_off + 4, 0)
                # Force descsz to 4 for QEMU PHYS32
                struct.pack_into("<I", data, off + 4, 4)
                patched += 1
                print(f"patched note @{off:#x} phys={phys:#x} (virt={virt:#x})")
            break
    i = j + 1
if not patched:
    sys.exit("no Xen PHYS32 note patched")
path.write_bytes(data)
print(f"OK {path} patches={patched}")
PY
