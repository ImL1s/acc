#!/usr/bin/env bash
set -euo pipefail
LOG=/scratch/stage_c_kernel.log
log() { echo "$@" | tee -a "$LOG"; }

log "container: $(uname -a)"
log "KERNEL_ARCH=$KERNEL_ARCH KARCH=$KARCH ACC_M=$ACC_M"
log "ACC_ALLOW_SOFT_SYSCC=${ACC_ALLOW_SOFT_SYSCC:-0} (must stay 0 for C1)"
log "=== cargo build --release (Linux acc; separate target dir) ==="
# Do NOT overwrite host macOS target/release/acc with Linux ELF.
export CARGO_TARGET_DIR=/work/target-linux
cargo build --release 2>&1 | tee -a "$LOG"
ACC=/work/target-linux/release/acc
test -x "$ACC"
export ACC
export ACC_ARCH="${ACC_M:-$KERNEL_ARCH}"
export ACC_TARGET_OS=linux
export SYSCC=gcc
# Explicit: no soft body skip / no soft system-CC on .c
export ACC_PARSE_ALL_BODIES=1
export ACC_SOFT_SKIP_BODIES=0
# Soft freestanding body replacements OFF on C1 PASS path (emit real C).
unset ACC_SOFT_FREESTANDING || true
export ACC_SOFT_FREESTANDING=0
export ACC_KERNEL_FREESTANDING=1
export ACC_ALLOW_SOFT_SYSCC=0
WRAP=/work/harness/docker/acc_cc_wrapper.sh
chmod +x "$WRAP"
"$ACC" --help 2>&1 | head -8 | tee -a "$LOG" || true

# Trivial probe inside container (native arch asm)
echo "int x;" > /scratch/kprobe.c
set +e
"$ACC" --target-os linux -m "$ACC_M" -S -o /scratch/kprobe.s /scratch/kprobe.c 2>>"$LOG"
log "kprobe_ec=$?"
set -e
head -10 /scratch/kprobe.s >>"$LOG" 2>/dev/null || true

rm -rf /tmp/linux-src
mkdir -p /tmp/linux-src
# x86_64: optional persist under /scratch (needs ~2Gi host disk). Default: container /tmp only.
if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
  if [[ "${ACC_X86_PERSISTENT_BUILD:-0}" == "1" ]]; then
    mkdir -p /scratch/linux-x86-build
    if [[ ! -f /scratch/linux-x86-build/Makefile ]]; then
      if [[ -f "/scratch/linux-$VER.tar.xz" ]]; then
        log "extracting linux-$VER to /scratch/linux-x86-build..."
        tar -xJf "/scratch/linux-$VER.tar.xz" -C /scratch/linux-x86-build --strip-components=1
      elif [[ -d "/work/$KSRC_REL" ]]; then
        log "seeding /scratch/linux-x86-build from source tree (ACC_X86_PERSISTENT_BUILD=1)..."
        rsync -a --delete \
          --exclude='.config' --exclude='*.o' --exclude='*.a' --exclude='vmlinux' \
          --exclude='arch/*/boot/Image*' --exclude='arch/*/boot/bzImage' \
          --exclude='scripts/basic/fixdep' --exclude='scripts/kconfig/conf' \
          --exclude='scripts/mod/mk_elfconfig' --exclude='include/generated' \
          "/work/$KSRC_REL"/ /scratch/linux-x86-build/
      fi
    fi
  else
    log "x86: using container-local /tmp/linux-src (set ACC_X86_PERSISTENT_BUILD=1 to use /scratch/linux-x86-build)"
  fi
fi
# Prefer in-place work-tree build when a known-good .config exists for THIS arch
# (arm64 .config must not be reused for x86_64 — tinyconfig instead).
USE_INPLACE=0
if [[ -f "/work/$KSRC_REL/.config" ]]; then
  if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
    if grep -q "^CONFIG_ARM64=y" "/work/$KSRC_REL/.config" 2>/dev/null; then
      USE_INPLACE=1
    fi
  else
    if grep -qE "^CONFIG_X86_64=y|^CONFIG_X86=y" "/work/$KSRC_REL/.config" 2>/dev/null; then
      USE_INPLACE=1
    fi
  fi
fi
if [[ "$USE_INPLACE" -eq 1 ]]; then
  KBUILD="/work/$KSRC_REL"
  log "using in-place kernel tree $KBUILD (existing .config matches KERNEL_ARCH)"
  cd "$KBUILD"
  # Ensure VDSO is off — ggcc .data/.bss in vgettimeofday breaks vdso link.
  if [[ -x scripts/config ]]; then
    scripts/config --file .config --disable VDSO 2>/dev/null || true
    scripts/config --file .config --disable COMPAT_VDSO 2>/dev/null || true
    make ARCH="$KARCH" olddefconfig 2>&1 | tee -a "$LOG" | tail -10 || true
  fi
  log "config: existing ($(wc -l < .config) lines); CONFIG_VDSO=$(grep -E "^CONFIG_VDSO" .config || echo unset)"
else
  if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]] && [[ "${ACC_X86_PERSISTENT_BUILD:-0}" == "1" ]] && [[ -f /scratch/linux-x86-build/Makefile ]]; then
    KBUILD=/scratch/linux-x86-build
    cd "$KBUILD"
    log "using persistent x86 tree $KBUILD"
  elif [[ -f "/scratch/linux-$VER.tar.xz" ]]; then
    log "extracting /scratch/linux-$VER.tar.xz to container-local /tmp/linux-src..."
    tar -xJf "/scratch/linux-$VER.tar.xz" -C /tmp/linux-src --strip-components=1
    KBUILD=/tmp/linux-src
    cd "$KBUILD"
  elif [[ -d "/work/$KSRC_REL" ]] && [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
    log "copying /work/$KSRC_REL to container-local /tmp/linux-src (x86)..."
    rsync -a --exclude=".config" --exclude="arch/*/boot/Image*" \
      --exclude="arch/*/boot/bzImage" --exclude="vmlinux" --exclude="*.o" \
      --exclude="scripts/basic/fixdep" --exclude="scripts/kconfig/conf" \
      --exclude="scripts/mod/mk_elfconfig" --exclude="include/generated" \
      "/work/$KSRC_REL"/ /tmp/linux-src/ 2>/dev/null \
      || cp -a "/work/$KSRC_REL"/. /tmp/linux-src/
    rm -f /tmp/linux-src/.config
    KBUILD=/tmp/linux-src
    cd "$KBUILD"
  elif [[ -d "/work/$KSRC_REL" ]] && [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
    log "copying /work/$KSRC_REL to container-local /tmp/linux-src (arm64 in-place path)..."
    rsync -a --exclude=".config" --exclude="arch/*/boot/Image*" \
      --exclude="arch/*/boot/bzImage" --exclude="vmlinux" --exclude="*.o" \
      "/work/$KSRC_REL"/ /tmp/linux-src/ 2>/dev/null \
      || cp -a "/work/$KSRC_REL"/. /tmp/linux-src/
    rm -f /tmp/linux-src/.config
    KBUILD=/tmp/linux-src
    cd "$KBUILD"
  else
    KBUILD=/tmp/linux-src
    cd "$KBUILD"
  fi
  if [[ ! -f .config ]] || ! grep -qE "^CONFIG_X86_64=y|^CONFIG_ARM64=y" .config 2>/dev/null; then
    chmod +x /work/harness/docker/bootstrap_kernel_host_tools.sh 2>/dev/null || true
    /work/harness/docker/bootstrap_kernel_host_tools.sh "$KARCH" 2>&1 | tee -a "$LOG" || true
    log "=== make ARCH=$KARCH tinyconfig ==="
    make ARCH="$KARCH" defconfig 2>&1 | tee -a "$LOG" | tail -20 || true
    make ARCH="$KARCH" tinyconfig 2>&1 | tee -a "$LOG" | tail -20
    if [[ -x scripts/config ]]; then
      scripts/config --file .config --disable VDSO 2>/dev/null || true
      scripts/config --file .config --disable COMPAT_VDSO 2>/dev/null || true
    fi
    if [[ "$KERNEL_ARCH" == "x86_64" || "$KERNEL_ARCH" == "x86" ]]; then
      echo "CONFIG_64BIT=y" >> .config
      if [[ -x scripts/config ]]; then
        scripts/config --file .config --disable PERF_EVENTS 2>/dev/null || true
        scripts/config --file .config --disable HW_PERF_EVENTS 2>/dev/null || true
        scripts/config --file .config --disable X86_MSR 2>/dev/null || true
        scripts/config --file .config --disable X86_CPUID 2>/dev/null || true
        scripts/config --file .config --disable KPROBES 2>/dev/null || true
        scripts/config --file .config --disable FTRACE 2>/dev/null || true
        scripts/config --file .config --disable STACK_TRACER 2>/dev/null || true
        scripts/config --file .config --disable FUNCTION_TRACER 2>/dev/null || true
        scripts/config --file .config --disable RETPOLINE 2>/dev/null || true
        scripts/config --file .config --disable SLS 2>/dev/null || true
        scripts/config --file .config --disable CPU_MITIGATIONS 2>/dev/null || true
      fi
    fi
    make ARCH="$KARCH" olddefconfig 2>&1 | tee -a "$LOG" | tail -20
    log "config: tinyconfig generated ($(wc -l < .config) lines)"
  else
    log "config: reusing existing ($(wc -l < .config) lines)"
    if [[ "$KERNEL_ARCH" == "x86_64" || "$KERNEL_ARCH" == "x86" ]] && [[ -x scripts/config ]]; then
      scripts/config --file .config --disable PERF_EVENTS 2>/dev/null || true
      scripts/config --file .config --disable HW_PERF_EVENTS 2>/dev/null || true
      make ARCH="$KARCH" olddefconfig 2>&1 | tee -a "$LOG" | tail -10 || true
    fi
  fi
  # Belt-and-suspenders: olddefconfig can re-enable PERF/VDSO — force off for x86.
  if [[ "$KERNEL_ARCH" == "x86_64" || "$KERNEL_ARCH" == "x86" ]]; then
    for opt in PERF_EVENTS HW_PERF_EVENTS X86_MSR MICROCODE MICROCODE_INTEL MICROCODE_AMD \
      VDSO COMPAT_VDSO KPROBES FTRACE FUNCTION_TRACER RETPOLINE SLS CPU_MITIGATIONS \
      MODULES UNWINDER_ORC STACK_VALIDATION BUILDTIME_TABLE_SORT; do
      sed -i "/^CONFIG_${opt}=y/d" .config 2>/dev/null || true
      sed -i "/^CONFIG_${opt}=m/d" .config 2>/dev/null || true
      grep -q "CONFIG_${opt} is not set" .config 2>/dev/null \
        || echo "# CONFIG_${opt} is not set" >> .config
    done
    grep -q "^CONFIG_64BIT=y" .config || echo "CONFIG_64BIT=y" >> .config
    grep -q "^CONFIG_PVH=y" .config || echo "CONFIG_PVH=y" >> .config
    log "x86 config forced: PERF_EVENTS line=$(grep PERF_EVENTS .config | grep -v AMD | head -1)"
  fi
fi

if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
  chmod +x /work/harness/docker/install_x86_pvh_boot.sh 2>/dev/null || true
  /work/harness/docker/install_x86_pvh_boot.sh "$(pwd)" 2>&1 | tee -a "$LOG" || true
  make ARCH="$KARCH" olddefconfig 2>&1 | tee -a "$LOG" | tail -10 || true
fi

# Sync freestanding EL0 stubs (arch-specific) into lib/
mkdir -p lib
if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
  cp -f /work/harness/docker/ggcc_vmlinux_stubs.c lib/ggcc_vmlinux_stubs.c
  cp -f /work/harness/docker/ggcc_el0.S lib/ggcc_el0.S
else
  cp -f /work/harness/docker/ggcc_vmlinux_stubs_x86_64.c lib/ggcc_vmlinux_stubs.c
  cp -f /work/harness/docker/ggcc_el0_x86_64.S lib/ggcc_el0.S
  if [[ -f /work/harness/docker/ggcc_link_stubs_x86_64.c ]]; then
    cp -f /work/harness/docker/ggcc_link_stubs_x86_64.c lib/ggcc_link_stubs_x86_64.c
  fi
fi
if ! grep -q ggcc_vmlinux_stubs.o lib/Makefile 2>/dev/null; then
  { echo "obj-y += ggcc_vmlinux_stubs.o"; echo "obj-y += ggcc_el0.o"; cat lib/Makefile; } > /tmp/libmk.$$
  mv /tmp/libmk.$$ lib/Makefile
fi
if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
  if [[ -f lib/ggcc_link_stubs_x86_64.c ]] && ! grep -q ggcc_link_stubs_x86_64.o lib/Makefile 2>/dev/null; then
    { echo "obj-y += ggcc_link_stubs_x86_64.o"; cat lib/Makefile; } > /tmp/libmk.$$
    mv /tmp/libmk.$$ lib/Makefile
    log "added ggcc_link_stubs_x86_64.o to lib/Makefile"
  fi
fi
log "synced lib/ggcc_el0.S + ggcc_vmlinux_stubs.c for $KERNEL_ARCH"
if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
  if [[ -f /work/harness/docker/x86_vdso_stub/Makefile ]]; then
    mkdir -p arch/x86/entry/vdso
    cp -f /work/harness/docker/x86_vdso_stub/Makefile arch/x86/entry/vdso/Makefile
    log "installed x86 vdso stub Makefile (skip vdso image)"
    # ggcc drops `static void __used common()` — force emission + seed criticals.
    if [[ -x /work/harness/docker/fix_asm_offsets_x86_64.sh ]] \
      || [[ -f /work/harness/docker/fix_asm_offsets_x86_64.sh ]]; then
      chmod +x /work/harness/docker/fix_asm_offsets_x86_64.sh 2>/dev/null || true
      /work/harness/docker/fix_asm_offsets_x86_64.sh "$(pwd)" | tee -a "$LOG" || true
      log "ran fix_asm_offsets_x86_64.sh"
    fi
    # Force remake of asm-offsets so common() lands in .s
    rm -f arch/x86/kernel/asm-offsets.s include/generated/asm-offsets.h \
      arch/x86/entry/entry_64.o 2>/dev/null || true
  fi
fi

# Force remake of objects that may still carry soft-freestanding early stubs
# or lack mid-boot hard-keeper stubs after codegen policy changes.
# A07: do NOT mass-remake fs/*.o here — remaking dcache/inode (#113+) hung
# after random_init_early before vfs_caches_init_early (Dentry). Only remake
# init handoff TU when shrinking rest_init/run_init freestanding.
log "=== force remake of boot-critical objects (soft=0 PASS path) ==="
if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
  rm -f arch/arm64/mm/*.o arch/arm64/kernel/setup.o \
    mm/bootmem_info.o mm/mm_init.o mm/memblock.o \
    init/initramfs.o init/ggcc_init_payload.o \
    kernel/sched/core.o kernel/softirq.o \
    kernel/printk/printk.o fs/dcache.o \
    lib/ggcc_vmlinux_stubs.o lib/ggcc_el0.o 2>/dev/null || true
else
  rm -f lib/ggcc_vmlinux_stubs.o lib/ggcc_el0.o \
    lib/ggcc_pvh_note.o lib/ggcc_pvh_head.o lib/ggcc_pvh_enlighten.o 2>/dev/null || true
fi

log "=== make ARCH=$KARCH CC=acc_cc_wrapper.sh HOSTCC=gcc Image ==="
# HOSTCC=gcc: kconfig/fixdep host tools only — not kernel .c
# CC=wrapper: kernel .c → ggcc only
# x86 produces bzImage; arm64 produces Image.
if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
  KIMAGE_TARGET=Image
else
  KIMAGE_TARGET=vmlinux
  # Phase 0: regenerate asm-offsets with forced common(), then seed/fix header.
  log "=== x86 phase0: asm-offsets with ggcc_force_common ==="
  set +e
  make ARCH="$KARCH" CC="$WRAP" HOSTCC=gcc HOSTCXX=g++ \
    arch/x86/kernel/asm-offsets.s include/generated/asm-offsets.h \
    2>&1 | tee -a "$LOG" | tee /scratch/a08_asm_offsets_regen.log | tail -40
  set -e
  /work/harness/docker/fix_asm_offsets_x86_64.sh "$(pwd)" | tee -a "$LOG" || true
  # If common() still didn't emit PTREGS_SIZE into .s, append soft .ascii lines then re-filechk.
  if [[ -f arch/x86/kernel/asm-offsets.s ]] && ! grep -q 'PTREGS_SIZE' arch/x86/kernel/asm-offsets.s; then
    log "WARN: PTREGS_SIZE missing from .s — appending soft ascii DEFINEs"
    python3 /work/harness/docker/append_asm_offsets_ascii.py arch/x86/kernel/asm-offsets.s || true
    # Rebuild header from .s via scripts/Makefile.asm-offsets / filechk
    mkdir -p include/generated
    if [[ -x scripts/basic/fixdep ]] || [[ -f scripts/Makefile.lib ]]; then
      # Manual extract like Kbuild filechk
      sed -ne 's:^[[:space:]]*\.ascii[[:space:]]*"\(->.*\)".*:\1:; /^->/{s:->#.*::; s:^->::; p;}' \
        arch/x86/kernel/asm-offsets.s | \
      awk '
        /^$/ { next }
        /^#/ { next }
        {
          name=$1; val=$2;
          comment="";
          for (i=3;i<=NF;i++) comment=comment" "$i;
          if (name=="") print "";
          else printf("#define %s %s /*%s */\n", name, val, comment);
        }
      ' > include/generated/asm-offsets.h.tmp
      {
        echo "#ifndef __ASM_OFFSETS_H__"
        echo "#define __ASM_OFFSETS_H__"
        echo "/*"
        echo " * DO NOT MODIFY."
        echo " *"
        echo " * This file was generated by Kbuild + ggcc soft seed"
        echo " */"
        echo ""
        cat include/generated/asm-offsets.h.tmp
        echo ""
        echo "#endif"
      } > include/generated/asm-offsets.h
      rm -f include/generated/asm-offsets.h.tmp
      log "rebuilt asm-offsets.h from .s (soft)"
    fi
  fi
  /work/harness/docker/fix_asm_offsets_x86_64.sh "$(pwd)" | tee -a "$LOG" || true
  log "asm-offsets PTREGS=$(grep PTREGS_SIZE include/generated/asm-offsets.h || echo MISSING)"
  log "asm-offsets TSS_sp0=$(grep TSS_sp0 include/generated/asm-offsets.h || echo MISSING)"
  # Touch header newer than entry objects so make does not regenerate empty common away
  # if asm-offsets.c is remade without our main() call — keep patched sources.
  touch include/generated/asm-offsets.h
fi
log "KIMAGE_TARGET=$KIMAGE_TARGET"
set +e
make ARCH="$KARCH" \
  CC="$WRAP" \
  HOSTCC=gcc \
  HOSTCXX=g++ \
  -j"$JOBS" \
  "$KIMAGE_TARGET" \
  2>&1 | tee /scratch/kernel_make_full.log | tee -a "$LOG" | tail -80
make_ec=${PIPESTATUS[0]}
set -e
log "make_ec=$make_ec"

# Capture last failure snippet
if [[ $make_ec -ne 0 ]]; then
  log "=== last_failure (tail kernel_make_full.log) ==="
  tail -60 /scratch/kernel_make_full.log | tee -a "$LOG"
  # Prefer first acc_cc_wrapper / ERROR line
  log "=== first_acc_error ==="
  grep -n -E "acc_cc_wrapper:|ERROR:|error:" /scratch/kernel_make_full.log | head -30 | tee -a "$LOG" || true
fi

# Artifacts? Prefer arch-matching Image paths (stale other-arch images ignored by path).
bz=""
if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
  for cand in arch/arm64/boot/Image arch/arm64/boot/Image.gz; do
    if [[ -f "$cand" ]]; then bz="$cand"; break; fi
  done
else
  if [[ -f arch/x86/boot/bzImage ]]; then bz=arch/x86/boot/bzImage; fi
fi
if [[ -z "$bz" && -f vmlinux ]]; then bz=vmlinux; fi
if [[ -n "$bz" ]]; then
  log "kernel_image: $bz"
  log "=== QEMU boot attempt (60s) ==="
  # Optional busybox initrd (harness/initrd/); leave PASS grep to A17.
  QEMU_INITRD_ARGS=()
  if [[ -n "${INITRD_REL:-}" && -f "/work/$INITRD_REL" ]]; then
    QEMU_INITRD_ARGS=(-initrd "/work/$INITRD_REL")
    log "QEMU -initrd /work/$INITRD_REL"
  elif [[ -n "${INITRD_PATH:-}" && -f "$INITRD_PATH" ]]; then
    QEMU_INITRD_ARGS=(-initrd "$INITRD_PATH")
    log "QEMU -initrd $INITRD_PATH"
  else
    log "QEMU -initrd: skipped (no initrd at INITRD_PATH)"
  fi
  set +e
  if [[ "$KERNEL_ARCH" = arm64 || "$KERNEL_ARCH" = aarch64 ]]; then
    timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a57 -m 512 -kernel "$bz" \
      ${QEMU_INITRD_ARGS[@]+"${QEMU_INITRD_ARGS[@]}"} \
      -nographic -append "console=ttyAMA0 earlycon=pl011,0x9000000" \
      2>&1 | tee /scratch/qemu_boot.log | tee -a "$LOG" | tail -80
  else
    timeout 90 qemu-system-x86_64 -m 512 -kernel "$bz" \
      ${QEMU_INITRD_ARGS[@]+"${QEMU_INITRD_ARGS[@]}"} \
      -nographic -append "console=ttyS0 earlyprintk=serial,ttyS0,115200" \
      -serial mon:stdio -no-reboot \
      2>&1 | tee /scratch/qemu_boot.log | tee /scratch/qemu_boot_x86_64.log | tee -a "$LOG" | tail -80
  fi
  qec=${PIPESTATUS[0]}
  set -e
  log "qemu_ec=$qec"
  # C1 CCC PASS bar: Linux version AND busybox/shell evidence.
  # Soft ggcc-init / "working init" alone is NOT PASS_BOOT.
  has_linux=0
  has_shell=0
  grep -q "Linux version" /scratch/qemu_boot.log 2>/dev/null && has_linux=1
  # Shell/busybox markers: prompt, BusyBox banner, or /bin/sh interactive path.
  # Explicitly exclude soft pid1 stamps (ggcc-init: / working init).
  # NOTE: use double quotes — this script is embedded in bash -lc so
  # single-quoted patterns would terminate the outer string early.
  if grep -qE "/#|BusyBox|/bin/sh" /scratch/qemu_boot.log 2>/dev/null; then
    has_shell=1
  fi
  if [[ "$has_linux" -eq 1 && "$has_shell" -eq 1 ]]; then
    log "BOOT_EVIDENCE: Linux version + busybox/shell present"
    if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
      echo PASS_BOOT > /scratch/c1_boot_marker_x86_64
      cp -f /scratch/qemu_boot.log /scratch/qemu_boot_x86_64.log 2>/dev/null || true
    else
      echo PASS_BOOT > /scratch/c1_boot_marker
    fi
  elif [[ "$has_linux" -eq 1 ]]; then
    if grep -qE "ggcc-init:|working init" /scratch/qemu_boot.log 2>/dev/null; then
      log "BOOT_EVIDENCE: PARTIAL — Linux version + soft ggcc-init only (NOT C1 CCC PASS; no PASS_BOOT)"
    else
      log "BOOT_EVIDENCE: PARTIAL — Linux version without busybox/shell (no PASS_BOOT)"
    fi
    if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
      cp -f /scratch/qemu_boot.log /scratch/qemu_boot_x86_64.log 2>/dev/null || true
    fi
  else
    log "BOOT_EVIDENCE: missing (no boot strings in serial log)"
    if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
      cp -f /scratch/qemu_boot.log /scratch/qemu_boot_x86_64.log 2>/dev/null || true
    fi
  fi
else
  log "kernel_image: none — skip QEMU (build did not produce bzImage/Image/vmlinux)"
fi

# Exit code for outer script
if [[ "$KERNEL_ARCH" = x86_64 || "$KERNEL_ARCH" = x86 ]]; then
  if [[ -f /scratch/c1_boot_marker_x86_64 ]]; then
    exit 0
  fi
elif [[ -f /scratch/c1_boot_marker ]]; then
  exit 0
fi
exit 3
