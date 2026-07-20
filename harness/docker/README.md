# Stage C Linux verification

Requires a running **Docker daemon** (macOS host cannot claim C1 without Linux Docker/VM + QEMU).

## Build image

```bash
docker build -t ggcc-linux -f harness/docker/Dockerfile.linux harness/docker
```

Image includes: build-essential, flex/bison/bc, libelf/ssl, QEMU (x86 + aarch64), rustc/cargo, xz.

**Important:** build `ggcc` **inside** the container. Host `target/release/ggcc` on macOS is Mach-O and will not execute under Linux.

## Stage C1 — kernel 6.9 + QEMU

```bash
export SCRATCH="${SCRATCH:-/tmp/ggcc-c1}"
mkdir -p "$SCRATCH"
bash harness/docker/build_kernel.sh
```

What the script does:

1. Builds/caches `ggcc-linux` image  
2. Fetches **Linux 6.9** → `third_party/linux-6.9` (if missing)  
3. Host freestanding `kstub` asm smoke (not boot proof)  
4. In Docker: `cargo build --release`, `make tinyconfig`, `make CC=ggcc_cc_wrapper HOSTCC=gcc`  
5. If `bzImage`/`Image`/`vmlinux` appears → QEMU serial capture  
6. Writes `$SCRATCH/stage_c_kernel.log` with **`VERDICT:`** and **`blocked_reason:`**

### CC wrapper

`ggcc_cc_wrapper.sh` is the only `$CC` for kernel **C** sources:

| Input | Tool |
|-------|------|
| `.c` | **ggcc only** → `.s`, then system assemble |
| `.S` / `.s` / link | system `cc` / `as` / `ld` |
| kconfig host tools | `HOSTCC=gcc` (not kernel `.c`) |

**Forbidden:** feeding kernel `.c` to gcc/clang.

### Honest status

Full C1 boot is **BLOCKED** until ggcc language/CLI coverage catches kernel C. See [KERNEL_STATUS.md](./KERNEL_STATUS.md). Do not treat kstub or image build alone as PASS.

## SQLite smoke (C2 helper)

```bash
bash harness/docker/run_sqlite_smoke.sh
```
