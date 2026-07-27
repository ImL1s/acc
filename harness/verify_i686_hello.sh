#!/usr/bin/env bash
# Verify i686 hello.s under qemu-i386 (Docker). Host is often arm64 macOS without -m32.
# Run from worktree root: bash scratch/verify_i686_hello.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

cargo test --bin acc codegen::i686::tests::write_hello_oracle_asm -- --exact
test -f scratch/hello_i686.s

IMG="${I686_VERIFY_IMAGE:-ubuntu:22.04}"
docker run --rm --platform linux/amd64 \
  -v "$ROOT/scratch:/work" -w /work \
  "$IMG" bash -c '
    set -e
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -qq -y gcc-multilib qemu-user >/dev/null
    gcc -m32 -no-pie hello_i686.s -o hello_i686
    out=$(qemu-i386 ./hello_i686); ec=$?
    echo "ec=$ec stdout=[$out]"
    test "$ec" = "0"
    test "$out" = "Hello, world!"
    echo PASS_I686_HELLO
  '
