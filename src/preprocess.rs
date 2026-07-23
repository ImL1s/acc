//! Minimal C preprocessor: #define / #undef / #if / #ifdef / include skip /
//! function-like macros + __VA_ARGS__. Enough for c-testsuite early macro tests.

use std::collections::HashMap;

#[derive(Clone, Debug)]
enum MacroBody {
    Object(String),
    Function { params: Vec<String>, body: String, variadic: bool },
}

pub fn preprocess(src: &str) -> Result<String, String> {
    preprocess_with_dir(src, None)
}

pub fn preprocess_with_dir(src: &str, include_dir: Option<&std::path::Path>) -> Result<String, String> {
    preprocess_with_options(src, include_dir, &[], /*for_linux*/ false, "<input>")
}

/// `for_linux`: omit Darwin-only predefined macros so headers skip Apple blocks.
/// `source_name`: path/name used for `__FILE__` expansion at the primary translation unit.
/// `extra_includes`: additional `-I` directories searched for `#include "..."/`#include <...>`.
/// `arch`: `"x86_64"` | `"aarch64"` — selects arch predefined macros for kernel headers.
pub fn preprocess_with_options(
    src: &str,
    include_dir: Option<&std::path::Path>,
    extra_includes: &[&std::path::Path],
    for_linux: bool,
    source_name: &str,
) -> Result<String, String> {
    preprocess_with_options_arch(src, include_dir, extra_includes, for_linux, source_name, "aarch64")
}

pub fn preprocess_with_options_arch(
    src: &str,
    include_dir: Option<&std::path::Path>,
    extra_includes: &[&std::path::Path],
    for_linux: bool,
    source_name: &str,
    arch: &str,
) -> Result<String, String> {
    let mut macros: HashMap<String, MacroBody> = HashMap::new();
    macros.insert("NULL".into(), MacroBody::Object("0".into()));
    // Do NOT object-macro `errno` to `0`: kernel headers use parameter names
    // like `int force_sig_ptrace_errno_trap(int errno, ...)` which would become
    // `int 0` and break asm-offsets parse. Soft prefix provides `extern int errno`.
    // Match SQLite default: NDEBUG on unless SQLITE_DEBUG. Without system
    // assert.h, leave `assert` as a no-op macro so expressions inside are
    // not evaluated (avoids SQLITE_DEBUG-only symbols like mutexIsInit).
    macros.insert("NDEBUG".into(), MacroBody::Object("1".into()));
    macros.insert(
        "assert".into(),
        MacroBody::Function {
            params: vec!["x".into()],
            body: "((void)0)".into(),
            variadic: false,
        },
    );
    // Common libc macros when system headers are stubbed
    macros.insert("FILENAME_MAX".into(), MacroBody::Object("1024".into()));
    macros.insert("PATH_MAX".into(), MacroBody::Object("4096".into()));
    macros.insert("EOF".into(), MacroBody::Object("(-1)".into()));
    macros.insert("SEEK_SET".into(), MacroBody::Object("0".into()));
    macros.insert("SEEK_CUR".into(), MacroBody::Object("1".into()));
    macros.insert("SEEK_END".into(), MacroBody::Object("2".into()));
    macros.insert("O_RDONLY".into(), MacroBody::Object("0".into()));
    macros.insert("O_WRONLY".into(), MacroBody::Object("1".into()));
    macros.insert("O_RDWR".into(), MacroBody::Object("2".into()));
    macros.insert("O_CREAT".into(), MacroBody::Object("64".into()));
    macros.insert("O_EXCL".into(), MacroBody::Object("128".into()));
    macros.insert("O_TRUNC".into(), MacroBody::Object("512".into()));
    macros.insert("O_APPEND".into(), MacroBody::Object("1024".into()));
    macros.insert("F_OK".into(), MacroBody::Object("0".into()));
    macros.insert("R_OK".into(), MacroBody::Object("4".into()));
    macros.insert("W_OK".into(), MacroBody::Object("2".into()));
    macros.insert("X_OK".into(), MacroBody::Object("1".into()));
    // sysconf / pathconf names (numeric values approximate Linux)
    macros.insert("_SC_PAGESIZE".into(), MacroBody::Object("30".into()));
    macros.insert("_SC_PAGE_SIZE".into(), MacroBody::Object("30".into()));
    macros.insert("_SC_NPROCESSORS_ONLN".into(), MacroBody::Object("84".into()));
    macros.insert("_SC_CLK_TCK".into(), MacroBody::Object("2".into()));
    macros.insert("_PC_PATH_MAX".into(), MacroBody::Object("4".into()));
    macros.insert("_PC_NAME_MAX".into(), MacroBody::Object("3".into()));
    // mmap / madvise
    macros.insert("PROT_READ".into(), MacroBody::Object("1".into()));
    macros.insert("PROT_WRITE".into(), MacroBody::Object("2".into()));
    macros.insert("PROT_EXEC".into(), MacroBody::Object("4".into()));
    macros.insert("PROT_NONE".into(), MacroBody::Object("0".into()));
    macros.insert("MAP_SHARED".into(), MacroBody::Object("1".into()));
    macros.insert("MAP_PRIVATE".into(), MacroBody::Object("2".into()));
    macros.insert("MAP_FIXED".into(), MacroBody::Object("16".into()));
    macros.insert("MAP_ANON".into(), MacroBody::Object("32".into()));
    macros.insert("MAP_ANONYMOUS".into(), MacroBody::Object("32".into()));
    macros.insert("MAP_FAILED".into(), MacroBody::Object("((void*)-1)".into()));
    // Linux mremap(2) flags — used by SQLite unixRemapfile when mmap is enabled.
    macros.insert("MREMAP_MAYMOVE".into(), MacroBody::Object("1".into()));
    macros.insert("MREMAP_FIXED".into(), MacroBody::Object("2".into()));
    macros.insert("MS_ASYNC".into(), MacroBody::Object("1".into()));
    macros.insert("MS_INVALIDATE".into(), MacroBody::Object("2".into()));
    macros.insert("MS_SYNC".into(), MacroBody::Object("4".into()));
    macros.insert("MADV_NORMAL".into(), MacroBody::Object("0".into()));
    macros.insert("MADV_DONTNEED".into(), MacroBody::Object("4".into()));
    macros.insert("MADV_WILLNEED".into(), MacroBody::Object("3".into()));
    // Linux epoll — Redis ae_epoll.c / ae.c when HAVE_EPOLL
    macros.insert("EPOLLIN".into(), MacroBody::Object("0x001".into()));
    macros.insert("EPOLLPRI".into(), MacroBody::Object("0x002".into()));
    macros.insert("EPOLLOUT".into(), MacroBody::Object("0x004".into()));
    macros.insert("EPOLLERR".into(), MacroBody::Object("0x008".into()));
    macros.insert("EPOLLHUP".into(), MacroBody::Object("0x010".into()));
    macros.insert("EPOLLRDHUP".into(), MacroBody::Object("0x2000".into()));
    macros.insert("EPOLLET".into(), MacroBody::Object("0x80000000".into()));
    macros.insert("EPOLLONESHOT".into(), MacroBody::Object("0x40000000".into()));
    macros.insert("EPOLL_CTL_ADD".into(), MacroBody::Object("1".into()));
    macros.insert("EPOLL_CTL_DEL".into(), MacroBody::Object("2".into()));
    macros.insert("EPOLL_CTL_MOD".into(), MacroBody::Object("3".into()));
    // sockets / TCP (Linux aarch64/x86_64 values)
    macros.insert("AF_UNSPEC".into(), MacroBody::Object("0".into()));
    macros.insert("AF_UNIX".into(), MacroBody::Object("1".into()));
    macros.insert("AF_INET".into(), MacroBody::Object("2".into()));
    macros.insert("AF_INET6".into(), MacroBody::Object("10".into()));
    macros.insert("SOCK_STREAM".into(), MacroBody::Object("1".into()));
    macros.insert("SOCK_DGRAM".into(), MacroBody::Object("2".into()));
    macros.insert("SOCK_NONBLOCK".into(), MacroBody::Object("0x800".into()));
    macros.insert("SOCK_CLOEXEC".into(), MacroBody::Object("0x80000".into()));
    macros.insert("SOL_SOCKET".into(), MacroBody::Object("1".into()));
    macros.insert("SO_DEBUG".into(), MacroBody::Object("1".into()));
    macros.insert("SO_REUSEADDR".into(), MacroBody::Object("2".into()));
    macros.insert("SO_TYPE".into(), MacroBody::Object("3".into()));
    macros.insert("SO_ERROR".into(), MacroBody::Object("4".into()));
    macros.insert("SO_DONTROUTE".into(), MacroBody::Object("5".into()));
    macros.insert("SO_BROADCAST".into(), MacroBody::Object("6".into()));
    macros.insert("SO_SNDBUF".into(), MacroBody::Object("7".into()));
    macros.insert("SO_RCVBUF".into(), MacroBody::Object("8".into()));
    macros.insert("SO_KEEPALIVE".into(), MacroBody::Object("9".into()));
    macros.insert("SO_OOBINLINE".into(), MacroBody::Object("10".into()));
    macros.insert("SO_LINGER".into(), MacroBody::Object("13".into()));
    macros.insert("SO_REUSEPORT".into(), MacroBody::Object("15".into()));
    macros.insert("SO_RCVTIMEO".into(), MacroBody::Object("20".into()));
    macros.insert("SO_SNDTIMEO".into(), MacroBody::Object("21".into()));
    macros.insert("IPPROTO_IP".into(), MacroBody::Object("0".into()));
    macros.insert("IPPROTO_TCP".into(), MacroBody::Object("6".into()));
    macros.insert("IPPROTO_UDP".into(), MacroBody::Object("17".into()));
    macros.insert("IPPROTO_IPV6".into(), MacroBody::Object("41".into()));
    macros.insert("TCP_NODELAY".into(), MacroBody::Object("1".into()));
    macros.insert("TCP_KEEPIDLE".into(), MacroBody::Object("4".into()));
    macros.insert("TCP_KEEPINTVL".into(), MacroBody::Object("5".into()));
    macros.insert("TCP_KEEPCNT".into(), MacroBody::Object("6".into()));
    macros.insert("AI_PASSIVE".into(), MacroBody::Object("0x01".into()));
    macros.insert("AI_CANONNAME".into(), MacroBody::Object("0x02".into()));
    macros.insert("AI_NUMERICHOST".into(), MacroBody::Object("0x04".into()));
    macros.insert("AI_V4MAPPED".into(), MacroBody::Object("0x08".into()));
    macros.insert("AI_ALL".into(), MacroBody::Object("0x10".into()));
    macros.insert("AI_ADDRCONFIG".into(), MacroBody::Object("0x20".into()));
    macros.insert("AI_NUMERICSERV".into(), MacroBody::Object("0x400".into()));
    macros.insert("NI_NUMERICHOST".into(), MacroBody::Object("1".into()));
    macros.insert("NI_NUMERICSERV".into(), MacroBody::Object("2".into()));
    macros.insert("MSG_PEEK".into(), MacroBody::Object("0x02".into()));
    macros.insert("MSG_DONTWAIT".into(), MacroBody::Object("0x40".into()));
    macros.insert("MSG_NOSIGNAL".into(), MacroBody::Object("0x4000".into()));
    macros.insert("SHUT_RD".into(), MacroBody::Object("0".into()));
    macros.insert("SHUT_WR".into(), MacroBody::Object("1".into()));
    macros.insert("SHUT_RDWR".into(), MacroBody::Object("2".into()));
    macros.insert("INADDR_ANY".into(), MacroBody::Object("0".into()));
    macros.insert("INADDR_LOOPBACK".into(), MacroBody::Object("0x7f000001".into()));
    // signal / fcntl extras
    macros.insert("SIG_SETMASK".into(), MacroBody::Object("2".into()));
    macros.insert("SIG_BLOCK".into(), MacroBody::Object("0".into()));
    macros.insert("SIG_UNBLOCK".into(), MacroBody::Object("1".into()));
    macros.insert("FD_CLOEXEC".into(), MacroBody::Object("1".into()));
    macros.insert("F_GETFD".into(), MacroBody::Object("1".into()));
    macros.insert("F_SETFD".into(), MacroBody::Object("2".into()));
    macros.insert("F_GETFL".into(), MacroBody::Object("3".into()));
    macros.insert("F_SETFL".into(), MacroBody::Object("4".into()));
    macros.insert("F_FULLFSYNC".into(), MacroBody::Object("51".into()));
    macros.insert("F_BARRIERFSYNC".into(), MacroBody::Object("85".into()));
    macros.insert("O_NONBLOCK".into(), MacroBody::Object("0x800".into()));
    macros.insert("AT_FDCWD".into(), MacroBody::Object("(-100)".into()));
    macros.insert("AT_SYMLINK_NOFOLLOW".into(), MacroBody::Object("256".into()));
    // POSIX SHM / open flags already partially set
    macros.insert("O_NOINHERIT".into(), MacroBody::Object("0".into()));
    macros.insert("O_SHORT_LIVED".into(), MacroBody::Object("0".into()));
    macros.insert("O_TEMPORARY".into(), MacroBody::Object("0".into()));
    macros.insert("O_RANDOM".into(), MacroBody::Object("0".into()));
    macros.insert("O_SEQUENTIAL".into(), MacroBody::Object("0".into()));
    // clock
    macros.insert("CLOCK_REALTIME".into(), MacroBody::Object("0".into()));
    macros.insert("CLOCK_MONOTONIC".into(), MacroBody::Object("1".into()));
    // C99 bool + stdint limits (Redis / SQLite)
    macros.insert("true".into(), MacroBody::Object("1".into()));
    macros.insert("false".into(), MacroBody::Object("0".into()));
    macros.insert("bool".into(), MacroBody::Object("_Bool".into()));
    macros.insert("INT8_MIN".into(), MacroBody::Object("(-128)".into()));
    macros.insert("INT8_MAX".into(), MacroBody::Object("127".into()));
    macros.insert("UINT8_MAX".into(), MacroBody::Object("255".into()));
    macros.insert("UINT16_MAX".into(), MacroBody::Object("65535".into()));
    macros.insert("UINT32_MAX".into(), MacroBody::Object("4294967295U".into()));
    macros.insert("UINT64_MAX".into(), MacroBody::Object("18446744073709551615ULL".into()));
    // LP64: uintptr_t is 64-bit (Redis quicklist QL_FILL_BITS via UINTPTR_MAX)
    macros.insert(
        "UINTPTR_MAX".into(),
        MacroBody::Object("0xffffffffffffffffULL".into()),
    );
    macros.insert(
        "UINTMAX_MAX".into(),
        MacroBody::Object("18446744073709551615ULL".into()),
    );
    macros.insert("INT16_MIN".into(), MacroBody::Object("(-32768)".into()));
    macros.insert("INT16_MAX".into(), MacroBody::Object("32767".into()));
    macros.insert("INT32_MIN".into(), MacroBody::Object("(-2147483647-1)".into()));
    macros.insert("INT32_MAX".into(), MacroBody::Object("2147483647".into()));
    macros.insert("INT64_MIN".into(), MacroBody::Object("(-9223372036854775807LL-1)".into()));
    macros.insert("INT64_MAX".into(), MacroBody::Object("9223372036854775807LL".into()));
    macros.insert("SIZE_MAX".into(), MacroBody::Object("18446744073709551615ULL".into()));
    macros.insert("PTRDIFF_MAX".into(), MacroBody::Object("9223372036854775807LL".into()));
    macros.insert("IOV_MAX".into(), MacroBody::Object("1024".into()));
    // ISO C: UINT64_C(c) / INT64_C(c) → integer constants
    macros.insert(
        "UINT64_C".into(),
        MacroBody::Function {
            params: vec!["c".into()],
            body: "((unsigned long long)(c))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "INT64_C".into(),
        MacroBody::Function {
            params: vec!["c".into()],
            body: "((long long)(c))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "UINT32_C".into(),
        MacroBody::Function {
            params: vec!["c".into()],
            body: "((unsigned long)(c))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "INT32_C".into(),
        MacroBody::Function {
            params: vec!["c".into()],
            body: "((long)(c))".into(),
            variadic: false,
        },
    );
    // offsetof → compiler builtin (parser/codegen already handle __builtin_offsetof)
    macros.insert(
        "offsetof".into(),
        MacroBody::Function {
            params: vec!["t".into(), "m".into()],
            body: "__builtin_offsetof(t, m)".into(),
            variadic: false,
        },
    );
    // math classification (libm provides the real ones; soft as macros when header missing)
    macros.insert(
        "isfinite".into(),
        MacroBody::Function {
            params: vec!["x".into()],
            body: "((x) == (x) && (x) - (x) == 0)".into(),
            variadic: false,
        },
    );
    // wait status (Linux)
    macros.insert(
        "WEXITSTATUS".into(),
        MacroBody::Function {
            params: vec!["s".into()],
            body: "(((s) & 0xff00) >> 8)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "WTERMSIG".into(),
        MacroBody::Function {
            params: vec!["s".into()],
            body: "((s) & 0x7f)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "WIFSIGNALED".into(),
        MacroBody::Function {
            params: vec!["s".into()],
            body: "(((signed char) (((s) & 0x7f) + 1) >> 1) > 0)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "WIFEXITED".into(),
        MacroBody::Function {
            params: vec!["s".into()],
            body: "(((s) & 0x7f) == 0)".into(),
            variadic: false,
        },
    );
    // syslog
    macros.insert("LOG_EMERG".into(), MacroBody::Object("0".into()));
    macros.insert("LOG_ALERT".into(), MacroBody::Object("1".into()));
    macros.insert("LOG_CRIT".into(), MacroBody::Object("2".into()));
    macros.insert("LOG_ERR".into(), MacroBody::Object("3".into()));
    macros.insert("LOG_WARNING".into(), MacroBody::Object("4".into()));
    macros.insert("LOG_NOTICE".into(), MacroBody::Object("5".into()));
    macros.insert("LOG_INFO".into(), MacroBody::Object("6".into()));
    macros.insert("LOG_DEBUG".into(), MacroBody::Object("7".into()));
    macros.insert("LOG_PID".into(), MacroBody::Object("0x01".into()));
    macros.insert("LOG_CONS".into(), MacroBody::Object("0x02".into()));
    macros.insert("LOG_ODELAY".into(), MacroBody::Object("0x04".into()));
    macros.insert("LOG_NDELAY".into(), MacroBody::Object("0x08".into()));
    macros.insert("LOG_NOWAIT".into(), MacroBody::Object("0x10".into()));
    macros.insert("LOG_PERROR".into(), MacroBody::Object("0x20".into()));
    macros.insert("LOG_USER".into(), MacroBody::Object("(1<<3)".into()));
    macros.insert("LOG_LOCAL0".into(), MacroBody::Object("(16<<3)".into()));
    macros.insert("LOG_LOCAL1".into(), MacroBody::Object("(17<<3)".into()));
    macros.insert("LOG_LOCAL2".into(), MacroBody::Object("(18<<3)".into()));
    macros.insert("LOG_LOCAL3".into(), MacroBody::Object("(19<<3)".into()));
    macros.insert("LOG_LOCAL4".into(), MacroBody::Object("(20<<3)".into()));
    macros.insert("LOG_LOCAL5".into(), MacroBody::Object("(21<<3)".into()));
    macros.insert("LOG_LOCAL6".into(), MacroBody::Object("(22<<3)".into()));
    macros.insert("LOG_LOCAL7".into(), MacroBody::Object("(23<<3)".into()));
    // cpu_set macros — soft no-ops when <sched.h> missing (Redis affinity)
    macros.insert(
        "CPU_ZERO".into(),
        MacroBody::Function {
            params: vec!["s".into()],
            body: "do { (void)(s); } while (0)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "CPU_SET".into(),
        MacroBody::Function {
            params: vec!["cpu".into(), "s".into()],
            body: "do { (void)(cpu); (void)(s); } while (0)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "CPU_CLR".into(),
        MacroBody::Function {
            params: vec!["cpu".into(), "s".into()],
            body: "do { (void)(cpu); (void)(s); } while (0)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "CPU_ISSET".into(),
        MacroBody::Function {
            params: vec!["cpu".into(), "s".into()],
            body: "((void)(cpu), (void)(s), 0)".into(),
            variadic: false,
        },
    );
    macros.insert("ITIMER_REAL".into(), MacroBody::Object("0".into()));
    macros.insert("ITIMER_VIRTUAL".into(), MacroBody::Object("1".into()));
    macros.insert("ITIMER_PROF".into(), MacroBody::Object("2".into()));
    macros.insert("RUSAGE_CHILDREN".into(), MacroBody::Object("(-1)".into()));
    macros.insert("SA_ONSTACK".into(), MacroBody::Object("0x08000000".into()));
    macros.insert("SI_USER".into(), MacroBody::Object("0".into()));
    macros.insert("TIOCGWINSZ".into(), MacroBody::Object("0x5413".into()));
    macros.insert("PTHREAD_CANCEL_ENABLE".into(), MacroBody::Object("0".into()));
    macros.insert("PTHREAD_CANCEL_DISABLE".into(), MacroBody::Object("1".into()));
    macros.insert("PTHREAD_CANCEL_ASYNCHRONOUS".into(), MacroBody::Object("1".into()));
    macros.insert("PTHREAD_CANCEL_DEFERRED".into(), MacroBody::Object("0".into()));
    macros.insert("STDIN_FILENO".into(), MacroBody::Object("0".into()));
    macros.insert("STDOUT_FILENO".into(), MacroBody::Object("1".into()));
    macros.insert("STDERR_FILENO".into(), MacroBody::Object("2".into()));
    // signals (Linux)
    macros.insert("SIGHUP".into(), MacroBody::Object("1".into()));
    macros.insert("SIGINT".into(), MacroBody::Object("2".into()));
    macros.insert("SIGQUIT".into(), MacroBody::Object("3".into()));
    macros.insert("SIGILL".into(), MacroBody::Object("4".into()));
    macros.insert("SIGTRAP".into(), MacroBody::Object("5".into()));
    macros.insert("SIGABRT".into(), MacroBody::Object("6".into()));
    macros.insert("SIGBUS".into(), MacroBody::Object("7".into()));
    macros.insert("SIGFPE".into(), MacroBody::Object("8".into()));
    macros.insert("SIGKILL".into(), MacroBody::Object("9".into()));
    macros.insert("SIGUSR1".into(), MacroBody::Object("10".into()));
    macros.insert("SIGSEGV".into(), MacroBody::Object("11".into()));
    macros.insert("SIGUSR2".into(), MacroBody::Object("12".into()));
    macros.insert("SIGPIPE".into(), MacroBody::Object("13".into()));
    macros.insert("SIGALRM".into(), MacroBody::Object("14".into()));
    macros.insert("SIGTERM".into(), MacroBody::Object("15".into()));
    macros.insert("SIGCHLD".into(), MacroBody::Object("17".into()));
    macros.insert("SIGCONT".into(), MacroBody::Object("18".into()));
    macros.insert("SIGSTOP".into(), MacroBody::Object("19".into()));
    macros.insert("SIGTSTP".into(), MacroBody::Object("20".into()));
    macros.insert("SA_RESTART".into(), MacroBody::Object("0x10000000".into()));
    macros.insert("SA_NODEFER".into(), MacroBody::Object("0x40000000".into()));
    macros.insert("SA_RESETHAND".into(), MacroBody::Object("0x80000000".into()));
    macros.insert("SA_SIGINFO".into(), MacroBody::Object("4".into()));
    macros.insert("AF_LOCAL".into(), MacroBody::Object("1".into()));
    macros.insert("EDOM".into(), MacroBody::Object("33".into()));
    macros.insert("ERANGE".into(), MacroBody::Object("34".into()));
    macros.insert("EAGAIN".into(), MacroBody::Object("11".into()));
    macros.insert("EWOULDBLOCK".into(), MacroBody::Object("11".into()));
    macros.insert("EINTR".into(), MacroBody::Object("4".into()));
    macros.insert("ECONNRESET".into(), MacroBody::Object("104".into()));
    macros.insert("EPIPE".into(), MacroBody::Object("32".into()));
    macros.insert("RUSAGE_SELF".into(), MacroBody::Object("0".into()));
    macros.insert("RLIMIT_NOFILE".into(), MacroBody::Object("7".into()));
    macros.insert("M_PI".into(), MacroBody::Object("3.14159265358979323846".into()));
    macros.insert("FP_ZERO".into(), MacroBody::Object("2".into()));
    macros.insert("FP_NORMAL".into(), MacroBody::Object("4".into()));
    macros.insert("RTLD_LAZY".into(), MacroBody::Object("1".into()));
    macros.insert("RTLD_NOW".into(), MacroBody::Object("2".into()));
    macros.insert("RTLD_GLOBAL".into(), MacroBody::Object("256".into()));
    macros.insert("RTLD_LOCAL".into(), MacroBody::Object("0".into()));
    macros.insert("RTLD_DEFAULT".into(), MacroBody::Object("((void*)0)".into()));
    macros.insert("RTLD_NEXT".into(), MacroBody::Object("((void*)-1)".into()));
    macros.insert("PRIO_PROCESS".into(), MacroBody::Object("0".into()));
    macros.insert("PRIO_PGRP".into(), MacroBody::Object("1".into()));
    macros.insert("PRIO_USER".into(), MacroBody::Object("2".into()));
    macros.insert("WNOHANG".into(), MacroBody::Object("1".into()));
    macros.insert("WUNTRACED".into(), MacroBody::Object("2".into()));
    macros.insert("AF_UNIX".into(), MacroBody::Object("1".into()));
    macros.insert("AF_INET".into(), MacroBody::Object("2".into()));
    // Linux aarch64/x86_64 values (Redis networking / posix_fadvise)
    macros.insert("IPV6_V6ONLY".into(), MacroBody::Object("26".into()));
    macros.insert("POSIX_FADV_NORMAL".into(), MacroBody::Object("0".into()));
    macros.insert("POSIX_FADV_RANDOM".into(), MacroBody::Object("1".into()));
    macros.insert("POSIX_FADV_SEQUENTIAL".into(), MacroBody::Object("2".into()));
    macros.insert("POSIX_FADV_WILLNEED".into(), MacroBody::Object("3".into()));
    macros.insert("POSIX_FADV_DONTNEED".into(), MacroBody::Object("4".into()));
    macros.insert("POSIX_FADV_NOREUSE".into(), MacroBody::Object("5".into()));
    macros.insert("SOCK_STREAM".into(), MacroBody::Object("1".into()));
    macros.insert("SOCK_DGRAM".into(), MacroBody::Object("2".into()));
    macros.insert("SOL_SOCKET".into(), MacroBody::Object("1".into()));
    macros.insert("SO_REUSEADDR".into(), MacroBody::Object("2".into()));
    macros.insert("SHUT_RD".into(), MacroBody::Object("0".into()));
    macros.insert("SHUT_WR".into(), MacroBody::Object("1".into()));
    macros.insert("SHUT_RDWR".into(), MacroBody::Object("2".into()));
    macros.insert("LOCK_SH".into(), MacroBody::Object("1".into()));
    macros.insert("LOCK_EX".into(), MacroBody::Object("2".into()));
    macros.insert("LOCK_NB".into(), MacroBody::Object("4".into()));
    macros.insert("LOCK_UN".into(), MacroBody::Object("8".into()));
    macros.insert("F_OK".into(), MacroBody::Object("0".into())); // idempotent if already set
    // syslog
    macros.insert("LOG_PID".into(), MacroBody::Object("1".into()));
    macros.insert("LOG_USER".into(), MacroBody::Object("8".into()));
    macros.insert("LOG_ERR".into(), MacroBody::Object("3".into()));
    // poll
    macros.insert("POLLIN".into(), MacroBody::Object("1".into()));
    macros.insert("POLLOUT".into(), MacroBody::Object("4".into()));
    macros.insert("POLLERR".into(), MacroBody::Object("8".into()));
    macros.insert("POLLHUP".into(), MacroBody::Object("16".into()));
    // fsync flags
    macros.insert("SYNC_FILE_RANGE_WAIT_BEFORE".into(), MacroBody::Object("1".into()));
    macros.insert("SYNC_FILE_RANGE_WRITE".into(), MacroBody::Object("2".into()));
    macros.insert("SYNC_FILE_RANGE_WAIT_AFTER".into(), MacroBody::Object("4".into()));
    macros.insert("F_GETFL".into(), MacroBody::Object("3".into()));
    macros.insert("F_SETFL".into(), MacroBody::Object("4".into()));
    macros.insert("F_GETLK".into(), MacroBody::Object("5".into()));
    macros.insert("F_SETLK".into(), MacroBody::Object("6".into()));
    macros.insert("F_SETLKW".into(), MacroBody::Object("7".into()));
    macros.insert("F_RDLCK".into(), MacroBody::Object("0".into()));
    macros.insert("F_WRLCK".into(), MacroBody::Object("1".into()));
    macros.insert("F_UNLCK".into(), MacroBody::Object("2".into()));
    macros.insert("O_NONBLOCK".into(), MacroBody::Object("2048".into()));
    macros.insert("O_CLOEXEC".into(), MacroBody::Object("524288".into()));
    macros.insert("O_NOFOLLOW".into(), MacroBody::Object("131072".into()));
    macros.insert("O_BINARY".into(), MacroBody::Object("0".into()));
    macros.insert("O_LARGEFILE".into(), MacroBody::Object("0".into()));
    macros.insert("S_IFMT".into(), MacroBody::Object("61440".into()));
    macros.insert("S_IFREG".into(), MacroBody::Object("32768".into()));
    macros.insert("S_IFDIR".into(), MacroBody::Object("16384".into()));
    macros.insert("S_IFCHR".into(), MacroBody::Object("8192".into()));
    macros.insert("S_IFBLK".into(), MacroBody::Object("24576".into()));
    macros.insert("S_IFIFO".into(), MacroBody::Object("4096".into()));
    macros.insert("S_IFSOCK".into(), MacroBody::Object("49152".into()));
    macros.insert("S_IFLNK".into(), MacroBody::Object("40960".into()));
    macros.insert("S_IRUSR".into(), MacroBody::Object("256".into()));
    macros.insert("S_IWUSR".into(), MacroBody::Object("128".into()));
    macros.insert("S_IXUSR".into(), MacroBody::Object("64".into()));
    // mode test macros (function-like)
    macros.insert(
        "S_ISDIR".into(),
        MacroBody::Function {
            params: vec!["m".into()],
            body: "(((m)&61440)==16384)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "S_ISREG".into(),
        MacroBody::Function {
            params: vec!["m".into()],
            body: "(((m)&61440)==32768)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "S_ISLNK".into(),
        MacroBody::Function {
            params: vec!["m".into()],
            body: "(((m)&61440)==40960)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "S_ISCHR".into(),
        MacroBody::Function {
            params: vec!["m".into()],
            body: "(((m)&61440)==8192)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "S_ISBLK".into(),
        MacroBody::Function {
            params: vec!["m".into()],
            body: "(((m)&61440)==24576)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "S_ISFIFO".into(),
        MacroBody::Function {
            params: vec!["m".into()],
            body: "(((m)&61440)==4096)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "S_ISSOCK".into(),
        MacroBody::Function {
            params: vec!["m".into()],
            body: "(((m)&61440)==49152)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "PTHREAD_MUTEX_INITIALIZER".into(),
        MacroBody::Object("{0}".into()),
    );
    macros.insert(
        "PTHREAD_COND_INITIALIZER".into(),
        MacroBody::Object("{0}".into()),
    );
    macros.insert(
        "PTHREAD_ONCE_INIT".into(),
        MacroBody::Object("0".into()),
    );
    macros.insert("DT_UNKNOWN".into(), MacroBody::Object("0".into()));
    macros.insert("DT_FIFO".into(), MacroBody::Object("1".into()));
    macros.insert("DT_CHR".into(), MacroBody::Object("2".into()));
    macros.insert("DT_DIR".into(), MacroBody::Object("4".into()));
    macros.insert("DT_BLK".into(), MacroBody::Object("6".into()));
    macros.insert("DT_REG".into(), MacroBody::Object("8".into()));
    macros.insert("DT_LNK".into(), MacroBody::Object("10".into()));
    macros.insert("DT_SOCK".into(), MacroBody::Object("12".into()));
    // Linux errno constants (values approximate / common Linux ABI)
    for (name, val) in [
        ("EPERM", "1"),
        ("ENOENT", "2"),
        ("ESRCH", "3"),
        ("EINTR", "4"),
        ("EIO", "5"),
        ("ENXIO", "6"),
        ("E2BIG", "7"),
        ("ENOEXEC", "8"),
        ("EBADF", "9"),
        ("ECHILD", "10"),
        ("EAGAIN", "11"),
        ("EWOULDBLOCK", "11"),
        ("ENOMEM", "12"),
        ("EACCES", "13"),
        ("EFAULT", "14"),
        ("EBUSY", "16"),
        ("EEXIST", "17"),
        ("EXDEV", "18"),
        ("ENODEV", "19"),
        ("ENOTDIR", "20"),
        ("EISDIR", "21"),
        ("EINVAL", "22"),
        ("ENFILE", "23"),
        ("EMFILE", "24"),
        ("ENOTTY", "25"),
        ("EFBIG", "27"),
        ("ENOSPC", "28"),
        ("ESPIPE", "29"),
        ("EROFS", "30"),
        ("EPIPE", "32"),
        ("ERANGE", "34"),
        ("EDEADLK", "35"),
        ("ENAMETOOLONG", "36"),
        ("ENOLCK", "37"),
        ("ENOSYS", "38"),
        ("ENOTEMPTY", "39"),
        ("ELOOP", "40"),
        ("ENOMSG", "42"),
        ("EIDRM", "43"),
        ("ENODATA", "61"),
        ("ETIME", "62"),
        ("ENOSR", "63"),
        ("EREMOTE", "66"),
        ("ENOLINK", "67"),
        ("EPROTO", "71"),
        ("EMULTIHOP", "72"),
        ("EBADMSG", "74"),
        ("EOVERFLOW", "75"),
        ("EILSEQ", "84"),
        ("EUSERS", "87"),
        ("ENOTSOCK", "88"),
        ("EDESTADDRREQ", "89"),
        ("EMSGSIZE", "90"),
        ("EPROTOTYPE", "91"),
        ("ENOPROTOOPT", "92"),
        ("EPROTONOSUPPORT", "93"),
        ("ESOCKTNOSUPPORT", "94"),
        ("EOPNOTSUPP", "95"),
        ("ENOTSUP", "95"),
        ("EPFNOSUPPORT", "96"),
        ("EAFNOSUPPORT", "97"),
        ("EADDRINUSE", "98"),
        ("EADDRNOTAVAIL", "99"),
        ("ENETDOWN", "100"),
        ("ENETUNREACH", "101"),
        ("ENETRESET", "102"),
        ("ECONNABORTED", "103"),
        ("ECONNRESET", "104"),
        ("ENOBUFS", "105"),
        ("EISCONN", "106"),
        ("ENOTCONN", "107"),
        ("ESHUTDOWN", "108"),
        ("ETOOMANYREFS", "109"),
        ("ETIMEDOUT", "110"),
        ("ECONNREFUSED", "111"),
        ("EHOSTDOWN", "112"),
        ("EHOSTUNREACH", "113"),
        ("EALREADY", "114"),
        ("EINPROGRESS", "115"),
        ("ESTALE", "116"),
        ("EDQUOT", "122"),
        ("ECANCELED", "125"),
        ("EOWNERDEAD", "130"),
        ("ENOTRECOVERABLE", "131"),
    ] {
        macros.insert(name.into(), MacroBody::Object(val.into()));
    }
    // Predefined macros matching a 64-bit LP64 host
    macros.insert("__LP64__".into(), MacroBody::Object("1".into()));
    macros.insert("_LP64".into(), MacroBody::Object("1".into()));
    macros.insert("__SIZEOF_POINTER__".into(), MacroBody::Object("8".into()));
    macros.insert("__SIZEOF_LONG__".into(), MacroBody::Object("8".into()));
    // Kernel uapi/linux/types.h gates __u128 on this (cmpxchg128 etc.).
    macros.insert("__SIZEOF_INT128__".into(), MacroBody::Object("16".into()));
    // stdint/stddef limits. ggcc uses signed idiv for `/`, so SIZE_MAX must
    // fit signed 64-bit or `SIZE_MAX/2` collapses to 0 (redis zmalloc OOM).
    // 2^63-1 is correct for SSIZE_MAX/PTRDIFF and safe for size checks.
    macros.insert("SIZE_MAX".into(), MacroBody::Object("9223372036854775807UL".into()));
    macros.insert("UINT64_MAX".into(), MacroBody::Object("9223372036854775807ULL".into()));
    macros.insert("UINT32_MAX".into(), MacroBody::Object("4294967295U".into()));
    macros.insert("INT64_MAX".into(), MacroBody::Object("9223372036854775807LL".into()));
    macros.insert("INT32_MAX".into(), MacroBody::Object("2147483647".into()));
    macros.insert("PTRDIFF_MAX".into(), MacroBody::Object("9223372036854775807L".into()));
    macros.insert("SSIZE_MAX".into(), MacroBody::Object("9223372036854775807L".into()));
    // limits.h-style (Lua luaconf selects long long via defined(LLONG_MAX)).
    macros.insert("CHAR_BIT".into(), MacroBody::Object("8".into()));
    macros.insert("SCHAR_MIN".into(), MacroBody::Object("(-128)".into()));
    macros.insert("SCHAR_MAX".into(), MacroBody::Object("127".into()));
    macros.insert("UCHAR_MAX".into(), MacroBody::Object("255".into()));
    macros.insert("CHAR_MIN".into(), MacroBody::Object("(-128)".into()));
    macros.insert("CHAR_MAX".into(), MacroBody::Object("127".into()));
    macros.insert("SHRT_MIN".into(), MacroBody::Object("(-32768)".into()));
    macros.insert("SHRT_MAX".into(), MacroBody::Object("32767".into()));
    macros.insert("USHRT_MAX".into(), MacroBody::Object("65535".into()));
    macros.insert("INT_MIN".into(), MacroBody::Object("(-2147483647-1)".into()));
    macros.insert("INT_MAX".into(), MacroBody::Object("2147483647".into()));
    // Critical for Lua L_INTHASBITS / MAXARG_Bx (else OFFSET_sBx becomes INT_MAX/2).
    macros.insert("UINT_MAX".into(), MacroBody::Object("4294967295U".into()));
    macros.insert("LONG_MIN".into(), MacroBody::Object("(-9223372036854775807L-1)".into()));
    macros.insert("LONG_MAX".into(), MacroBody::Object("9223372036854775807L".into()));
    macros.insert("ULONG_MAX".into(), MacroBody::Object("18446744073709551615UL".into()));
    macros.insert("LLONG_MIN".into(), MacroBody::Object("(-9223372036854775807LL-1)".into()));
    macros.insert("LLONG_MAX".into(), MacroBody::Object("9223372036854775807LL".into()));
    macros.insert("ULLONG_MAX".into(), MacroBody::Object("18446744073709551615ULL".into()));
    // float.h / math.h / stdlib / time / locale — needed when system headers are
    // not fully ingested (Lua loslib/lmathlib/lstrlib/lua.c).
    macros.insert("FLT_RADIX".into(), MacroBody::Object("2".into()));
    macros.insert("FLT_MANT_DIG".into(), MacroBody::Object("24".into()));
    macros.insert("DBL_MANT_DIG".into(), MacroBody::Object("53".into()));
    macros.insert("LDBL_MANT_DIG".into(), MacroBody::Object("64".into()));
    macros.insert("FLT_DIG".into(), MacroBody::Object("6".into()));
    macros.insert("DBL_DIG".into(), MacroBody::Object("15".into()));
    macros.insert("FLT_MIN_EXP".into(), MacroBody::Object("(-125)".into()));
    macros.insert("DBL_MIN_EXP".into(), MacroBody::Object("(-1021)".into()));
    macros.insert("FLT_MAX_EXP".into(), MacroBody::Object("128".into()));
    macros.insert("DBL_MAX_EXP".into(), MacroBody::Object("1024".into()));
    macros.insert("FLT_MIN_10_EXP".into(), MacroBody::Object("(-37)".into()));
    macros.insert("DBL_MIN_10_EXP".into(), MacroBody::Object("(-307)".into()));
    macros.insert("FLT_MAX_10_EXP".into(), MacroBody::Object("38".into()));
    macros.insert("DBL_MAX_10_EXP".into(), MacroBody::Object("308".into()));
    macros.insert("FLT_MAX".into(), MacroBody::Object("3.402823466e+38".into()));
    macros.insert("DBL_MAX".into(), MacroBody::Object("1.7976931348623157e+308".into()));
    macros.insert("FLT_MIN".into(), MacroBody::Object("1.175494351e-38".into()));
    macros.insert("DBL_MIN".into(), MacroBody::Object("2.2250738585072014e-308".into()));
    macros.insert("FLT_EPSILON".into(), MacroBody::Object("1.192092896e-07".into()));
    macros.insert("DBL_EPSILON".into(), MacroBody::Object("2.2204460492503131e-16".into()));
    // Soft numeric stand-ins (no libm sentinel symbols required at link).
    macros.insert("HUGE_VAL".into(), MacroBody::Object("1.0e300".into()));
    macros.insert("HUGE_VALF".into(), MacroBody::Object("1.0e38".into()));
    macros.insert("INFINITY".into(), MacroBody::Object("1.0e300".into()));
    macros.insert("NAN".into(), MacroBody::Object("0.0".into()));
    macros.insert("CLOCKS_PER_SEC".into(), MacroBody::Object("1000000".into()));
    macros.insert("EXIT_SUCCESS".into(), MacroBody::Object("0".into()));
    macros.insert("EXIT_FAILURE".into(), MacroBody::Object("1".into()));
    macros.insert("LC_ALL".into(), MacroBody::Object("0".into()));
    macros.insert("LC_COLLATE".into(), MacroBody::Object("1".into()));
    macros.insert("LC_CTYPE".into(), MacroBody::Object("2".into()));
    macros.insert("LC_MONETARY".into(), MacroBody::Object("3".into()));
    macros.insert("LC_NUMERIC".into(), MacroBody::Object("4".into()));
    macros.insert("LC_TIME".into(), MacroBody::Object("5".into()));
    macros.insert("RAND_MAX".into(), MacroBody::Object("2147483647".into()));
    macros.insert("BUFSIZ".into(), MacroBody::Object("1024".into()));
    macros.insert("FOPEN_MAX".into(), MacroBody::Object("20".into()));
    macros.insert("TMP_MAX".into(), MacroBody::Object("308915776".into()));
    macros.insert("L_tmpnam".into(), MacroBody::Object("1024".into()));
    macros.insert("_IOFBF".into(), MacroBody::Object("0".into()));
    macros.insert("_IOLBF".into(), MacroBody::Object("1".into()));
    macros.insert("_IONBF".into(), MacroBody::Object("2".into()));
    macros.insert("SIGINT".into(), MacroBody::Object("2".into()));
    macros.insert("SIG_DFL".into(), MacroBody::Object("((void (*)(int))0)".into()));
    macros.insert("SIG_IGN".into(), MacroBody::Object("((void (*)(int))1)".into()));
    // BSD/Darwin ioctl helpers (sqlite afpSetLock) — soft to 0 when headers missing.
    macros.insert(
        "_IOWR".into(),
        MacroBody::Function {
            params: vec!["g".into(), "n".into(), "t".into()],
            body: "0".into(),
            variadic: false,
        },
    );
    macros.insert(
        "_IOR".into(),
        MacroBody::Function {
            params: vec!["g".into(), "n".into(), "t".into()],
            body: "0".into(),
            variadic: false,
        },
    );
    macros.insert(
        "_IOW".into(),
        MacroBody::Function {
            params: vec!["g".into(), "n".into(), "t".into()],
            body: "0".into(),
            variadic: false,
        },
    );
    macros.insert(
        "_IO".into(),
        MacroBody::Function {
            params: vec!["g".into(), "n".into()],
            body: "0".into(),
            variadic: false,
        },
    );
    macros.insert("MAXPATHLEN".into(), MacroBody::Object("1024".into()));
    macros.insert("PATH_MAX".into(), MacroBody::Object("1024".into()));
    macros.insert("MNT_LOCAL".into(), MacroBody::Object("0".into()));
    macros.insert("MNT_RDONLY".into(), MacroBody::Object("1".into()));
    macros.insert("MNT_NOSUID".into(), MacroBody::Object("2".into()));
    macros.insert("MNT_NODEV".into(), MacroBody::Object("4".into()));
    macros.insert("S_IRUSR".into(), MacroBody::Object("256".into()));
    macros.insert("S_IWUSR".into(), MacroBody::Object("128".into()));
    macros.insert("S_IXUSR".into(), MacroBody::Object("64".into()));
    macros.insert("S_IRGRP".into(), MacroBody::Object("32".into()));
    macros.insert("S_IWGRP".into(), MacroBody::Object("16".into()));
    macros.insert("S_IXGRP".into(), MacroBody::Object("8".into()));
    macros.insert("S_IROTH".into(), MacroBody::Object("4".into()));
    macros.insert("S_IWOTH".into(), MacroBody::Object("2".into()));
    macros.insert("S_IXOTH".into(), MacroBody::Object("1".into()));
    macros.insert("S_IFMT".into(), MacroBody::Object("61440".into()));
    macros.insert("S_IFREG".into(), MacroBody::Object("32768".into()));
    macros.insert("S_IFDIR".into(), MacroBody::Object("16384".into()));
    macros.insert("__ATOMIC_RELAXED".into(), MacroBody::Object("0".into()));
    macros.insert("__ATOMIC_ACQUIRE".into(), MacroBody::Object("2".into()));
    macros.insert("__ATOMIC_RELEASE".into(), MacroBody::Object("3".into()));
    macros.insert("__ATOMIC_ACQ_REL".into(), MacroBody::Object("4".into()));
    macros.insert("__ATOMIC_SEQ_CST".into(), MacroBody::Object("5".into()));
    macros.insert(
        "__atomic_load_n".into(),
        MacroBody::Function {
            params: vec!["p".into(), "o".into()],
            body: "(*(p))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "__atomic_store_n".into(),
        MacroBody::Function {
            params: vec!["p".into(), "v".into(), "o".into()],
            body: "((void)((*(p)) = (v)))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "__sync_synchronize".into(),
        MacroBody::Function {
            params: vec![],
            body: "((void)0)".into(),
            variadic: true,
        },
    );
    // Do NOT soft-macro __builtin_clzll — parser folds it for order_base_2 /
    // ilog2 / kbuild DEFINE. The old `((x)==0?64:0)` made NR_CPUS_BITS=64.
    //
    // Do NOT soft-macro __builtin_{add,sub,mul}_overflow either.
    // The old `((*r)=((a)+(b)),0)` always reported "no overflow", which made
    // SQLite's sqlite3AddInt64 (taken under #if GCC_VERSION>=5004000 because we
    // advertise __GNUC__=13) silently wrap i64_max+1 → i64_min and break
    // where-27 / integer-affinity tests. Real overflow is emitted in codegen.
    // Do NOT macro-define size_t/uintptr_t/etc. — they are typedef'd in the
    // soft prefix. A `#define size_t unsigned long` would break
    // `typedef unsigned long size_t` into `typedef unsigned long unsigned long`.
    // Arch predefined macros — must match codegen target (x86 kernel ≠ aarch64).
    match arch {
        "x86_64" | "x86" | "amd64" => {
            macros.insert("__x86_64__".into(), MacroBody::Object("1".into()));
            macros.insert("__x86_64".into(), MacroBody::Object("1".into()));
            macros.insert("__amd64__".into(), MacroBody::Object("1".into()));
            macros.insert("__amd64".into(), MacroBody::Object("1".into()));
            // Linux x86_64: O_NOFOLLOW=0400000
            macros.insert("O_NOFOLLOW".into(), MacroBody::Object("131072".into()));
        }
        _ => {
            macros.insert("__aarch64__".into(), MacroBody::Object("1".into()));
            macros.insert("__ARM_64BIT_STATE".into(), MacroBody::Object("1".into()));
            // Linux aarch64 (asm-generic): O_NOFOLLOW=0100000 (=32768), not x86's 0400000.
            macros.insert("O_NOFOLLOW".into(), MacroBody::Object("32768".into()));
        }
    }
    // ISO C required static predefined macros (__LINE__/__FILE__ are dynamic specials).
    macros.insert("__STDC__".into(), MacroBody::Object("1".into()));
    macros.insert("__STDC_HOSTED__".into(), MacroBody::Object("1".into()));
    macros.insert("__STDC_VERSION__".into(), MacroBody::Object("201112L".into()));
    // Pretend GCC enough for Linux/kernel header guards (Kconfig already saw real gcc -E).
    macros.insert("__GNUC__".into(), MacroBody::Object("13".into()));
    macros.insert("__GNUC_MINOR__".into(), MacroBody::Object("0".into()));
    macros.insert("__GNUC_PATCHLEVEL__".into(), MacroBody::Object("0".into()));
    // C11 atomics → plain ops for freestanding smoke (Redis zmalloc etc.).
    // Real multi-thread atomics not required for Stage C2 single-thread tests.
    macros.insert(
        "atomic_fetch_add_explicit".into(),
        MacroBody::Function {
            params: vec!["p".into(), "v".into(), "o".into()],
            body: "((*(p)) += (v), (*(p)) - (v))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "atomic_fetch_sub_explicit".into(),
        MacroBody::Function {
            params: vec!["p".into(), "v".into(), "o".into()],
            body: "((*(p)) -= (v), (*(p)) + (v))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "atomic_load_explicit".into(),
        MacroBody::Function {
            params: vec!["p".into(), "o".into()],
            body: "(*(p))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "atomic_store_explicit".into(),
        MacroBody::Function {
            params: vec!["p".into(), "v".into(), "o".into()],
            body: "((void)((*(p)) = (v)))".into(),
            variadic: false,
        },
    );
    macros.insert("memory_order_relaxed".into(), MacroBody::Object("0".into()));
    macros.insert("memory_order_acquire".into(), MacroBody::Object("0".into()));
    macros.insert("memory_order_release".into(), MacroBody::Object("0".into()));
    macros.insert("memory_order_seq_cst".into(), MacroBody::Object("0".into()));
    if !for_linux {
        macros.insert("__APPLE__".into(), MacroBody::Object("1".into()));
        macros.insert("__MACH__".into(), MacroBody::Object("1".into()));
        // SQLite mem1.c Apple path: `_sqliteZone_->size(zone,p)` needs full
        // malloc_zone_t layout. Without system header field offsets, codegen
        // loads reserved1 (often null) as the size fn → SIGSEGV in open().
        // Use the plain malloc/malloc_size path instead (still real libc).
        macros.insert(
            "SQLITE_WITHOUT_ZONEMALLOC".into(),
            MacroBody::Object("1".into()),
        );
        // Darwin/user code (Redis sds packed headers): do NOT empty-macro
        // __attribute__ here — the lexer turns `__attribute__((packed))` into
        // TokenKind::Packed so struct layout stays 1-byte aligned.
    } else {
        macros.insert("__linux__".into(), MacroBody::Object("1".into()));
        macros.insert("linux".into(), MacroBody::Object("1".into()));
        macros.insert("__linux".into(), MacroBody::Object("1".into()));
        // Erase GNU attributes at PP time for kernel headers. Nested attribute
        // macros otherwise explode into multi-KB tokens that break parsing.
        // Exception: `__weak` must survive (COND_SYSCALL stubs). Map it to a
        // sticky marker the lexer turns into TokenKind::Weak — if we leave
        // `__weak` → `__attribute__((__weak__))` it would be erased below.
        macros.insert(
            "__weak".into(),
            MacroBody::Object("__ggcc_weak_attr".into()),
        );
        macros.insert(
            "__attribute__".into(),
            MacroBody::Function {
                params: vec![],
                body: "".into(),
                variadic: true,
            },
        );
        macros.insert(
            "__attribute".into(),
            MacroBody::Function {
                params: vec![],
                body: "".into(),
                variadic: true,
            },
        );
        // Kernel address-space / sparse markers — normally attribute macros;
        // force empty so `const char __user *p` parses as `const char *p`.
        for q in [
            "__user",
            "__kernel",
            "__iomem",
            "__percpu",
            "__rcu",
            "__force",
            "__chk_user_ptr",
            "__chk_io_ptr",
            "__builtin_warning",
            "__must_hold",
            "__acquires",
            "__releases",
            "__no_kasan_or_inline",
            "__no_sanitize_address",
            "__no_sanitize_coverage",
            "notrace",
            "__notrace",
            "__sched",
            "__always_inline",
            "__gnu_inline",
            "__cold",
            "__hot",
            "__flatten",
            "__pure",
            "__noreturn",
            "__malloc",
            "__must_check",
            "__cond_lock",
            "__private",
            "__safe",
            "__nocast",
            "__pmem",
            "__vmlinux_symbol",
            "__kernel_symbol",
        ] {
            macros.insert(q.into(), MacroBody::Object("".into()));
        }
        // EXPORT_SYMBOL* expand to multi-line asm/section soup that our asm
        // parser cannot consume; kbuild linking does not need them for .o
        // generation under Stage C fail-drive (symbols stay global via .globl).
        for exp in [
            "EXPORT_SYMBOL",
            "EXPORT_SYMBOL_GPL",
            "EXPORT_SYMBOL_NS",
            "EXPORT_SYMBOL_NS_GPL",
            "EXPORT_SYMBOL_GPL_FUTURE",
            "EXPORT_UNUSED_SYMBOL",
            "EXPORT_UNUSED_SYMBOL_GPL",
            "EXPORT_DATA_SYMBOL",
            "EXPORT_DATA_SYMBOL_GPL",
            "__EXPORT_SYMBOL",
            "EXPORT_STATIC_CALL",
            "EXPORT_STATIC_CALL_GPL",
            "EXPORT_STATIC_CALL_TRAMP",
            "EXPORT_STATIC_CALL_TRAMP_GPL",
        ] {
            macros.insert(
                exp.into(),
                MacroBody::Function {
                    params: vec!["sym".into()],
                    body: "/*export*/".into(),
                    variadic: true,
                },
            );
        }
    }
    // stdarg: ggcc uses a char* cursor into the GP regsave for internal
    // va_arg (sqlite3_mprintf etc.). Linux libc v*printf needs AAPCS64
    // va_list — codegen converts at the call site.
    macros.insert(
        "va_start".into(),
        MacroBody::Function {
            params: vec!["ap".into(), "last".into()],
            body: "((void)(last), (ap) = __ggcc_va_start())".into(),
            variadic: false,
        },
    );
    macros.insert(
        "va_arg".into(),
        MacroBody::Function {
            params: vec!["ap".into(), "type".into()],
            // type is substituted textually (may include `*`).
            // Codegen rewrites *(double*)__ggcc_va_arg to the VR walker.
            body: "(*(type*)__ggcc_va_arg(&(ap)))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "va_end".into(),
        MacroBody::Function {
            params: vec!["ap".into()],
            body: "((void)(ap))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "va_copy".into(),
        MacroBody::Function {
            params: vec!["d".into(), "s".into()],
            body: "((d) = (s))".into(),
            variadic: false,
        },
    );
    let stdio_syms = if for_linux {
        "typedef struct __FILE FILE;\nextern FILE *stdout;\nextern FILE *stderr;\nextern FILE *stdin;\n"
    } else {
        #[cfg(target_os = "macos")]
        {
            macros.insert("stdout".into(), MacroBody::Object("__stdoutp".into()));
            macros.insert("stderr".into(), MacroBody::Object("__stderrp".into()));
            macros.insert("stdin".into(), MacroBody::Object("__stdinp".into()));
        }
        if cfg!(target_os = "macos") {
            "typedef struct __FILE FILE;\nextern FILE *__stdoutp;\nextern FILE *__stderrp;\nextern FILE *__stdinp;\n"
        } else {
            "typedef struct __FILE FILE;\nextern FILE *stdout;\nextern FILE *stderr;\nextern FILE *stdin;\n"
        }
    };
    // Stubs so public headers (sqlite3.h, lua setjmp, etc.) parse without full libc.
    // Always inject — Darwin host Stage B also needs these (system <pthread.h>/<setjmp.h>
    // are skipped when angle includes are missing).
    macros.insert(
        "PTHREAD_MUTEX_RECURSIVE".into(),
        MacroBody::Object("1".into()),
    );
    macros.insert(
        "PTHREAD_MUTEX_NORMAL".into(),
        MacroBody::Object("0".into()),
    );
    macros.insert(
        "PTHREAD_MUTEX_ERRORCHECK".into(),
        MacroBody::Object("2".into()),
    );
    macros.insert(
        "PTHREAD_MUTEX_DEFAULT".into(),
        MacroBody::Object("0".into()),
    );
    macros.insert(
        "PTHREAD_CREATE_JOINABLE".into(),
        MacroBody::Object("0".into()),
    );
    macros.insert(
        "PTHREAD_CREATE_DETACHED".into(),
        MacroBody::Object("1".into()),
    );
    // Darwin arm64/x86_64: sizeof(jmp_buf)==192 (int[48]). Linux aarch64/x86_64 often
    // similar; oversize is safe for stack, undersize corrupts (Lua setjmp crash).
    // Mach soft types only when !for_linux — kernel vdso discards .data, and enum
    // constants as .data symbols break the vdso link (MACH_PORT_NULL etc.).
    let pthread_stubs = if for_linux {
        // Kernel fixed-width types (uapi int-ll64) — soft inject so casts like
        // `*(__u8 *)p` parse even when header include order/guards skip them.
        "typedef signed char __s8;\n\
         typedef unsigned char __u8;\n\
         typedef signed short __s16;\n\
         typedef unsigned short __u16;\n\
         typedef signed int __s32;\n\
         typedef unsigned int __u32;\n\
         typedef signed long long __s64;\n\
         typedef unsigned long long __u64;\n\
         typedef unsigned long long __uint128_t;\n\
         typedef long long __int128_t;\n\
         typedef __u8 u8;\n\
         typedef __u16 u16;\n\
         typedef __u32 u32;\n\
         typedef __u64 u64;\n\
         typedef __s8 s8;\n\
         typedef __s16 s16;\n\
         typedef __s32 s32;\n\
         typedef __s64 s64;\n\
         /* glibc aarch64 sizes + 8-byte alignment (long[] not char[]): mutex/cond
          * =48, attr=8, rwlock=56, attr_t=64. char[N] soft types had align 1 so
          * mutexes sat at odd offsets → pthread_mutex_lock owner assert. */\n\
         typedef struct { long __s[6]; } pthread_mutex_t;\n\
         typedef struct { long __s[1]; } pthread_mutexattr_t;\n\
         typedef unsigned long pthread_t;\n\
         typedef struct { long __s[6]; } pthread_cond_t;\n\
         typedef struct { long __s[1]; } pthread_condattr_t;\n\
         typedef int pthread_once_t;\n\
         typedef int pthread_key_t;\n\
         typedef struct { long __s[8]; } pthread_attr_t;\n\
         typedef struct { long __s[7]; } pthread_rwlock_t;\n\
         typedef int jmp_buf[48];\n\
         typedef int sigjmp_buf[48];\n\
         int getpagesize(void);\n\
         int getpid(void);\n"
    } else {
        "typedef struct { char __s[64]; } pthread_mutex_t;\n\
         typedef struct { char __s[64]; } pthread_mutexattr_t;\n\
         typedef unsigned long pthread_t;\n\
         typedef struct { char __s[64]; } pthread_cond_t;\n\
         typedef struct { char __s[8]; } pthread_condattr_t;\n\
         typedef struct { char __s[8]; } pthread_once_t;\n\
         typedef int pthread_key_t;\n\
         typedef struct { char __s[64]; } pthread_attr_t;\n\
         typedef int pthread_rwlock_t;\n\
         typedef int jmp_buf[48];\n\
         typedef int sigjmp_buf[48];\n\
         typedef unsigned int task_t;\n\
         typedef unsigned int mach_port_t;\n\
         typedef int mach_msg_type_number_t;\n\
         typedef int *task_info_t;\n\
         typedef struct { unsigned long resident_size; unsigned long virtual_size; } task_basic_info;\n\
         typedef struct { unsigned long pri_pages_dirtied; unsigned long pri_pages_resident; } proc_regioninfo;\n\
         enum { MACH_PORT_NULL = 0, KERN_SUCCESS = 0, TASK_BASIC_INFO = 4, TASK_BASIC_INFO_COUNT = 10, PROC_PIDREGIONINFO = 7, PROC_PIDREGIONINFO_SIZE = 64 };\n\
         task_t current_task(void);\n\
         int task_for_pid(task_t, int, task_t *);\n\
         int task_info(task_t, int, task_info_t, mach_msg_type_number_t *);\n\
         int proc_pidinfo(int, int, unsigned long long, void *, int);\n\
         int getpagesize(void);\n\
         int getpid(void);\n"
    };
    let out_prefix = format!(
        "typedef int int32_t;\n\
         typedef long int64_t;\n\
         typedef short int16_t;\n\
         typedef unsigned long size_t;\n\
         typedef long ssize_t;\n\
         typedef unsigned long uintptr_t;\n\
         typedef long intptr_t;\n\
         typedef unsigned int uint32_t;\n\
         typedef unsigned short uint16_t;\n\
         typedef unsigned char uint8_t;\n\
         typedef signed char int8_t;\n\
         typedef unsigned long uint64_t;\n\
         typedef unsigned long long uintmax_t;\n\
         typedef long long intmax_t;\n\
         /* char* cursor into GP regsave; VR (d0..d7) tracked via codegen\n\
          * side state for AAPCS64 system-gcc callers (see __ggcc_va_arg_fp). */\n\
         typedef char *va_list;\n\
         /* errno: Linux userspace uses __errno_location (codegen); keep a bare\n\
          * declaration for kernel soft/freestanding. Do NOT #define errno — it\n\
          * breaks `int foo(int errno)` parameter names in kernel headers. */\n\
         extern int errno;\n\
         extern int __ggcc_errno;\n\
         int *__errno_location(void);\n\
         typedef long off_t;\n\
         typedef int pid_t;\n\
         typedef unsigned int uid_t;\n\
         typedef unsigned int gid_t;\n\
         typedef unsigned int mode_t;\n\
         typedef unsigned long dev_t;\n\
         typedef unsigned long ino_t;\n\
         typedef long blksize_t;\n\
         typedef long blkcnt_t;\n\
         /* Linux aarch64/x86_64: nlink_t is 32-bit; wrong size shifts st_mode reads. */\n\
         typedef unsigned int nlink_t;\n\
         typedef long clock_t;\n\
         typedef int socklen_t;\n\
         typedef unsigned char uuid_t[16];\n\
         /* Linux LP64: time_t is signed. Redis nolocks_localtime does t-=tz then
          * t/secs_day; unsigned time_t underflows on positive timezone → infinite
          * year loop (is_leap_year busy-spin, never listen). */\n\
         typedef long time_t;\n\
         typedef long suseconds_t;\n\
         struct timespec {{ time_t tv_sec; long tv_nsec; }};\n\
         struct timeval {{ time_t tv_sec; suseconds_t tv_usec; }};\n\
         struct tm {{\n\
           int tm_sec; int tm_min; int tm_hour; int tm_mday; int tm_mon;\n\
           int tm_year; int tm_wday; int tm_yday; int tm_isdst;\n\
           long tm_gmtoff; char *tm_zone;\n\
         }};\n\
         struct iovec {{ void *iov_base; size_t iov_len; }};\n\
         struct flock {{\n\
           short l_type; short l_whence; off_t l_start; off_t l_len; pid_t l_pid;\n\
         }};\n\
         struct utsname {{ char sysname[65]; char nodename[65]; char release[65];\n\
           char version[65]; char machine[65]; char domainname[65]; }};\n\
         struct dirent {{ ino_t d_ino; off_t d_off; unsigned short d_reclen;\n\
           unsigned char d_type; char d_name[256]; }};\n\
         typedef struct DIR DIR;\n\
         /* Layout matches glibc aarch64 struct stat (sizeof 128): st_mode@16,\n\
          * st_size@48, timespec times. libc lstat fills this; wrong layout made\n\
          * S_ISLNK/size checks and SQLite unix VFS misbehave. */\n\
         struct stat {{\n\
           dev_t st_dev; ino_t st_ino; mode_t st_mode; nlink_t st_nlink;\n\
           uid_t st_uid; gid_t st_gid; dev_t st_rdev; dev_t __st_pad1;\n\
           off_t st_size; blksize_t st_blksize; int __st_pad2; blkcnt_t st_blocks;\n\
           struct timespec st_atim; struct timespec st_mtim; struct timespec st_ctim;\n\
           int __glibc_reserved[2];\n\
         }};\n\
         /* glibc aarch64 addrinfo sizeof=48: pad after ai_addrlen so ai_addr@24.
          * Missing layout made Redis getaddrinfo results mis-read → socket()
          * \"ai_socktype not supported\" and failed listen. */\n\
         typedef unsigned short sa_family_t;\n\
         struct sockaddr {{ sa_family_t sa_family; char sa_data[14]; }};\n\
         struct sockaddr_in {{\n\
           sa_family_t sin_family; unsigned short sin_port; unsigned int sin_addr;\n\
           char sin_zero[8];\n\
         }};\n\
         struct sockaddr_in6 {{\n\
           sa_family_t sin6_family; unsigned short sin6_port; unsigned int sin6_flowinfo;\n\
           unsigned char sin6_addr[16]; unsigned int sin6_scope_id;\n\
         }};\n\
         struct addrinfo {{\n\
           int ai_flags; int ai_family; int ai_socktype; int ai_protocol;\n\
           socklen_t ai_addrlen; int __ai_pad;\n\
           struct sockaddr *ai_addr; char *ai_canonname; struct addrinfo *ai_next;\n\
         }};\n\
         /* glibc aarch64 epoll_event: sizeof=16, events@0, data@8 (4B pad).\n\
          * Missing layout put ee.data.fd at offset 0 over events → epoll never\n\
          * fires for Redis listen fd (connect ok, no accept/PING). */\n\
         typedef union epoll_data {{\n\
           void *ptr; int fd; unsigned int u32; unsigned long u64;\n\
         }} epoll_data_t;\n\
         struct epoll_event {{\n\
           unsigned int events; unsigned int __epoll_pad; epoll_data_t data;\n\
         }};\n\
         {stdio_syms}\
         {pthread_stubs}"
    );
    let mut out = out_prefix;
    preprocess_into(
        src,
        include_dir,
        extra_includes,
        &mut macros,
        &mut out,
        true,
        source_name,
    )?;
    if for_linux {
        out = soften_kernel_pp_residue(&out);
        out = soften_kernel_builtins(&out);
        out = strip_expanded_export_soup(&out);
        // Kernel headers glue entire functions onto one physical line after PP;
        // break after `;` / `}` outside strings so lex/parse stay O(n).
        out = break_glued_kernel_lines(&out);
    }
    Ok(out)
}

/// After headers expand EXPORT_SYMBOL, residual soup looks like:
///   extern typeof(f) f; static void *__UNIQUE_ID_... = &f; asm(".section "...);
/// Drop those pieces so parse does not thrash on broken asm strings.
fn strip_expanded_export_soup(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim_start();
        if t.contains("__UNIQUE_ID___addressable_")
            || t.contains("__addressable_")
            || (t.starts_with("asm(") && t.contains("export_symbol"))
            || (t.starts_with("asm (") && t.contains("export_symbol"))
            || (t.starts_with("extern typeof(") && t.contains(");") && !t.contains('{'))
        {
            // Do NOT emit `/*...*/` — may sit inside an open block comment and
            // terminate it early (then `* We can't...` becomes code).
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Insert newlines after `;` and `}` when not inside string/char/block comments.
fn break_glued_kernel_lines(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len() + src.len() / 32);
    let mut i = 0usize;
    let mut in_str = false;
    let mut in_chr = false;
    let mut in_block = false;
    let mut in_line = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_line {
            out.push(b as char);
            if b == b'\n' {
                in_line = false;
            }
            i += 1;
            continue;
        }
        if in_block {
            out.push(b as char);
            if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                out.push('/');
                i += 2;
                in_block = false;
                continue;
            }
            i += 1;
            continue;
        }
        if in_str {
            out.push(b as char);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if in_chr {
            out.push(b as char);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if b == b'\'' {
                in_chr = false;
            }
            i += 1;
            continue;
        }
        // Enter comments / strings
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            out.push_str("/*");
            i += 2;
            in_block = true;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            out.push_str("//");
            i += 2;
            in_line = true;
            continue;
        }
        match b {
            b'"' => {
                in_str = true;
                out.push('"');
            }
            b'\'' => {
                in_chr = true;
                out.push('\'');
            }
            b';' | b'}' => {
                out.push(b as char);
                // Don't double-newline if already followed by newline.
                if i + 1 < bytes.len() && bytes[i + 1] != b'\n' {
                    out.push('\n');
                }
            }
            _ => out.push(b as char),
        }
        i += 1;
    }
    out
}

/// Skip balanced `(...)` starting at `i` pointing at `(`. Returns index after `)`.
fn skip_balanced_parens(bytes: &[u8], mut i: usize) -> usize {
    if i >= bytes.len() || bytes[i] != b'(' {
        return i;
    }
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
                continue;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// Split top-level commas in a parameter list (no surrounding parens).
fn split_top_level_commas(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if start <= s.len() {
        let t = s[start..].trim();
        if !t.is_empty() {
            parts.push(t.to_string());
        }
    }
    parts
}

/// Rewrite unexpanded `SYSCALL_DEFINEn(name, type, arg, ...)` into
/// `long sys_name(type arg, ...)` so the following `{ body }` is a real def.
/// Also strip EXPORT_SYMBOL* residue headers leave behind.
fn soften_kernel_pp_residue(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
            let is_export = matches!(
                name,
                "EXPORT_SYMBOL"
                    | "EXPORT_SYMBOL_GPL"
                    | "EXPORT_SYMBOL_NS"
                    | "EXPORT_SYMBOL_NS_GPL"
                    | "EXPORT_SYMBOL_GPL_FUTURE"
                    | "EXPORT_UNUSED_SYMBOL"
                    | "EXPORT_UNUSED_SYMBOL_GPL"
                    | "EXPORT_DATA_SYMBOL"
                    | "EXPORT_DATA_SYMBOL_GPL"
                    | "__EXPORT_SYMBOL"
                    | "EXPORT_STATIC_CALL"
                    | "EXPORT_STATIC_CALL_GPL"
                    | "EXPORT_STATIC_CALL_TRAMP"
                    | "EXPORT_STATIC_CALL_TRAMP_GPL"
            );
            let is_syscall = matches!(
                name,
                "SYSCALL_DEFINE0"
                    | "SYSCALL_DEFINE1"
                    | "SYSCALL_DEFINE2"
                    | "SYSCALL_DEFINE3"
                    | "SYSCALL_DEFINE4"
                    | "SYSCALL_DEFINE5"
                    | "SYSCALL_DEFINE6"
                    | "COMPAT_SYSCALL_DEFINE0"
                    | "COMPAT_SYSCALL_DEFINE1"
                    | "COMPAT_SYSCALL_DEFINE2"
                    | "COMPAT_SYSCALL_DEFINE3"
                    | "COMPAT_SYSCALL_DEFINE4"
                    | "COMPAT_SYSCALL_DEFINE5"
                    | "COMPAT_SYSCALL_DEFINE6"
            );
            // Tracepoint macros (often left unexpanded when TRACEPOINTS soft-off).
            let is_trace = matches!(
                name,
                "DECLARE_EVENT_CLASS"
                    | "DECLARE_EVENT_CLASS_NOP"
                    | "DEFINE_EVENT"
                    | "DEFINE_EVENT_NOP"
                    | "DEFINE_EVENT_CONDITION"
                    | "DEFINE_EVENT_PRINT"
                    | "TRACE_EVENT"
                    | "TRACE_EVENT_CONDITION"
                    | "TRACE_EVENT_FN"
                    | "TRACE_EVENT_FN_COND"
                    | "TRACE_EVENT_FLAGS"
                    | "TRACE_EVENT_PERF_PERM"
                    | "TRACE_DEFINE_ENUM"
                    | "TRACE_DEFINE_SIZEOF"
                    | "TP_PROTO"
                    | "TP_ARGS"
                    | "TP_STRUCT__entry"
                    | "TP_fast_assign"
                    | "TP_printk"
                    | "TP_CONDITION"
                    | "__perf_task"
                    | "__perf_count"
                    | "__perf_addr"
                    | "DECLARE_TRACE"
                    | "DEFINE_TRACE"
                    | "DEFINE_TRACE_FN"
            ) || name.starts_with("TRACE_EVENT_")
                || name.starts_with("DECLARE_EVENT_")
                || name.starts_with("DEFINE_EVENT_");
            // x86 IDT entry declaration macros left unexpanded after soft PP.
            let is_idt_decl = name.starts_with("DECLARE_IDTENTRY");
            // DEFINE_IDTENTRY* acts as a function header; rewrite to void name(...)
            // and leave the following { body } for the parser.
            let is_idt_def = name.starts_with("DEFINE_IDTENTRY");
            // Conditional flag-name helpers from mmflags.h (often unexpanded in
            // soft PP when CONFIG_* gates don't fire the empty macro form).
            // Residue looks like `IF_HAVE_PG_IDLE(idle)` mid-initializer list.
            let is_if_have = name.starts_with("IF_HAVE_PG_")
                || name.starts_with("IF_HAVE_")
                || name.starts_with("DEF_PAGEFLAG_NAME")
                || name.starts_with("DEF_PAGETYPE_NAME")
                || name.starts_with("DEF_VMAFLAG_NAME");
            // KVM static-call table macros (KVM_X86_OP / KVM_X86_OP_OPTIONAL …)
            // partially expand; leftover KVM_X86_OP(name) breaks file-scope parse.
            let is_kvm_op = name.starts_with("KVM_X86_OP")
                || name.starts_with("KVM_X86_CALL")
                || name.starts_with("static_call_cond")
                || name == "static_call_update"
                || name == "STATIC_CALL_TRAMP_ADDR";
            if is_export
                || is_syscall
                || is_trace
                || is_idt_decl
                || is_idt_def
                || is_if_have
                || is_kvm_op
            {
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'(' {
                    let args_start = i + 1;
                    let after = skip_balanced_parens(bytes, i);
                    let args_end = after.saturating_sub(1);
                    let args_src = if args_end > args_start {
                        std::str::from_utf8(&bytes[args_start..args_end]).unwrap_or("")
                    } else {
                        ""
                    };
                    i = after;
                    if is_export || is_trace || is_if_have || is_kvm_op {
                        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                            i += 1;
                        }
                        if i < bytes.len() && bytes[i] == b';' {
                            i += 1;
                        }
                        out.push(' ');
                        continue;
                    }
                    if is_idt_decl {
                        // DECLARE_IDTENTRY(vector, func) → void asm_func(void); void func(void *regs);
                        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                            i += 1;
                        }
                        if i < bytes.len() && bytes[i] == b';' {
                            i += 1;
                        }
                        let parts = split_top_level_commas(args_src);
                        let func = parts
                            .get(1)
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("idt_handler");
                        out.push_str(&format!(
                            "void asm_{func}(void); void xen_asm_{func}(void); void fred_{func}(void *regs); void {func}(void *regs); "
                        ));
                        continue;
                    }
                    if is_idt_def {
                        // DEFINE_IDTENTRY(func) { ... } → void func(void *regs)
                        let parts = split_top_level_commas(args_src);
                        let func = parts
                            .first()
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("idt_handler");
                        out.push_str(&format!("void {func}(void *regs) "));
                        continue;
                    }
                    // SYSCALL_DEFINEn → long sys_name(...)
                    let parts = split_top_level_commas(args_src);
                    if parts.is_empty() {
                        out.push_str("/*syscall*/ long sys_unknown(void) ");
                        continue;
                    }
                    let sys_name = parts[0].trim();
                    let prefix = if name.starts_with("COMPAT_") {
                        "compat_sys_"
                    } else {
                        "sys_"
                    };
                    if parts.len() == 1 {
                        // SYSCALL_DEFINE0(name)
                        out.push_str(&format!("long {prefix}{sys_name}(void) "));
                        continue;
                    }
                    // pairs: type, arg, type, arg, ...
                    let mut params = Vec::new();
                    let mut j = 1;
                    while j + 1 < parts.len() {
                        let ty = parts[j].trim();
                        let arg = parts[j + 1].trim();
                        // Drop leftover sparse markers in type text.
                        let ty = ty
                            .replace("__user", "")
                            .replace("__kernel", "")
                            .replace("__force", "")
                            .replace("__rcu", "");
                        params.push(format!("{} {}", ty.trim(), arg));
                        j += 2;
                    }
                    // Odd leftover type without name
                    if j < parts.len() {
                        let ty = parts[j]
                            .trim()
                            .replace("__user", "")
                            .replace("__kernel", "")
                            .replace("__force", "");
                        params.push(format!("{} _a{j}", ty.trim()));
                    }
                    out.push_str(&format!(
                        "long {prefix}{sys_name}({}) ",
                        params.join(", ")
                    ));
                    continue;
                }
            }
            out.push_str(name);
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Soft-replace heavy GCC builtins left after kernel header expansion.
/// These explode into multi-KB expressions that thrash the parser on large TUs.
fn soften_kernel_builtins(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
            let soft = match name {
                // type compatibility → treat as true (1)
                "__builtin_types_compatible_p" => Some("1"),
                // Keep __builtin_constant_p for the parser: order_base_2 / ilog2 /
                // DEFINE("i"(…)) need the constant branch. Soft-0 forced
                // NR_CPUS_BITS=xzr in bounds.h and broke page-flag masks.
                // choose_expr(c,a,b) → a (first alternative); good enough for soft
                "__builtin_choose_expr" => Some("__ggcc_choose"),
                // expect(x,c) → (x)
                "__builtin_expect" => Some("__ggcc_expect"),
                // noreturn sink — soft to no-op expression (kernel heads use it)
                "__builtin_unreachable" => Some("((void)0)"),
                // Map freestanding builtins to libc/kernel helpers (compressed boot
                // has memcpy/memset; calling __builtin_* leaves undef at link).
                "__builtin_memcpy" => Some("memcpy"),
                "__builtin_memmove" => Some("memmove"),
                "__builtin_memset" => Some("memset"),
                "__builtin_memcmp" => Some("memcmp"),
                "__builtin_strlen" => Some("strlen"),
                // Soft: map bswap builtins to freestanding helpers (emitted by codegen)
                // so kernel crc32/etc. does not leave undef __builtin_bswap* at link.
                "__builtin_bswap16" => Some("__ggcc_bswap16"),
                "__builtin_bswap32" => Some("__ggcc_bswap32"),
                "__builtin_bswap64" => Some("__ggcc_bswap64"),
                // C11 _Generic is multi-assoc type switch; kernel uses it in
                // READ_ONCE-style helpers. Soft → 0 to avoid parse thrash.
                "_Generic" => Some("0"),
                // typeof(expr)/typeof(type) in kernel READ_ONCE/percpu — soft to long.
                "typeof" | "__typeof" | "__typeof__" => Some("__ggcc_typeof"),
                _ => None,
            };
            if let Some(rep) = soft {
                // skip whitespace + (args)
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'(' {
                    if rep == "__ggcc_choose" {
                        // keep first non-condition arg: choose_expr(c, a, b) → (a)
                        let args_start = i + 1;
                        let after = skip_balanced_parens(bytes, i);
                        let args_end = after.saturating_sub(1);
                        let args =
                            std::str::from_utf8(&bytes[args_start..args_end]).unwrap_or("");
                        let parts = split_top_level_commas(args);
                        let pick = if parts.len() >= 2 {
                            parts[1].trim()
                        } else if !parts.is_empty() {
                            parts[0].trim()
                        } else {
                            "0"
                        };
                        out.push('(');
                        out.push_str(pick);
                        out.push(')');
                        i = after;
                        continue;
                    }
                    if rep == "__ggcc_expect" {
                        // expect(x, c) → (x)
                        let args_start = i + 1;
                        let after = skip_balanced_parens(bytes, i);
                        let args_end = after.saturating_sub(1);
                        let args =
                            std::str::from_utf8(&bytes[args_start..args_end]).unwrap_or("");
                        let parts = split_top_level_commas(args);
                        let pick = parts.first().map(|s| s.trim()).unwrap_or("0");
                        out.push('(');
                        out.push_str(pick);
                        out.push(')');
                        i = after;
                        continue;
                    }
                    if rep == "__ggcc_typeof" {
                        // typeof(T) / typeof(expr) → long (size/align soft path)
                        i = skip_balanced_parens(bytes, i);
                        out.push_str("long");
                        continue;
                    }
                    // types_compatible_p / constant_p → literal
                    i = skip_balanced_parens(bytes, i);
                    out.push_str(rep);
                    continue;
                }
            }
            out.push_str(name);
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Dynamic special macros that depend on expansion site (not stored in the table).
fn is_dynamic_predef(name: &str) -> bool {
    matches!(name, "__LINE__" | "__FILE__")
}

fn macro_is_defined(name: &str, macros: &HashMap<String, MacroBody>) -> bool {
    is_dynamic_predef(name) || macros.contains_key(name)
}

/// Expand `__LINE__` / `__FILE__` at a use site. Returns None for other ids.
fn expand_dynamic_predef(id: &str, line: usize, file: &str) -> Option<String> {
    match id {
        "__LINE__" => Some(line.to_string()),
        "__FILE__" => {
            // Produce a C string literal with minimal escaping.
            let mut lit = String::with_capacity(file.len() + 2);
            lit.push('"');
            for ch in file.chars() {
                match ch {
                    '\\' | '"' => {
                        lit.push('\\');
                        lit.push(ch);
                    }
                    '\n' => lit.push_str("\\n"),
                    '\r' => lit.push_str("\\r"),
                    c if c.is_ascii_control() => {
                        lit.push_str(&format!("\\{:03o}", c as u32));
                    }
                    c => lit.push(c),
                }
            }
            lit.push('"');
            Some(lit)
        }
        _ => None,
    }
}

/// Shared-macro recursive preprocess so `#include` exports `#define`s to the parent.
fn preprocess_into(
    src: &str,
    include_dir: Option<&std::path::Path>,
    extra_includes: &[&std::path::Path],
    macros: &mut HashMap<String, MacroBody>,
    out: &mut String,
    emit_body: bool,
    source_name: &str,
) -> Result<(), String> {
    // Phase 2: backslash-newline line splicing
    let src = splice_backslash_newlines(src);
    // Phase 3-ish: strip block comments before directive/macro work.
    // Prevents SQLITE_OK-style `/* ... */` in expansions from breaking docs,
    // and avoids expanding macros inside comment text (huge speed win on sqlite3.c).
    let src_nc = strip_block_comments_preserve_newlines(&src);
    let lines: Vec<&str> = src_nc.lines().collect();
    let mut i = 0usize;
    let mut cond_stack: Vec<CondFrame> = Vec::new();

    while i < lines.len() {
        let raw = lines[i];
        i += 1;
        let line = strip_line_comment_keep_string(raw);
        let trimmed = line.trim();

        if trimmed.starts_with('#') {
            let dir = trimmed.trim_start_matches('#').trim_start();
            if dir.starts_with("include") {
                if !is_active(&cond_stack) {
                    continue;
                }
                let rest0 = dir["include".len()..].trim();
                // Support `#include MACRO` (Redis rax: `#include RAX_MALLOC_INCLUDE`
                // where MACRO expands to `"rax_malloc.h"`). Expand identifiers once.
                let rest_owned: String;
                let rest: &str = if rest0.starts_with('"') || rest0.starts_with('<') {
                    rest0
                } else {
                    // Object-like expand of the include token(s).
                    match expand_line(rest0, macros, i, source_name) {
                        Ok(e) => {
                            rest_owned = e.trim().to_string();
                            rest_owned.as_str()
                        }
                        Err(_) => rest0,
                    }
                };
                let mut found: Option<std::path::PathBuf> = None;
                if let Some(path) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    // "quote" include: absolute / CWD path first (gcc -include),
                    // then input dir, then -I paths.
                    let as_path = std::path::Path::new(path);
                    if as_path.is_file() {
                        found = Some(as_path.to_path_buf());
                    }
                    if found.is_none() {
                        if let Some(base) = include_dir {
                            let full = base.join(path);
                            if full.is_file() {
                                found = Some(full);
                            }
                        }
                    }
                    if found.is_none() {
                        for dir in extra_includes {
                            let full = dir.join(path);
                            if full.is_file() {
                                found = Some(full);
                                break;
                            }
                        }
                    }
                    if found.is_none() {
                        return Err(format!("#include \"{path}\" not found"));
                    }
                } else if let Some(path) = rest.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
                    // <angle> include: -I paths first, then input dir, then CWD
                    for dir in extra_includes {
                        let full = dir.join(path);
                        if full.is_file() {
                            found = Some(full);
                            break;
                        }
                    }
                    if found.is_none() {
                        if let Some(base) = include_dir {
                            let full = base.join(path);
                            if full.is_file() {
                                found = Some(full);
                            }
                        }
                    }
                    if found.is_none() {
                        let as_path = std::path::Path::new(path);
                        if as_path.is_file() {
                            found = Some(as_path.to_path_buf());
                        }
                    }
                    // Missing system headers: skip silently (legacy stub behavior)
                    // unless -I made it look like a project header under linux/.
                    if found.is_none() {
                        continue;
                    }
                } else {
                    // Still not quote/angle after expand — skip (unknown form).
                    continue;
                }
                if let Some(full) = found {
                    if let Ok(inc) = std::fs::read_to_string(&full) {
                        // Nested include shares macro table (critical for SQLITE_OK etc.)
                        let inc_name = full.to_string_lossy();
                        // Nested files search from their own directory too
                        let nested_dir = full.parent();
                        preprocess_into(
                            &inc,
                            nested_dir.or(include_dir),
                            extra_includes,
                            macros,
                            out,
                            emit_body,
                            &inc_name,
                        )?;
                        out.push('\n');
                    }
                }
                continue;
            }
            if dir.starts_with("define") {
                if !is_active(&cond_stack) {
                    continue;
                }
                let rest = dir["define".len()..].trim_start();
                let (name, body) = parse_define(rest)?;
                macros.insert(name, body);
                continue;
            }
            if dir.starts_with("undef") {
                if !is_active(&cond_stack) {
                    continue;
                }
                let name = dir["undef".len()..].trim();
                // Keep dynamic specials defined even if a translation unit #undefs them.
                if !is_dynamic_predef(name) {
                    macros.remove(name);
                }
                continue;
            }
            if dir.starts_with("ifdef") {
                let name = dir["ifdef".len()..].trim();
                let parent = is_active(&cond_stack);
                let cur = parent && macro_is_defined(name, macros);
                cond_stack.push(CondFrame {
                    parent_active: parent,
                    branch_taken: cur,
                    active: cur,
                });
                continue;
            }
            if dir.starts_with("ifndef") {
                let name = dir["ifndef".len()..].trim();
                let parent = is_active(&cond_stack);
                let cur = parent && !macro_is_defined(name, macros);
                cond_stack.push(CondFrame {
                    parent_active: parent,
                    branch_taken: cur,
                    active: cur,
                });
                continue;
            }
            if dir.starts_with("elif") {
                let frame = cond_stack
                    .last_mut()
                    .ok_or_else(|| "#elif without #if".to_string())?;
                if frame.branch_taken || !frame.parent_active {
                    frame.active = false;
                } else {
                    let expr = dir["elif".len()..].trim();
                    let v = eval_pp_expr(expr, macros, i, source_name)?;
                    frame.active = v != 0;
                    if frame.active {
                        frame.branch_taken = true;
                    }
                }
                continue;
            }
            if dir.starts_with("else") {
                let frame = cond_stack
                    .last_mut()
                    .ok_or_else(|| "#else without #if".to_string())?;
                if !frame.parent_active {
                    frame.active = false;
                } else {
                    frame.active = !frame.branch_taken;
                    if frame.active {
                        frame.branch_taken = true;
                    }
                }
                continue;
            }
            if dir.starts_with("endif") {
                cond_stack
                    .pop()
                    .ok_or_else(|| "#endif without #if".to_string())?;
                continue;
            }
            if dir.starts_with("if") {
                let parent = is_active(&cond_stack);
                let expr = dir["if".len()..].trim();
                let v = if parent {
                    eval_pp_expr(expr, macros, i, source_name)?
                } else {
                    0
                };
                let cur = parent && v != 0;
                cond_stack.push(CondFrame {
                    parent_active: parent,
                    branch_taken: cur,
                    active: cur,
                });
                continue;
            }
            // unknown directive: skip
            continue;
        }

        if !is_active(&cond_stack) {
            continue;
        }

        if !emit_body {
            continue;
        }

        // 1-based line of the start of this logical group (after the earlier i += 1).
        let line_no = i;

        // Join physical lines until macro-arg parentheses balance (C allows
        // multi-line invocations without backslash). Kernel `struct_group(...)`
        // spans `#ifdef` blocks; process those directives instead of stopping.
        let mut logical = trimmed.to_string();
        while paren_balance_outside_strings(&logical) > 0 && i < lines.len() {
            let next_raw = strip_line_comment_keep_string(lines[i]);
            let next = next_raw.trim().to_string();
            i += 1;
            if next.starts_with('#') {
                // Apply conditional directives so later members of the still-open
                // invocation are correctly included/excluded.
                let dir = next.trim_start_matches('#').trim_start();
                if dir.starts_with("ifdef") {
                    let name = dir["ifdef".len()..].trim();
                    let parent = is_active(&cond_stack);
                    let cur = parent && macro_is_defined(name, macros);
                    cond_stack.push(CondFrame {
                        parent_active: parent,
                        branch_taken: cur,
                        active: cur,
                    });
                } else if dir.starts_with("ifndef") {
                    let name = dir["ifndef".len()..].trim();
                    let parent = is_active(&cond_stack);
                    let cur = parent && !macro_is_defined(name, macros);
                    cond_stack.push(CondFrame {
                        parent_active: parent,
                        branch_taken: cur,
                        active: cur,
                    });
                } else if dir.starts_with("if")
                    && !dir.starts_with("ifdef")
                    && !dir.starts_with("ifndef")
                {
                    let expr = dir["if".len()..].trim();
                    let parent = is_active(&cond_stack);
                    let v = eval_pp_expr(expr, macros, line_no, source_name).unwrap_or(0);
                    let cur = parent && v != 0;
                    cond_stack.push(CondFrame {
                        parent_active: parent,
                        branch_taken: cur,
                        active: cur,
                    });
                } else if dir.starts_with("elif") {
                    if let Some(frame) = cond_stack.last_mut() {
                        if frame.branch_taken {
                            frame.active = false;
                        } else {
                            let expr = dir["elif".len()..].trim();
                            let v =
                                eval_pp_expr(expr, macros, line_no, source_name).unwrap_or(0);
                            let cur = frame.parent_active && v != 0;
                            frame.branch_taken = cur;
                            frame.active = cur;
                        }
                    }
                } else if dir.starts_with("else") {
                    if let Some(frame) = cond_stack.last_mut() {
                        let cur = frame.parent_active && !frame.branch_taken;
                        frame.branch_taken = frame.branch_taken || cur;
                        frame.active = cur;
                    }
                } else if dir.starts_with("endif") {
                    cond_stack.pop();
                }
                // Other directives mid-arg: ignore
                continue;
            }
            if !is_active(&cond_stack) {
                continue;
            }
            logical.push(' ');
            logical.push_str(&next);
        }

        let expanded = expand_line(&logical, macros, line_no, source_name)?;
        if !expanded.is_empty() {
            out.push_str(&expanded);
            out.push('\n');
        } else if logical.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&expanded);
            out.push('\n');
        }
    }
    Ok(())
}

/// Net open '(' count outside strings/chars (0 = balanced, >0 = need more lines).
fn paren_balance_outside_strings(s: &str) -> i32 {
    let mut bal = 0i32;
    let mut in_str = false;
    let mut in_char = false;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if !in_str && !in_char {
            if c == b'(' {
                bal += 1;
            } else if c == b')' {
                bal -= 1;
            } else if c == b'"' {
                in_str = true;
            } else if c == b'\'' {
                in_char = true;
            }
        } else if in_str {
            if c == b'\\' && i + 1 < b.len() {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
        } else if in_char {
            if c == b'\\' && i + 1 < b.len() {
                i += 2;
                continue;
            }
            if c == b'\'' {
                in_char = false;
            }
        }
        i += 1;
    }
    bal
}

struct CondFrame {
    parent_active: bool,
    branch_taken: bool,
    active: bool,
}

fn is_active(stack: &[CondFrame]) -> bool {
    stack.iter().all(|f| f.active) || stack.is_empty()
}

fn strip_line_comment_keep_string(s: &str) -> String {
    // very simple: strip // outside quotes
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut in_str = false;
    let mut in_char = false;
    while let Some(c) = chars.next() {
        if c == '"' && !in_char {
            in_str = !in_str;
            out.push(c);
            continue;
        }
        if c == '\'' && !in_str {
            in_char = !in_char;
            out.push(c);
            continue;
        }
        if !in_str && !in_char && c == '/' && chars.peek() == Some(&'/') {
            break;
        }
        out.push(c);
    }
    out
}

/// C phase 2: delete backslash immediately followed by optional horizontal whitespace and newline.
fn splice_backslash_newlines(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            let mut j = i + 1;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            if j < b.len() && b[j] == b'\n' {
                i = j + 1;
                continue;
            }
            if j + 1 < b.len() && b[j] == b'\r' && b[j + 1] == b'\n' {
                i = j + 2;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// Escape backslashes and quotes for C preprocessor # stringification (ISO C99 6.10.3.2).
fn stringify_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {},
            _ => out.push(c),
        }
    }
    out
}

/// Strip /* */ across the whole translation unit, keeping newlines so line
/// numbers stay roughly stable. Strings/chars preserved.
fn strip_block_comments_preserve_newlines(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    let mut in_char = false;
    while i < b.len() {
        let c = b[i];
        // C strings/chars cannot contain unescaped newlines — force-close so a
        // drifted quote state does not swallow `/* ... */` (kernel namei.c
        // `/////` inside block comments was line-commented away when an earlier
        // unclosed `"` made strip miss the block comment terminator).
        if c == b'\n' {
            in_str = false;
            in_char = false;
            out.push('\n');
            i += 1;
            continue;
        }
        if !in_str && !in_char && c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            if i + 1 < b.len() {
                i += 2;
            }
            out.push(' ');
            continue;
        }
        if c == b'"' && !in_char {
            in_str = !in_str;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b'\'' && !in_str {
            in_char = !in_char;
            out.push('\'');
            i += 1;
            continue;
        }
        if (in_str || in_char) && c == b'\\' && i + 1 < b.len() {
            out.push('\\');
            out.push(b[i + 1] as char);
            i += 2;
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Remove // and /* */ comments (not nested), preserving string/char literals.
fn strip_c_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    let mut in_char = false;
    while i < b.len() {
        let c = b[i] as char;
        if !in_str && !in_char && c == '/' && i + 1 < b.len() {
            if b[i + 1] == b'/' {
                break; // rest of line is //
            }
            if b[i + 1] == b'*' {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                if i + 1 < b.len() {
                    i += 2; // skip */
                }
                out.push(' ');
                continue;
            }
        }
        if c == '"' && !in_char {
            // handle escapes lightly
            in_str = !in_str;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '\'' && !in_str {
            in_char = !in_char;
            out.push(c);
            i += 1;
            continue;
        }
        if (in_str || in_char) && c == '\\' && i + 1 < b.len() {
            out.push(c);
            out.push(b[i + 1] as char);
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn parse_define(rest: &str) -> Result<(String, MacroBody), String> {
    let bytes = rest.as_bytes();
    if bytes.is_empty() {
        return Err("empty #define".into());
    }
    let mut i = 0;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    let name = rest[..i].to_string();
    if name.is_empty() {
        return Err("macro name missing".into());
    }
    // function-like only if '(' immediately after name (no space)
    if i < bytes.len() && bytes[i] == b'(' {
        i += 1;
        let mut params = Vec::new();
        let mut variadic = false;
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b')' {
                i += 1;
                break;
            }
            if i + 2 < bytes.len() && &rest[i..i + 3] == "..." {
                variadic = true;
                i += 3;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b')' {
                    i += 1;
                }
                break;
            }
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if start == i {
                return Err(format!("bad macro params in #define {name}"));
            }
            params.push(rest[start..i].to_string());
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b',' {
                i += 1;
                continue;
            }
            if i < bytes.len() && bytes[i] == b')' {
                i += 1;
                break;
            }
        }
        let body = strip_c_comments(rest[i..].trim()).trim().to_string();
        Ok((
            name,
            MacroBody::Function {
                params,
                body,
                variadic,
            },
        ))
    } else {
        // Strip trailing block comments so
        // `#define SQLITE_OK 0 /* Successful result */` → body "0"
        // (otherwise expanding inside a /* ... */ comment injects `*/` and
        // prematurely closes the outer comment — real-world sqlite3.h).
        let body = strip_c_comments(rest[i..].trim()).trim().to_string();
        Ok((name, MacroBody::Object(body)))
    }
}

fn eval_pp_expr(
    expr: &str,
    macros: &HashMap<String, MacroBody>,
    line_no: usize,
    source_name: &str,
) -> Result<i64, String> {
    let e = expr.trim();
    if e.is_empty() {
        return Ok(0);
    }
    // Rewrite defined(X) / defined X to 0/1 first
    let rewritten = rewrite_defined(e, macros);
    let expanded = expand_pp_tokens(&rewritten, macros, 0, line_no, source_name)?;
    eval_simple_int(&expanded)
}

fn rewrite_defined(s: &str, macros: &HashMap<String, MacroBody>) -> String {
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if i + 7 <= bytes.len() && &s[i..i + 7] == "defined" {
            let after = i + 7;
            let mut j = after;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let name = if j < bytes.len() && bytes[j] == b'(' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let start = j;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                let n = s[start..j].to_string();
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b')' {
                    j += 1;
                }
                i = j;
                n
            } else {
                let start = j;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                i = j;
                s[start..j].to_string()
            };
            out.push(if macro_is_defined(&name, macros) { '1' } else { '0' });
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn expand_pp_tokens(
    s: &str,
    macros: &HashMap<String, MacroBody>,
    depth: usize,
    line_no: usize,
    source_name: &str,
) -> Result<String, String> {
    expand_pp_tokens_disabled(s, macros, depth, line_no, source_name, &std::collections::HashSet::new())
}

fn expand_pp_tokens_disabled(
    s: &str,
    macros: &HashMap<String, MacroBody>,
    depth: usize,
    line_no: usize,
    source_name: &str,
    disabled: &std::collections::HashSet<String>,
) -> Result<String, String> {
    if depth > 64 {
        return Ok(s.to_string());
    }
    // For #if, only object-like expand of identifiers
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let id = &s[start..i];
            if id == "defined" {
                out.push_str(id);
                continue;
            }
            if let Some(dyn_exp) = expand_dynamic_predef(id, line_no, source_name) {
                // __FILE__ is a string; in #if integer context it becomes 0 via eval_simple_int.
                out.push_str(&dyn_exp);
                continue;
            }
            if disabled.contains(id) {
                out.push('0'); // painted macro in #if → treat as non-defined number path
                continue;
            }
            if let Some(m) = macros.get(id) {
                match m {
                    MacroBody::Object(body) => {
                        let mut d2 = disabled.clone();
                        d2.insert(id.to_string());
                        out.push_str(&expand_pp_tokens_disabled(
                            body,
                            macros,
                            depth + 1,
                            line_no,
                            source_name,
                            &d2,
                        )?);
                    }
                    MacroBody::Function {
                        params,
                        body,
                        variadic,
                    } => {
                        // Function-like in #if: L_INTHASBITS(SIZE_Bx) etc.
                        let mut j = i;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if j >= bytes.len() || bytes[j] != b'(' {
                            // not invoked — unknown id in #if → 0
                            out.push('0');
                            continue;
                        }
                        j += 1;
                        let (args, new_j) = parse_macro_args(s, j)?;
                        i = new_j;
                        let mut exp_args = Vec::with_capacity(args.len());
                        for a in &args {
                            exp_args.push(expand_pp_tokens_disabled(
                                a,
                                macros,
                                depth + 1,
                                line_no,
                                source_name,
                                disabled,
                            )?);
                        }
                        while exp_args.len() < params.len() {
                            exp_args.push(String::new());
                        }
                        let mut raw_args = args.clone();
                        while raw_args.len() < params.len() {
                            raw_args.push(String::new());
                        }
                        let replaced =
                            substitute_macro(params, *variadic, body, &raw_args, &exp_args)?;
                        let mut d2 = disabled.clone();
                        d2.insert(id.to_string());
                        out.push_str(&expand_pp_tokens_disabled(
                            &replaced,
                            macros,
                            depth + 1,
                            line_no,
                            source_name,
                            &d2,
                        )?);
                    }
                }
            } else {
                // unknown id in #if → 0
                out.push('0');
            }
        } else if bytes[i].is_ascii_digit() {
            // Full pp-number: decimal, 0x/0X hex, 0b/0B binary, optional U/L suffixes.
            // Critical: without this, `0x7fff0000` is tokenized as `0` + id `x7fff0000`
            // → expands to `00`, so `#if SQLITE_MAX_MMAP_SIZE>0` is false and mmap
            // support is stripped from SQLite (temptable2-10.2 residual).
            let start = i;
            i += 1;
            if i < bytes.len() && (bytes[i] == b'x' || bytes[i] == b'X') {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
            } else if i < bytes.len() && (bytes[i] == b'b' || bytes[i] == b'B') {
                i += 1;
                while i < bytes.len() && (bytes[i] == b'0' || bytes[i] == b'1') {
                    i += 1;
                }
            } else {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            while i < bytes.len() && matches!(bytes[i], b'u' | b'U' | b'l' | b'L') {
                i += 1;
            }
            out.push_str(&s[start..i]);
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

fn eval_simple_int(s: &str) -> Result<i64, String> {
    // recursive descent: || && ! + - * / ( ) numbers
    let t = s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    fn parse_or(chars: &[char], i: &mut usize) -> Result<i64, String> {
        let mut v = parse_and(chars, i)?;
        while *i + 1 < chars.len() && chars[*i] == '|' && chars[*i + 1] == '|' {
            *i += 2;
            let r = parse_and(chars, i)?;
            v = if v != 0 || r != 0 { 1 } else { 0 };
        }
        Ok(v)
    }
    fn parse_and(chars: &[char], i: &mut usize) -> Result<i64, String> {
        let mut v = parse_eq(chars, i)?;
        while *i + 1 < chars.len() && chars[*i] == '&' && chars[*i + 1] == '&' {
            *i += 2;
            let r = parse_eq(chars, i)?;
            v = if v != 0 && r != 0 { 1 } else { 0 };
        }
        Ok(v)
    }
    fn parse_eq(chars: &[char], i: &mut usize) -> Result<i64, String> {
        let mut v = parse_rel(chars, i)?;
        while *i < chars.len() {
            if *i + 1 < chars.len() && chars[*i] == '=' && chars[*i + 1] == '=' {
                *i += 2;
                let r = parse_rel(chars, i)?;
                v = if v == r { 1 } else { 0 };
            } else if *i + 1 < chars.len() && chars[*i] == '!' && chars[*i + 1] == '=' {
                *i += 2;
                let r = parse_rel(chars, i)?;
                v = if v != r { 1 } else { 0 };
            } else {
                break;
            }
        }
        Ok(v)
    }
    fn parse_rel(chars: &[char], i: &mut usize) -> Result<i64, String> {
        let mut v = parse_shift(chars, i)?;
        while *i < chars.len() {
            // Must check multi-char ops before single '<'/'>' so >> and << work.
            if *i + 1 < chars.len() && chars[*i] == '<' && chars[*i + 1] == '=' {
                *i += 2;
                let r = parse_shift(chars, i)?;
                v = if v <= r { 1 } else { 0 };
            } else if *i + 1 < chars.len() && chars[*i] == '>' && chars[*i + 1] == '=' {
                *i += 2;
                let r = parse_shift(chars, i)?;
                v = if v >= r { 1 } else { 0 };
            } else if chars[*i] == '<' {
                *i += 1;
                let r = parse_shift(chars, i)?;
                v = if v < r { 1 } else { 0 };
            } else if chars[*i] == '>' {
                *i += 1;
                let r = parse_shift(chars, i)?;
                v = if v > r { 1 } else { 0 };
            } else {
                break;
            }
        }
        Ok(v)
    }
    fn parse_shift(chars: &[char], i: &mut usize) -> Result<i64, String> {
        let mut v = parse_add(chars, i)?;
        while *i + 1 < chars.len() {
            if chars[*i] == '<' && chars[*i + 1] == '<' {
                *i += 2;
                let r = parse_add(chars, i)?;
                v = v.wrapping_shl(r as u32);
            } else if chars[*i] == '>' && chars[*i + 1] == '>' {
                *i += 2;
                let r = parse_add(chars, i)?;
                // C #if uses intmax_t arithmetic; treat as unsigned for large values
                // so UINT_MAX >> n works (Lua L_INTHASBITS).
                v = ((v as u64) >> (r as u32)) as i64;
            } else {
                break;
            }
        }
        Ok(v)
    }
    fn parse_add(chars: &[char], i: &mut usize) -> Result<i64, String> {
        let mut v = parse_term(chars, i)?;
        while *i < chars.len() {
            match chars[*i] {
                '+' => {
                    *i += 1;
                    v = v.wrapping_add(parse_term(chars, i)?);
                }
                '-' => {
                    *i += 1;
                    v = v.wrapping_sub(parse_term(chars, i)?);
                }
                _ => break,
            }
        }
        Ok(v)
    }
    fn parse_term(chars: &[char], i: &mut usize) -> Result<i64, String> {
        let mut v = parse_unary(chars, i)?;
        while *i < chars.len() {
            match chars[*i] {
                '*' => {
                    *i += 1;
                    v = v.wrapping_mul(parse_unary(chars, i)?);
                }
                '/' => {
                    *i += 1;
                    let r = parse_unary(chars, i)?;
                    if r != 0 {
                        v /= r;
                    }
                }
                _ => break,
            }
        }
        Ok(v)
    }
    fn parse_unary(chars: &[char], i: &mut usize) -> Result<i64, String> {
        if *i < chars.len() && chars[*i] == '!' {
            *i += 1;
            let v = parse_unary(chars, i)?;
            return Ok(if v == 0 { 1 } else { 0 });
        }
        if *i < chars.len() && chars[*i] == '~' {
            *i += 1;
            return Ok(!parse_unary(chars, i)?);
        }
        if *i < chars.len() && chars[*i] == '-' {
            *i += 1;
            return Ok(-parse_unary(chars, i)?);
        }
        if *i < chars.len() && chars[*i] == '+' {
            *i += 1;
            return parse_unary(chars, i);
        }
        if *i < chars.len() && chars[*i] == '(' {
            *i += 1;
            let v = parse_or(chars, i)?;
            if *i < chars.len() && chars[*i] == ')' {
                *i += 1;
            }
            return Ok(v);
        }
        let start = *i;
        if start >= chars.len() || !chars[start].is_ascii_digit() {
            return Ok(0);
        }
        *i += 1;
        let n = if *i < chars.len() && (chars[*i] == 'x' || chars[*i] == 'X') {
            // Hex: 0x...
            *i += 1;
            let hex_start = *i;
            while *i < chars.len() && chars[*i].is_ascii_hexdigit() {
                *i += 1;
            }
            if hex_start == *i {
                0i64
            } else {
                let hex: String = chars[hex_start..*i].iter().collect();
                u64::from_str_radix(&hex, 16).unwrap_or(0) as i64
            }
        } else if *i < chars.len() && (chars[*i] == 'b' || chars[*i] == 'B') {
            // Binary: 0b...
            *i += 1;
            let bin_start = *i;
            while *i < chars.len() && (chars[*i] == '0' || chars[*i] == '1') {
                *i += 1;
            }
            if bin_start == *i {
                0i64
            } else {
                let bin: String = chars[bin_start..*i].iter().collect();
                u64::from_str_radix(&bin, 2).unwrap_or(0) as i64
            }
        } else {
            while *i < chars.len() && chars[*i].is_ascii_digit() {
                *i += 1;
            }
            let num: String = chars[start..*i].iter().collect();
            // Parse as u64 first so 4294967295 fits, then cast.
            num.parse::<u64>().unwrap_or(0) as i64
        };
        // Strip integer suffixes U/L/UL/LL/ULL (case-insensitive).
        while *i < chars.len() && matches!(chars[*i], 'u' | 'U' | 'l' | 'L') {
            *i += 1;
        }
        Ok(n)
    }
    parse_or(&chars, &mut i)
}

fn expand_line(
    line: &str,
    macros: &HashMap<String, MacroBody>,
    line_no: usize,
    source_name: &str,
) -> Result<String, String> {
    let disabled = std::collections::HashSet::new();
    expand_macros_in_text(line, macros, 0, line_no, source_name, &disabled)
}

fn body_needs_reexpand(body: &str) -> bool {
    // If body has no identifier-like tokens, paste as-is (huge speed win).
    body.bytes().any(|c| c.is_ascii_alphabetic() || c == b'_')
}

fn expand_macros_in_text(
    text: &str,
    macros: &HashMap<String, MacroBody>,
    depth: usize,
    line_no: usize,
    source_name: &str,
    disabled: &std::collections::HashSet<String>,
) -> Result<String, String> {
    // Cap recursion: sqlite has deep macro chains; exponential blowup must stop early.
    if depth > 24 {
        return Ok(text.to_string());
    }
    // Guard against pathological expansion size (string-bomb macros).
    if text.len() > 4_000_000 {
        return Ok(text.to_string());
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_str = false;
    let mut in_char = false;
    while i < bytes.len() {
        let c = bytes[i];
        // Honor backslash escapes inside string/char so `\"` does not end the
        // literal. Without this, `#sym` → `"VM_EXEC"` after `"...\"->"` is
        // rescanned with VM_EXEC *outside* a string and re-expanded (kernel DEFINE).
        if (in_str || in_char) && c == b'\\' && i + 1 < bytes.len() {
            out.push('\\');
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if c == b'"' && !in_char {
            in_str = !in_str;
            out.push(c as char);
            i += 1;
            continue;
        }
        if c == b'\'' && !in_str {
            in_char = !in_char;
            out.push(c as char);
            i += 1;
            continue;
        }
        if !in_str && !in_char && (c.is_ascii_alphabetic() || c == b'_') {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let id = &text[start..i];
            // Dynamic predefined macros (__LINE__/__FILE__) — expand at use site.
            if let Some(dyn_exp) = expand_dynamic_predef(id, line_no, source_name) {
                out.push_str(&dyn_exp);
                continue;
            }
            // ISO C: a macro is not re-expanded while its own expansion is in progress
            // ("blue paint"). Critical for kernel `#define inline inline __attribute__(...)`.
            if disabled.contains(id) {
                out.push_str(id);
                continue;
            }
            if let Some(m) = macros.get(id) {
                match m {
                    MacroBody::Object(body) => {
                        if body.is_empty() {
                            // empty object macro → erase token
                        } else if body == id {
                            out.push_str(id);
                        } else {
                            let exp = if !body_needs_reexpand(body) {
                                body.clone()
                            } else {
                                let mut d2 = disabled.clone();
                                d2.insert(id.to_string());
                                expand_macros_in_text(
                                    body,
                                    macros,
                                    depth + 1,
                                    line_no,
                                    source_name,
                                    &d2,
                                )?
                            };
                            // Object → function-like glue: `#define setobj2n setobj`
                            // then `setobj2n(L,a,b)` must expand setobj as a call.
                            let exp_id = exp.trim();
                            let is_simple_id = !exp_id.is_empty()
                                && exp_id
                                    .bytes()
                                    .all(|c| c.is_ascii_alphanumeric() || c == b'_');
                            if is_simple_id {
                                if matches!(
                                    macros.get(exp_id),
                                    Some(MacroBody::Function { .. })
                                ) {
                                    let mut j = i;
                                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                                        j += 1;
                                    }
                                    if j < bytes.len() && bytes[j] == b'(' {
                                        let mut combined = exp_id.to_string();
                                        if needs_rescan_sep(&combined, &text[i..]) {
                                            combined.push(' ');
                                        }
                                        combined.push_str(&text[i..]);
                                        let full = expand_macros_in_text(
                                            &combined,
                                            macros,
                                            depth + 1,
                                            line_no,
                                            source_name,
                                            disabled,
                                        )?;
                                        out.push_str(&full);
                                        return Ok(out);
                                    }
                                }
                            }
                            out.push_str(&exp);
                        }
                    }
                    MacroBody::Function {
                        params,
                        body,
                        variadic,
                    } => {
                        // need '('
                        let mut j = i;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if j >= bytes.len() || bytes[j] != b'(' {
                            // not a call — leave name
                            out.push_str(id);
                            continue;
                        }
                        j += 1;
                        let (args, new_j) = parse_macro_args(text, j)?;
                        i = new_j;
                        if id == "__attribute__" || id == "__attribute" {
                            let is_packed = args.iter().any(|a| a.contains("packed"));
                            if is_packed {
                                out.push_str("__attribute__((packed))");
                            }
                            // Preserve weak for COND_SYSCALL (must lose to real SYSCALL_DEFINE).
                            let is_weak = args.iter().any(|a| a.contains("weak"));
                            if is_weak {
                                out.push_str("__ggcc_weak_attr ");
                            }
                            if let Some(sec_arg) = args.iter().find(|a| a.contains("section")) {
                                out.push_str(&format!("__attribute__(({}))", sec_arg));
                            }
                            continue;
                        }
                        // ISO C: expand each argument once, then substitute.
                        // `#param` / `##` use the *unexpanded* argument; bare `param`
                        // uses the expanded one. Kernel DEFINE(VM_EXEC, VM_EXEC) needs
                        // #sym → "VM_EXEC" while "i"(val) gets the expanded constant.
                        let mut exp_args = Vec::with_capacity(args.len());
                        for a in &args {
                            exp_args.push(expand_macros_in_text(
                                a,
                                macros,
                                depth + 1,
                                line_no,
                                source_name,
                                disabled,
                            )?);
                        }
                        // pad missing args
                        while exp_args.len() < params.len() {
                            exp_args.push(String::new());
                        }
                        let mut raw_args = args.clone();
                        while raw_args.len() < params.len() {
                            raw_args.push(String::new());
                        }
                        let replaced =
                            substitute_macro(params, *variadic, body, &raw_args, &exp_args)?;
                        // Rescan replacement with this macro disabled (no self-recursion / blue paint).
                        // Critical for kernel: `#define __alloc_size__(x,...) __attribute__((__alloc_size__(x,...)))`
                        // — the inner name must stay painted. NEVER re-scan the replacement glued to
                        // following text with paint lifted (that re-expanded __alloc_size__ ~50×).
                        let mut d2 = disabled.clone();
                        d2.insert(id.to_string());
                        let exp_repl = expand_macros_in_text(
                            &replaced,
                            macros,
                            depth + 1,
                            line_no,
                            source_name,
                            &d2,
                        )?;
                        // CAT(A,B)(x) → AB(x): only when replacement is a single identifier naming a
                        // function-like macro and the next token is '('. Glue that call under the
                        // *caller's* disabled set (not d2), so a different macro can still expand.
                        let exp_id = exp_repl.trim();
                        let is_simple_id = !exp_id.is_empty()
                            && exp_id
                                .bytes()
                                .all(|c| c.is_ascii_alphanumeric() || c == b'_');
                        if is_simple_id
                            && matches!(
                                macros.get(exp_id),
                                Some(MacroBody::Function { .. })
                            )
                        {
                            let mut j = i;
                            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                                j += 1;
                            }
                            if j < bytes.len() && bytes[j] == b'(' {
                                let mut glued = exp_id.to_string();
                                if needs_rescan_sep(&glued, &text[i..]) {
                                    glued.push(' ');
                                }
                                glued.push_str(&text[i..]);
                                let full = expand_macros_in_text(
                                    &glued,
                                    macros,
                                    depth + 1,
                                    line_no,
                                    source_name,
                                    disabled,
                                )?;
                                out.push_str(&full);
                                return Ok(out);
                            }
                        }
                        // Default: emit fully-expanded replacement (paint held), then keep scanning
                        // the following source with the original disabled set.
                        out.push_str(&exp_repl);
                        continue;
                    }
                }
            } else {
                out.push_str(id);
            }
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    Ok(out)
}

fn parse_macro_args(text: &str, mut i: usize) -> Result<(Vec<String>, usize), String> {
    let bytes = text.as_bytes();
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut in_char = false;
    if i < bytes.len() && bytes[i] == b')' {
        return Ok((args, i + 1));
    }
    while i < bytes.len() {
        let c = bytes[i] as char;
        // Escapes inside string/char so '\'' and "\"" don't end the literal early.
        if (in_str || in_char) && c == '\\' && i + 1 < bytes.len() {
            cur.push('\\');
            cur.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if c == '"' && !in_char {
            in_str = !in_str;
            cur.push(c);
            i += 1;
            continue;
        }
        if c == '\'' && !in_str {
            in_char = !in_char;
            cur.push(c);
            i += 1;
            continue;
        }
        if !in_str && !in_char {
            if c == '(' {
                depth += 1;
                cur.push(c);
                i += 1;
                continue;
            }
            if c == ')' {
                if depth == 0 {
                    args.push(cur.trim().to_string());
                    return Ok((args, i + 1));
                }
                depth -= 1;
                cur.push(c);
                i += 1;
                continue;
            }
            if c == ',' && depth == 0 {
                args.push(cur.trim().to_string());
                cur.clear();
                i += 1;
                continue;
            }
        }
        cur.push(c);
        i += 1;
    }
    // Soft recovery: kernel headers sometimes leave multi-line / nested
    // invocations that our line-joiner misses. Treat EOF as closing `)`.
    if !cur.is_empty() || !args.is_empty() {
        args.push(cur.trim().to_string());
        return Ok((args, i));
    }
    Err("unterminated macro args".into())
}

fn substitute_macro(
    params: &[String],
    variadic: bool,
    body: &str,
    raw_args: &[String],
    exp_args: &[String],
) -> Result<String, String> {
    // raw_map: for # and ## (unexpanded); exp_map: bare parameter uses.
    let mut raw_map: HashMap<String, String> = HashMap::new();
    let mut exp_map: HashMap<String, String> = HashMap::new();
    for (i, p) in params.iter().enumerate() {
        let r = raw_args.get(i).cloned().unwrap_or_default();
        let e = exp_args.get(i).cloned().unwrap_or_else(|| r.clone());
        raw_map.insert(p.clone(), r);
        exp_map.insert(p.clone(), e);
    }
    if variadic {
        let rest_raw = if raw_args.len() > params.len() {
            raw_args[params.len()..].join(", ")
        } else {
            String::new()
        };
        let rest_exp = if exp_args.len() > params.len() {
            exp_args[params.len()..].join(", ")
        } else {
            rest_raw.clone()
        };
        raw_map.insert("__VA_ARGS__".into(), rest_raw);
        exp_map.insert("__VA_ARGS__".into(), rest_exp);
    }
    let bytes = body.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    // After ##, if the RHS param expands empty, the next body token must not
    // glue to the LHS when re-tokenized (e.g. A ## B+ with B empty → "+ +" not "++").
    let mut after_paste = false;
    let mut paste_rhs_empty = false;
    while i < bytes.len() {
        // stringify #param — uses unexpanded argument
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] != b'#' {
            after_paste = false;
            paste_rhs_empty = false;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let id = &body[start..i];
            let v = raw_map.get(id).cloned().unwrap_or_else(|| id.to_string());
            let escaped_v = stringify_escape(&v);
            out.push('"');
            out.push_str(&escaped_v);
            out.push('"');
            continue;
        }
        // token paste a ## b — operands use unexpanded args
        if i + 1 < bytes.len() && bytes[i] == b'#' && bytes[i + 1] == b'#' {
            i += 2;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            // left already in out — trim trailing space
            while out.ends_with(' ') {
                out.pop();
            }
            after_paste = true;
            paste_rhs_empty = false;
            continue; // next ident concatenates without space
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let id = &body[start..i];
            let next_is_hashhash = {
                let mut k = i;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                k + 1 < bytes.len() && bytes[k] == b'#' && bytes[k + 1] == b'#'
            };
            if after_paste || next_is_hashhash {
                // ## LHS or RHS: use unexpanded argument
                if let Some(v) = raw_map.get(id) {
                    paste_rhs_empty = v.is_empty();
                    if !v.is_empty() {
                        out.push_str(v);
                        after_paste = false;
                        paste_rhs_empty = false;
                    } else {
                        after_paste = false;
                    }
                } else {
                    out.push_str(id);
                    after_paste = false;
                    paste_rhs_empty = false;
                }
            } else if let Some(v) = exp_map.get(id) {
                out.push_str(v);
            } else {
                out.push_str(id);
            }
        } else {
            let ch = bytes[i] as char;
            if paste_rhs_empty && needs_token_sep(out.chars().last(), ch) {
                out.push(' ');
            }
            paste_rhs_empty = false;
            after_paste = false;
            out.push(ch);
            i += 1;
        }
    }
    Ok(out)
}

/// When string-based ## leaves an empty RHS, prevent the next operator char
/// from merging with the left token (`+` + `+` → `++` instead of two Pluses).
fn needs_token_sep(prev: Option<char>, next: char) -> bool {
    let Some(p) = prev else {
        return false;
    };
    // operator / punct that can form multi-char tokens with the next char
    matches!(
        (p, next),
        ('+', '+')
            | ('+', '=')
            | ('-', '-')
            | ('-', '=')
            | ('-', '>')
            | ('|', '|')
            | ('|', '=')
            | ('&', '&')
            | ('&', '=')
            | ('=', '=')
            | ('!', '=')
            | ('<', '<')
            | ('<', '=')
            | ('>', '>')
            | ('>', '=')
            | ('*', '=')
            | ('/', '=')
            | ('%', '=')
            | ('^', '=')
    ) || (p.is_ascii_alphanumeric() || p == '_') && (next.is_ascii_alphanumeric() || next == '_')
}

/// Space needed between macro expansion and following source so tokens don't merge.
fn needs_rescan_sep(left: &str, right: &str) -> bool {
    let l = left.chars().rev().find(|c| !c.is_ascii_whitespace());
    let r = right.chars().find(|c| !c.is_ascii_whitespace());
    match (l, r) {
        (Some(a), Some(b)) => needs_token_sep(Some(a), b),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_hex_literal_comparison() {
        // 0x7fff0000 is SQLITE_MAX_MMAP_SIZE default — must not become 0 in #if.
        let s = "\
#define SQLITE_MAX_MMAP_SIZE 0x7fff0000\n\
#if SQLITE_MAX_MMAP_SIZE>0\n\
int mmap_on = 1;\n\
#else\n\
int mmap_on = 0;\n\
#endif\n\
#if 0x1 > 0\n\
int hex1 = 1;\n\
#else\n\
int hex1 = 0;\n\
#endif\n\
#if 0x7fff0000 == 2147418112\n\
int hex_eq = 1;\n\
#else\n\
int hex_eq = 0;\n\
#endif\n";
        let o = preprocess(s).unwrap();
        assert!(
            o.contains("mmap_on = 1") && !o.contains("mmap_on = 0"),
            "hex #if MAX_MMAP failed: {o}"
        );
        assert!(
            o.contains("hex1 = 1") && !o.contains("hex1 = 0"),
            "0x1>0 failed: {o}"
        );
        assert!(
            o.contains("hex_eq = 1") && !o.contains("hex_eq = 0"),
            "hex==decimal failed: {o}"
        );
    }

    #[test]
    fn define_object() {
        let s = "#define FOO 0\nint main(){return FOO;}\n";
        let o = preprocess(s).unwrap();
        assert!(o.contains("return 0"));
        assert!(!o.contains("FOO"));
    }

    #[test]
    fn define_function() {
        let s = "#define ADD(X, Y) (X + Y)\nint main(){return ADD(1, 2);}\n";
        let o = preprocess(s).unwrap();
        assert!(o.contains("(1 + 2)") || o.contains("(1+2)") || o.contains("1 + 2"));
    }

    #[test]
    fn token_paste_rescan_function_chain() {
        // CAT(A,B)(x) must become AB(x) then expand to xy (suite 00201).
        let s = "\
#define CAT2(a,b) a##b\n\
#define CAT(a,b) CAT2(a,b)\n\
#define AB(x) CAT(x,y)\n\
int main(void){int xy=42; return CAT(A,B)(x);}\n";
        let o = preprocess(s).unwrap();
        assert!(
            o.contains("return xy") || o.contains("return  xy"),
            "paste+rescan failed: {o}"
        );
        assert!(!o.contains("CAT("), "CAT left unexpanded: {o}");
    }

    #[test]
    fn define_stringify_keeps_unexpanded_name() {
        // Kernel kbuild: DEFINE(VM_EXEC, VM_EXEC) must emit symbol name VM_EXEC,
        // not the expanded 0x4, while the "i" operand still gets the constant.
        let s = "\
#define VM_EXEC 0x00000004\n\
#define DEFINE(sym, val) asm volatile(\"->\" #sym \" %0 \" #val : : \"i\" (val))\n\
void f(void) { DEFINE(VM_EXEC, VM_EXEC); }\n";
        let o = preprocess(s).unwrap();
        assert!(
            o.contains("\"VM_EXEC\"") || o.contains("->VM_EXEC"),
            "symbol name was expanded away: {o}"
        );
        assert!(
            !o.contains("->0x00000004") && !o.contains("->0x4"),
            "stringified name wrongly expanded: {o}"
        );
    }

    #[test]
    fn alloc_size_blue_paint_no_recursive_attr() {
        // Linux compiler_attributes.h:
        //   #define __alloc_size__(x, ...) __attribute__((__alloc_size__(x, ## __VA_ARGS__)))
        // Self-name in body must stay painted — otherwise ~50 nested __attribute__ layers.
        let s = "\
#define __alloc_size__(x, ...) __attribute__((__alloc_size__(x, ## __VA_ARGS__)))\n\
void *krealloc_array(void *p, int n, int sz) __alloc_size__(2, 3);\n";
        let o = preprocess(s).unwrap();
        let n_attr = o.matches("__attribute__").count();
        assert!(
            n_attr <= 2,
            "recursive __alloc_size__ expansion exploded attrs={n_attr}: {o}"
        );
        assert!(
            o.contains("krealloc_array"),
            "lost function name: {o}"
        );
        assert!(
            o.contains("__alloc_size__(2, 3)") || o.contains("__alloc_size__(2,3)"),
            "inner painted name missing: {o}"
        );
    }

    #[test]
    fn token_paste_empty_rhs_keeps_operator_tokens() {
        // Q(+,) with body A##B+ must not become ++ (suite 00202).
        let s = "#define Q(A,B) A ## B+\nint x = 60 Q(+,)3;\n";
        let o = preprocess(s).unwrap();
        assert!(
            o.contains("60 + +3") || o.contains("60 +  +3") || o.contains("60 + + 3"),
            "empty ## RHS glued operators: {o}"
        );
        assert!(!o.contains("60 ++"), "produced ++: {o}");
    }

    #[test]
    fn line_and_file_expand() {
        // Line 1: #define ...  Line 2: empty  Line 3: int x = __LINE__;
        let s = "#define BKPT err(__LINE__)\nint x = __LINE__;\nint y = BKPT;\nconst char *f = __FILE__;\n";
        let o = preprocess_with_options(s, None, &[], false, "unit.c").unwrap();
        assert!(
            o.contains("int x = 2;") || o.contains("int x = 2"),
            "direct __LINE__ not expanded: {o}"
        );
        // BKPT expands with invocation line (y = BKPT is line 3)
        assert!(
            o.contains("err(3)") || o.contains("err( 3 )"),
            "macro-nested __LINE__ not expanded: {o}"
        );
        assert!(
            o.contains("\"unit.c\""),
            "__FILE__ not expanded to unit.c: {o}"
        );
        assert!(
            !o.contains("__LINE__"),
            "raw __LINE__ leaked into output: {o}"
        );
        assert!(!o.contains("__FILE__"), "raw __FILE__ leaked into output: {o}");
    }

    #[test]
    fn line_defined_in_if() {
        let s = "#if defined(__LINE__)\nint ok;\n#endif\n#if !defined(__FILE__)\nint bad;\n#endif\n";
        let o = preprocess_with_options(s, None, &[], false, "t.c").unwrap();
        assert!(o.contains("int ok"), "defined(__LINE__) should be true: {o}");
        assert!(!o.contains("int bad"), "defined(__FILE__) should be true: {o}");
    }

    #[test]
    fn stringify_escaped_macro_arg() {
        let s = "#define S(X) #X\nchar *s = S(asm(\".section \\\"sec\\\"\"));\n";
        let o = preprocess(s).unwrap();
        assert!(o.contains("asm(\\\".section \\\\\\\"sec\\\\\\\"\\\")"), "quotes not escaped in stringification: {o}");
    }

    #[test]
    fn backslash_newline_with_trailing_whitespace() {
        let s = "int x \\\t \n= 42;\n";
        let o = splice_backslash_newlines(s);
        assert_eq!(o, "int x = 42;\n");
    }
}
