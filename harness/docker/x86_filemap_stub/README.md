# x86_filemap_stub

Replaces `mm/filemap.o` with `ggcc_filemap_stub.o` for freestanding C1 x86_64.

Soft cannot compile upstream `mm/filemap.c` (`ERROR: not an aggregate`).
Installed by `c1_build_inner.sh` (same spirit as `x86_dma_stub`).
