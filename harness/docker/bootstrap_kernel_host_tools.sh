#!/usr/bin/env bash
# Bootstrap kbuild host tools on a fresh or copied tree (wrong-arch binaries removed).
# Usage: bootstrap_kernel_host_tools.sh [KARCH]
set -euo pipefail

KARCH="${1:-x86}"
log() { echo "[bootstrap_host_tools] $*"; }

cd "$(pwd)"

log "KARCH=$KARCH — clearing stale host tool binaries"
rm -f scripts/basic/fixdep \
  scripts/kconfig/conf scripts/kconfig/mconf \
  scripts/mod/mk_elfconfig \
  scripts/selinux/genheaders scripts/selinux/mdp \
  2>/dev/null || true

if [[ -f scripts/basic/fixdep.c ]]; then
  log "building scripts/basic/fixdep"
  gcc -o scripts/basic/fixdep scripts/basic/fixdep.c
fi

log "building scripts/kconfig/conf"
make ARCH="$KARCH" HOSTCC=gcc scripts/kconfig/conf 2>&1 | tail -5 || true

if [[ -f scripts/mod/mk_elfconfig.c ]] && [[ ! -x scripts/mod/mk_elfconfig ]]; then
  log "building scripts/mod/mk_elfconfig"
  gcc -o scripts/mod/mk_elfconfig scripts/mod/mk_elfconfig.c || \
    make ARCH="$KARCH" HOSTCC=gcc scripts/mod/mk_elfconfig 2>&1 | tail -5 || true
fi

log "done (fixdep=$(test -x scripts/basic/fixdep && echo ok || echo missing) conf=$(test -x scripts/kconfig/conf && echo ok || echo missing) mk_elfconfig=$(test -x scripts/mod/mk_elfconfig && echo ok || echo missing))"
