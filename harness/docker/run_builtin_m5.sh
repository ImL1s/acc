#!/usr/bin/env bash
# M5: hosted hello via builtin assembler + builtin linker (static musl), no system cc/ld.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
IMAGE="${ACC_DOCKER_IMAGE:-${GGCC_DOCKER_IMAGE:-ggcc-linux-arm64}}"
PLATFORM="${ACC_DOCKER_PLATFORM:-${GGCC_DOCKER_PLATFORM:-linux/arm64}}"
STAMP="$ROOT/scratch/builtin_m5_marker"
LOG="$ROOT/scratch/builtin_m5_run.log"
PROG="$ROOT/scratch/builtin_m5_prog"
SRC="$ROOT/tests/builtin_m5_hello.c"

mkdir -p "$ROOT/scratch"

exec > >(tee "$LOG") 2>&1

echo "=== builtin M5 hosted link smoke ==="
echo "image=$IMAGE platform=$PLATFORM"

docker run --rm --platform "$PLATFORM" -v "$ROOT:/work" -w /work "$IMAGE" bash -lc '
  set -euo pipefail
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq curl build-essential musl-dev musl-tools binutils strace 2>/dev/null | tail -1
  export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
  export PATH=/usr/local/cargo/bin:$PATH
  if [ ! -x /usr/local/cargo/bin/cargo ]; then
    curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --no-modify-path
  fi
  export CARGO_TARGET_DIR=/work/target-linux-arm64
  cargo build --features builtin_linker --release

  ACC=/work/target-linux-arm64/release/acc
  test -x "$ACC"
  cp -f "$ACC" /work/target-linux-arm64/release/ggcc
  OUT=/work/scratch/builtin_m5_prog
  rm -f "$OUT" "$OUT.s" "$OUT.o"

  set +e
  strace -f -e trace=execve env \
    ACC_BUILTIN_AS=1 ACC_BUILTIN_LD=1 ACC_BUILTIN_LD_STRICT=1 \
    "$ACC" -m aarch64 --target-os linux -o "$OUT" /work/tests/builtin_m5_hello.c \
    2> /work/scratch/builtin_m5_strace.log
  compile_rc=$?
  set -e

  if grep -E "execve\(\"/usr/bin/(cc|gcc|ld|as)\"" /work/scratch/builtin_m5_strace.log; then
    echo "FAIL: system cc/ld/as spawned"
    exit 1
  fi
  if [ "$compile_rc" -ne 0 ]; then
    echo "FAIL: acc compile exit $compile_rc"
    grep -a ERROR /work/scratch/builtin_m5_strace.log || true
    exit 1
  fi
  if [ ! -x "$OUT" ]; then
    echo "FAIL: missing executable $OUT"
    exit 1
  fi

  run_out="$("$OUT")"
  run_rc=$?
  echo "run_stdout=${run_out}"
  echo "run_exit=$run_rc"
  if [ "$run_rc" -ne 0 ] || [ "$run_out" != "Hello, world!" ]; then
    echo "FAIL: bad run (rc=$run_rc stdout=$run_out)"
    exit 1
  fi
  readelf -h "$OUT" | grep -q "EXEC"
  echo "link=builtin"
'

utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cat > "$STAMP" <<EOF
builtin_linker M5=ok
isa=aarch64-linux
stamp_utc=${utc}
approach=static-musl
docker=${IMAGE}
link=builtin (no system cc/ld/as)
run_log=${LOG#"$ROOT/"}
run_stdout=Hello, world!
run_exit=0
exe=${PROG#"$ROOT/"}
note=Scrt1+crti+crtn + libgcc.a + libc.a via in-tree ld; ACC_BUILTIN_LD_STRICT=1; +x via linker
EOF

echo "PASS: wrote $STAMP"
