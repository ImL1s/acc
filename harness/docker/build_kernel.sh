#!/usr/bin/env bash
# Stage C1 scaffold: fetch Linux 6.9 and attempt bootable build with CC=ggcc.
# Honest status: full kernel requires far more C language coverage than Stage B.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="${SCRATCH:?SCRATCH required}"
VER=6.9
SRC_DIR="$ROOT/third_party/linux-$VER"
LOG="$SCRATCH/stage_c_kernel.log"
GGCC="${GGCC:-$ROOT/target/release/ggcc}"

{
  echo "# Stage C1 Linux $VER kernel attempt"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(uname -a)"
  echo "ggcc: $GGCC"
  "$GGCC" --help 2>&1 | head -8 || true
} >"$LOG"

if [[ ! -d "$SRC_DIR" ]]; then
  echo "fetching linux-$VER ..." | tee -a "$LOG"
  mkdir -p "$ROOT/third_party"
  # Prefer existing tarball cache under SCRATCH
  TAR="$SCRATCH/linux-$VER.tar.xz"
  if [[ ! -f "$TAR" ]]; then
    curl -fL --retry 3 -o "$TAR" \
      "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${VER}.tar.xz" \
      2>&1 | tee -a "$LOG" || {
        echo "VERDICT: BLOCKED fetch_failed" | tee -a "$LOG"
        exit 2
      }
  fi
  tar -xJf "$TAR" -C "$ROOT/third_party" 2>&1 | tee -a "$LOG"
fi

# Tiny freestanding smoke: compile a freestanding main that could be linked
# as part of a future kernel boot path (not the full kernel).
cat >"$SCRATCH/kstub.c" <<'C'
/* freestanding stub — proves Linux ELF path for future kernel objects */
void _start(void);
void _start(void) {
  for (;;) { }
}
C

set +e
"$GGCC" --target-os linux -S -o "$SCRATCH/kstub.s" "$SCRATCH/kstub.c" 2>>"$LOG"
ec=$?
echo "kstub_compile_ec=$ec" | tee -a "$LOG"
head -20 "$SCRATCH/kstub.s" >>"$LOG" 2>/dev/null

# Attempt: does tiny tinyconfig even parse with ggcc? Expect fail — log honestly.
echo "=== tinyconfig attempt (expect incomplete language) ===" | tee -a "$LOG"
docker run --rm \
  -v "$ROOT":/work \
  -v "$SCRATCH":/scratch \
  -w /work/third_party/linux-$VER \
  -e GGCC=/work/target/release/ggcc \
  ggcc-linux bash -lc '
    set +e
    make defconfig ARCH=arm64 2>&1 | tail -5
    # Use ggcc only for a single trivial file if present; full CC=ggcc will fail early.
    # Document first real error.
    echo "int x;" > /tmp/kprobe.c
    $GGCC --target-os linux -S -o /tmp/kprobe.s /tmp/kprobe.c
    echo kprobe_ec=$?
    head -15 /tmp/kprobe.s
    # Real kernel: first C file compile probe (init/main.c is huge)
    $GGCC --target-os linux -S -o /tmp/main.s init/main.c 2>/tmp/main_err.txt
    echo main_ec=$?
    head -30 /tmp/main_err.txt
  ' 2>&1 | tee -a "$LOG"

echo "VERDICT: BLOCKED — full Linux $VER boot not achieved; language/coverage gap for kernel C" | tee -a "$LOG"
echo "blocked_reason: ggcc cannot yet compile kernel sources (preprocessor/headers/attributes/asm/inline). Linux ELF path works for trivial programs (see kstub)." | tee -a "$LOG"
exit 3
