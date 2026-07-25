# GGCC_SKIP_VDSO — freestanding C1 arm64: skip real vDSO image build.
# ggcc-built vgettimeofday.o puts _vdso_data in discarded .bss and breaks the link.
# prepare still needs include/generated/vdso-offsets.h — emit a minimal stub.

PHONY += include/generated/vdso-offsets.h

include/generated/vdso-offsets.h:
	$(Q)mkdir -p include/generated
	$(Q)printf '%s\n' \
	  '/* ggcc C1: minimal vdso-offsets (real vdso skipped) */' \
	  '#define vdso_offset_sigtramp 0' \
	  > $@

# Quiet no-op targets so vdso_prepare / vdso-install do not fail.
obj-y :=
obj-m :=
targets :=
