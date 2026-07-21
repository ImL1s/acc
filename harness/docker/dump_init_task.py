#!/usr/bin/env python3
"""Dump pcpu_hot / init_task from vmlinux for C1 boot debugging."""
import struct
import sys

path = sys.argv[1] if len(sys.argv) > 1 else "vmlinux"
map_path = sys.argv[2] if len(sys.argv) > 2 else "System.map"

syms = {}
with open(map_path) as f:
    for line in f:
        parts = line.split()
        if len(parts) >= 3:
            try:
                syms[parts[2]] = int(parts[0], 16)
            except ValueError:
                pass

it = syms["init_task"]
end = syms.get("__end_init_task", 0)
start = syms.get("__start_init_task", 0)
pcpu = syms["pcpu_hot"]
print("init_task", hex(it), "start", hex(start), "end", hex(end), "size", (end - start) if end else "?")
print("pcpu_hot", hex(pcpu))

with open(path, "rb") as f:
    data = f.read()

e_shoff = struct.unpack_from("<Q", data, 40)[0]
e_shentsize, e_shnum, e_shstrndx = struct.unpack_from("<HHH", data, 58)


def sh(i):
    return struct.unpack_from("<IIQQQQIIQQ", data, e_shoff + i * e_shentsize)


names = data[sh(e_shstrndx)[4] : sh(e_shstrndx)[4] + sh(e_shstrndx)[5]]
secs = []
for i in range(e_shnum):
    h = sh(i)
    nm = names[h[0] :].split(b"\0", 1)[0].decode(errors="replace")
    secs.append((nm, h[1], h[3], h[4], h[5]))


def file_off(addr):
    for n, t, a, o, sz in secs:
        if t != 8 and sz and a and a <= addr < a + sz:
            return o + (addr - a), n
    return None, None


fo, sec = file_off(it)
print("init_task file_off", hex(fo) if fo is not None else None, "sec", sec)
blob = data[fo : fo + 2048]

candidates = []
for off in range(0, len(blob) - 7, 8):
    q = struct.unpack_from("<Q", blob, off)[0]
    if start and end and start <= q <= end + 0x100:
        candidates.append((off, q))
    elif end and abs(q - end) < 0x400:
        candidates.append((off, q))

print("pointers into init_stack region:")
for off, q in candidates[:40]:
    print(f"  +{off:4d} (+{off:#x}): {q:#x}")

print("at TASK_threadsp 1496:", hex(struct.unpack_from("<Q", blob, 1496)[0]))

fo2, sec2 = file_off(pcpu)
blob2 = data[fo2 : fo2 + 64]
print("pcpu_hot sec", sec2)
for off in range(0, 64, 8):
    q = struct.unpack_from("<Q", blob2, off)[0]
    print(f"  +{off:02x}: {q:016x}")

if end:
    for pad in (0, 8, 16):
        for ptsz in (168, 160, 176, 184, 192, 208, 216):
            pred = end - pad - ptsz
            print(f"  predict pad={pad} ptsz={ptsz}: {hex(pred)}")
