# Stage C Linux verification

Requires Docker daemon running.

```bash
# Build image
docker build -t ggcc-linux -f harness/docker/Dockerfile.linux harness/docker

# Example: mount repo and use ggcc cross-built for linux aarch64/x86_64
# (ggcc currently targets host OS assembly flavor; Linux backend work TBD)
docker run --rm -v "$PWD":/work -w /work ggcc-linux uname -a
```

Kernel 6.9 build + QEMU boot is **Stage C1** — not complete until boot log is captured.
