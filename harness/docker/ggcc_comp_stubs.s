.text
.code64
/* Soft weak stubs for compressed boot path (ggcc soft compile gaps) */
.macro GStub name
.weak \name
.globl \name
\name:
xorl %eax, %eax
retq
.endm
GStub accept_memory
GStub boot_stage1_vc
GStub boot_stage2_vc
GStub do_vc_no_ghcb
GStub do_boot_stage2_vc
GStub __builtin_memcpy
GStub __builtin_memset
GStub __builtin_memmove
