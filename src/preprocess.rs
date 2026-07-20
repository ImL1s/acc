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
    preprocess_with_options(src, include_dir, /*for_linux*/ false, "<input>")
}

/// `for_linux`: omit Darwin-only predefined macros so headers skip Apple blocks.
/// `source_name`: path/name used for `__FILE__` expansion at the primary translation unit.
pub fn preprocess_with_options(
    src: &str,
    include_dir: Option<&std::path::Path>,
    for_linux: bool,
    source_name: &str,
) -> Result<String, String> {
    let mut macros: HashMap<String, MacroBody> = HashMap::new();
    macros.insert("NULL".into(), MacroBody::Object("0".into()));
    // errno as a simple extern int (avoids needing typed function returns).
    macros.insert("errno".into(), MacroBody::Object("__ggcc_errno".into()));
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
    macros.insert("MS_ASYNC".into(), MacroBody::Object("1".into()));
    macros.insert("MS_INVALIDATE".into(), MacroBody::Object("2".into()));
    macros.insert("MS_SYNC".into(), MacroBody::Object("4".into()));
    macros.insert("MADV_NORMAL".into(), MacroBody::Object("0".into()));
    macros.insert("MADV_DONTNEED".into(), MacroBody::Object("4".into()));
    macros.insert("MADV_WILLNEED".into(), MacroBody::Object("3".into()));
    // signal / fcntl extras
    macros.insert("SIG_SETMASK".into(), MacroBody::Object("2".into()));
    macros.insert("SIG_BLOCK".into(), MacroBody::Object("0".into()));
    macros.insert("SIG_UNBLOCK".into(), MacroBody::Object("1".into()));
    macros.insert("FD_CLOEXEC".into(), MacroBody::Object("1".into()));
    macros.insert("F_GETFD".into(), MacroBody::Object("1".into()));
    macros.insert("F_SETFD".into(), MacroBody::Object("2".into()));
    macros.insert("F_FULLFSYNC".into(), MacroBody::Object("51".into()));
    macros.insert("F_BARRIERFSYNC".into(), MacroBody::Object("85".into()));
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
    macros.insert("S_IFLNK".into(), MacroBody::Object("40960".into()));
    macros.insert("S_IRUSR".into(), MacroBody::Object("256".into()));
    macros.insert("S_IWUSR".into(), MacroBody::Object("128".into()));
    macros.insert("S_IXUSR".into(), MacroBody::Object("64".into()));
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
    macros.insert("__aarch64__".into(), MacroBody::Object("1".into()));
    // ISO C required static predefined macros (__LINE__/__FILE__ are dynamic specials).
    macros.insert("__STDC__".into(), MacroBody::Object("1".into()));
    macros.insert("__STDC_HOSTED__".into(), MacroBody::Object("1".into()));
    macros.insert("__STDC_VERSION__".into(), MacroBody::Object("201112L".into()));
    if !for_linux {
        macros.insert("__APPLE__".into(), MacroBody::Object("1".into()));
        macros.insert("__MACH__".into(), MacroBody::Object("1".into()));
    } else {
        macros.insert("__linux__".into(), MacroBody::Object("1".into()));
        macros.insert("linux".into(), MacroBody::Object("1".into()));
        macros.insert("__linux".into(), MacroBody::Object("1".into()));
    }
    // stdarg stubs — enough for parse/codegen of va_arg(ap, T) forms
    macros.insert(
        "va_start".into(),
        MacroBody::Function {
            params: vec!["ap".into(), "last".into()],
            body: "((void)0)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "va_arg".into(),
        MacroBody::Function {
            params: vec!["ap".into(), "type".into()],
            body: "(*(type*)(0))".into(),
            variadic: false,
        },
    );
    macros.insert(
        "va_end".into(),
        MacroBody::Function {
            params: vec!["ap".into()],
            body: "((void)0)".into(),
            variadic: false,
        },
    );
    macros.insert(
        "va_copy".into(),
        MacroBody::Function {
            params: vec!["d".into(), "s".into()],
            body: "((void)0)".into(),
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
    // Stubs so public headers (sqlite3.h, etc.) can parse without full libc.
    let pthread_stubs = if for_linux {
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
        "typedef struct { char __s[64]; } pthread_mutex_t;\n\
         typedef struct { char __s[64]; } pthread_mutexattr_t;\n\
         typedef unsigned long pthread_t;\n\
         typedef struct { char __s[64]; } pthread_cond_t;\n\
         typedef struct { char __s[8]; } pthread_condattr_t;\n\
         typedef struct { char __s[8]; } pthread_once_t;\n\
         typedef int pthread_key_t;\n\
         typedef struct { char __s[64]; } pthread_attr_t;\n\
         typedef int pthread_rwlock_t;\n"
    } else {
        ""
    };
    let out_prefix = format!(
        "typedef int int32_t;\n\
         typedef long int64_t;\n\
         typedef short int16_t;\n\
         typedef long size_t;\n\
         typedef long ssize_t;\n\
         typedef unsigned long uintptr_t;\n\
         typedef long intptr_t;\n\
         typedef unsigned int uint32_t;\n\
         typedef unsigned short uint16_t;\n\
         typedef unsigned char uint8_t;\n\
         typedef signed char int8_t;\n\
         typedef unsigned long uint64_t;\n\
         typedef void *va_list;\n\
         extern int __ggcc_errno;\n\
         typedef long off_t;\n\
         typedef int pid_t;\n\
         typedef unsigned int uid_t;\n\
         typedef unsigned int gid_t;\n\
         typedef unsigned int mode_t;\n\
         typedef unsigned long dev_t;\n\
         typedef unsigned long ino_t;\n\
         typedef long blksize_t;\n\
         typedef long blkcnt_t;\n\
         typedef unsigned long nlink_t;\n\
         typedef long clock_t;\n\
         typedef int socklen_t;\n\
         typedef unsigned char uuid_t[16];\n\
         typedef unsigned long time_t;\n\
         struct timespec {{ long tv_sec; long tv_nsec; }};\n\
         struct timeval {{ long tv_sec; int tv_usec; }};\n\
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
         struct stat {{\n\
           dev_t st_dev; ino_t st_ino; mode_t st_mode; nlink_t st_nlink;\n\
           uid_t st_uid; gid_t st_gid; dev_t st_rdev; off_t st_size;\n\
           blksize_t st_blksize; blkcnt_t st_blocks;\n\
           long st_atime; long st_mtime; long st_ctime;\n\
         }};\n\
         {stdio_syms}\
         {pthread_stubs}"
    );
    let mut out = out_prefix;
    preprocess_into(src, include_dir, &mut macros, &mut out, true, source_name)?;
    Ok(out)
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
                let rest = dir["include".len()..].trim();
                if let Some(path) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    if let Some(base) = include_dir {
                        let full = base.join(path);
                        let inc = std::fs::read_to_string(&full).map_err(|e| {
                            format!("#include \"{path}\" read {}: {e}", full.display())
                        })?;
                        // Nested include shares macro table (critical for SQLITE_OK etc.)
                        let inc_name = full.to_string_lossy();
                        preprocess_into(
                            &inc,
                            include_dir,
                            macros,
                            out,
                            emit_body,
                            inc_name.as_ref(),
                        )?;
                        out.push('\n');
                    }
                }
                // <...> system headers ignored
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
        // multi-line invocations without backslash).
        let mut logical = trimmed.to_string();
        while paren_balance_outside_strings(&logical) > 0 && i < lines.len() {
            let next = strip_line_comment_keep_string(lines[i]).trim().to_string();
            i += 1;
            if next.starts_with('#') {
                // directive mid-invocation — stop joining
                i -= 1;
                break;
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

/// C phase 2: delete backslash immediately followed by newline.
fn splice_backslash_newlines(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() && b[i + 1] == b'\n' {
            i += 2;
            continue;
        }
        if b[i] == b'\\' && i + 2 < b.len() && b[i + 1] == b'\r' && b[i + 2] == b'\n' {
            i += 3;
            continue;
        }
        out.push(b[i] as char);
        i += 1;
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
            if let Some(MacroBody::Object(body)) = macros.get(id) {
                out.push_str(&expand_pp_tokens(body, macros, depth + 1, line_no, source_name)?);
            } else {
                // unknown id in #if → 0
                out.push('0');
            }
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
        let mut v = parse_add(chars, i)?;
        while *i < chars.len() {
            if *i + 1 < chars.len() && chars[*i] == '<' && chars[*i + 1] == '=' {
                *i += 2;
                let r = parse_add(chars, i)?;
                v = if v <= r { 1 } else { 0 };
            } else if *i + 1 < chars.len() && chars[*i] == '>' && chars[*i + 1] == '=' {
                *i += 2;
                let r = parse_add(chars, i)?;
                v = if v >= r { 1 } else { 0 };
            } else if chars[*i] == '<' {
                *i += 1;
                let r = parse_add(chars, i)?;
                v = if v < r { 1 } else { 0 };
            } else if chars[*i] == '>' {
                *i += 1;
                let r = parse_add(chars, i)?;
                v = if v > r { 1 } else { 0 };
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
                    v += parse_term(chars, i)?;
                }
                '-' => {
                    *i += 1;
                    v -= parse_term(chars, i)?;
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
                    v *= parse_unary(chars, i)?;
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
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
        if start == *i {
            return Ok(0);
        }
        let n: i64 = chars[start..*i].iter().collect::<String>().parse().unwrap_or(0);
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
    expand_macros_in_text(line, macros, 0, line_no, source_name)
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
            if let Some(m) = macros.get(id) {
                match m {
                    MacroBody::Object(body) => {
                        if body.is_empty() {
                            // empty object macro → erase token
                        } else if !body_needs_reexpand(body) {
                            out.push_str(body);
                        } else if body == id {
                            out.push_str(id);
                        } else {
                            let exp =
                                expand_macros_in_text(body, macros, depth + 1, line_no, source_name)?;
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
                        // Expand args before substitution unless # or ## (cheap scan once).
                        let body_has_hash = body.as_bytes().contains(&b'#');
                        let mut exp_args = Vec::with_capacity(args.len());
                        for a in &args {
                            if body_has_hash {
                                exp_args.push(a.clone());
                            } else {
                                exp_args.push(expand_macros_in_text(
                                    a,
                                    macros,
                                    depth + 1,
                                    line_no,
                                    source_name,
                                )?);
                            }
                        }
                        // pad missing args
                        while exp_args.len() < params.len() {
                            exp_args.push(String::new());
                        }
                        let replaced = substitute_macro(params, *variadic, body, &exp_args)?;
                        // Rescan substitution with following text so that
                        // CAT(A,B)(x) → AB(x) → expands AB as a function macro.
                        // (ISO C: macro replacement list is re-examined for more macros.)
                        // Insert a space when gluing would merge tokens (X()2 → 1 2, not 12).
                        let mut combined = replaced;
                        if needs_rescan_sep(&combined, &text[i..]) {
                            combined.push(' ');
                        }
                        combined.push_str(&text[i..]);
                        let exp = expand_macros_in_text(
                            &combined,
                            macros,
                            depth + 1,
                            line_no,
                            source_name,
                        )?;
                        out.push_str(&exp);
                        return Ok(out);
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
    Err("unterminated macro args".into())
}

fn substitute_macro(
    params: &[String],
    variadic: bool,
    body: &str,
    args: &[String],
) -> Result<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for (i, p) in params.iter().enumerate() {
        let a = args.get(i).cloned().unwrap_or_default();
        map.insert(p.clone(), a);
    }
    if variadic {
        let rest = if args.len() > params.len() {
            args[params.len()..].join(", ")
        } else {
            String::new()
        };
        map.insert("__VA_ARGS__".into(), rest);
    }
    let bytes = body.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    // After ##, if the RHS param expands empty, the next body token must not
    // glue to the LHS when re-tokenized (e.g. A ## B+ with B empty → "+ +" not "++").
    let mut after_paste = false;
    let mut paste_rhs_empty = false;
    while i < bytes.len() {
        // stringify #param
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
            let v = map.get(id).cloned().unwrap_or_else(|| id.to_string());
            out.push('"');
            out.push_str(&v);
            out.push('"');
            continue;
        }
        // token paste a ## b
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
            if let Some(v) = map.get(id) {
                if after_paste {
                    paste_rhs_empty = v.is_empty();
                    // empty RHS of ##: leave a marker so the following body token
                    // does not merge with the LHS when re-lexed as text.
                    if !v.is_empty() {
                        out.push_str(v);
                        after_paste = false;
                        paste_rhs_empty = false;
                    } else {
                        // keep after_paste; next non-param token may need a space
                        after_paste = false;
                        // paste_rhs_empty stays true
                    }
                } else {
                    out.push_str(v);
                }
            } else if after_paste {
                out.push_str(id);
                after_paste = false;
                paste_rhs_empty = false;
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
        let o = preprocess_with_options(s, None, false, "unit.c").unwrap();
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
        let o = preprocess_with_options(s, None, false, "t.c").unwrap();
        assert!(o.contains("int ok"), "defined(__LINE__) should be true: {o}");
        assert!(!o.contains("int bad"), "defined(__FILE__) should be true: {o}");
    }
}
