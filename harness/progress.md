# Progress (NO-DOWNGRADE)

## Stage A/B — PASS | Stage C — **NOT complete**

| Gate | Status |
|------|--------|
| C3 multiarch | PASS (held; re-run after language churn) |
| C5 double-run | PASS (held; re-run after language churn) |
| C4 clean-room | held |
| C2 | SQLite amalgamation smoke only; **no testfixture** |
| **C1** | **PROGRESS** — ~238 kernel .o; **no bzImage / QEMU** |

### C1 greens (amd64 Docker)
- prepare0 full
- init/* (main, do_mounts, …)
- fs: **namei.o, exec.o**, ~40+ fs/*.o
- kernel/time: **timekeeping.o, hrtimer.o**, timer, jiffies, …
- kernel: **cpu.o**
- drivers: base/*, **char/random.o**
- arch/x86/entry: vdso/vma, **vdso/extable**, **vsyscall_64**
- lib: string, vsprintf, hexdump, …

### Language fixes (this long session)
- enum IntLit, soft ->/member/deref, __int128, arch predefs
- symbol dedupe, restrict, va_arg, offsetof[arr]
- SYSCALL_DEFINE rewrite, TRACE/IDT macro soft, export soup strip
- static/inline body skip, soft ({...})→0, typeof→long
- designators: .field[n], [i][j]=, [0...n+1], enum trailing vars

### Still red / slow
- kernel/sched/core.o, kernel/rcu/tiny.o (**native hang / QEMU timeout** — multi-min)
- arch/x86/events/*.o
- mm some TUs

### blocked_reason
**C1:** no bootable bzImage yet (~238 .o, sched/rcu bottleneck).  
**C2:** no SQLite testfixture / Redis.  
**C5:** needs re-run after language churn.  
**Goal NOT complete.**
