# Stage C1 — Linux 6.9 + QEMU (honest status)

**Gate:** Compile+link bootable Linux **6.9** with `CC=ggcc` (via wrapper), verify with **Linux Docker + QEMU** serial log.  
**Evidence file:** `{SCRATCH}/stage_c_kernel.log` must contain boot proof and a `VERDICT: PASS` only when boot strings appear.

## Current status: **BLOCKED** (preparation ready; full boot not claimed)

| Piece | Status |
|-------|--------|
| Docker image `ggcc-linux` | Scripted (`Dockerfile.linux` + `build_kernel.sh`) |
| Fetch linux-6.9 | Scripted → `third_party/linux-6.9` |
| Minimal config | `make ARCH=… tinyconfig` inside container |
| `CC=ggcc` for kernel `.c` | `ggcc_cc_wrapper.sh` (ggcc only; **no gcc on `.c`**) |
| System `as`/`ld`/`cc` | Used only for `.S`/`.s`/`.o` assemble & link |
| `HOSTCC=gcc` | Allowed for kconfig/fixdep **host** tools only |
| In-Docker `cargo build` | Required (host macOS ggcc is Mach-O) |
| Full kernel `.o` / bzImage | **Not expected yet** — language gap |
| QEMU boot evidence | Path wired; runs only if image exists |

## Wrapper contract

`harness/docker/ggcc_cc_wrapper.sh`:

1. **`.c` → ggcc** (`--target-os linux -m … -S`) then **system `cc -c`** on emitted `.s` only.
2. **`.S`/`.s`/link** → system toolchain only.
3. **Never** pass kernel `.c` to gcc/clang as the compiler.
4. Strips unknown gcc flags before calling ggcc (ggcc CLI is flag-minimal).
5. Probes (`--version`, `-dumpmachine`) answered without compiling.

## Why full compile is blocked (language / CLI)

ggcc today is Stage B–class. Kernel 6.9 needs far more, including roughly:

- Full `#include` search paths / system headers / generated `include/generated`
- `-E` / dependency generation (`-MD`) without gcc fallback
- GNU attributes (`__attribute__`, `__section__`, `__aligned__`, …)
- Inline assembly (`asm volatile`, constraints)
- `__builtin_*`, `__noreturn`, statement expressions, `typeof`, `empty structs`, bitfields at scale
- Freestanding / `-ffreestanding` codegen expectations
- Huge translation units and Kbuild-generated headers

Until those work, **honest `VERDICT: BLOCKED`** is required. Partial freestanding stubs (`kstub.c`) prove Linux ELF asm dialect only — **not** C1.

## How to run (parent / `omg accept`)

```bash
export SCRATCH=/path/to/evidence   # required
# optional: KERNEL_ARCH=x86_64 JOBS=4
bash harness/docker/build_kernel.sh
# log: $SCRATCH/stage_c_kernel.log
# full make log: $SCRATCH/kernel_make_full.log
# qemu (only if image built): $SCRATCH/qemu_boot.log
```

## Definition of done (do not lower)

1. `make` with `CC=ggcc_cc_wrapper` produces bootable image for ≥1 arch.
2. QEMU serial contains kernel start evidence (e.g. `Linux version`).
3. Log shows `VERDICT: PASS` and no gcc-on-`.c` bypass.
4. `harness/progress.md` C1 flipped only after the above.
