#!/usr/bin/env bash
# End-to-end: emit asm via smoke bin, assemble+run under qemu-riscv64 (one Docker).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
OUT="$ROOT/harness/riscv64_smoke/out"
mkdir -p "$OUT"

cargo test --manifest-path harness/riscv64_smoke/Cargo.toml --quiet
for p in return_code arith multi_fn hello; do
  cargo run --manifest-path harness/riscv64_smoke/Cargo.toml --quiet -- "$p" "$OUT/$p.s"
done

IMG="${GGCC_RISCV_DOCKER_IMAGE:-ubuntu:24.04}"
docker run --rm --platform linux/amd64 \
  -v "$OUT:/work" \
  -w /work \
  "$IMG" \
  bash -lc '
    set -euo pipefail
    if ! command -v riscv64-linux-gnu-gcc >/dev/null 2>&1; then
      apt-get update -qq
      DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
        gcc-riscv64-linux-gnu qemu-user-static >/dev/null
    fi
    fail=0
    run() {
      local name="$1" expect="$2"
      riscv64-linux-gnu-gcc -static -o "/work/${name}.riscv64" "/work/${name}.s"
      set +e
      out=$(qemu-riscv64-static "/work/${name}.riscv64" 2>&1)
      ret=$?
      set -e
      echo "[$name] ret=$ret stdout=[$out] expect_ret=$expect"
      if [[ "$ret" -ne "$expect" ]]; then echo "FAIL ret"; fail=1; fi
    }
    run return_code 7
    run arith 42
    run multi_fn 0
    riscv64-linux-gnu-gcc -static -o /work/hello.riscv64 /work/hello.s
    set +e
    out=$(qemu-riscv64-static /work/hello.riscv64 2>&1)
    ret=$?
    set -e
    echo "[hello] ret=$ret stdout=[$out]"
    [[ "$ret" -eq 0 ]] || fail=1
    [[ "$out" == "Hello, world!" ]] || { echo "FAIL stdout"; fail=1; }
    if [[ "$fail" -ne 0 ]]; then exit 1; fi
    echo ALL_PASS
  '

echo "ALL RISCV64 SMOKES PASSED"
