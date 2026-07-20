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
    preprocess_with_options(src, include_dir, /*for_linux*/ false)
}

/// `for_linux`: omit Darwin-only predefined macros so headers skip Apple blocks.
pub fn preprocess_with_options(
    src: &str,
    include_dir: Option<&std::path::Path>,
    for_linux: bool,
) -> Result<String, String> {
    let mut macros: HashMap<String, MacroBody> = HashMap::new();
    macros.insert("NULL".into(), MacroBody::Object("0".into()));
    // Predefined macros matching a 64-bit LP64 host
    macros.insert("__LP64__".into(), MacroBody::Object("1".into()));
    macros.insert("_LP64".into(), MacroBody::Object("1".into()));
    macros.insert("__aarch64__".into(), MacroBody::Object("1".into()));
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
        "typedef struct { char __s[64]; } pthread_mutex_t;\n\
         typedef struct { char __s[64]; } pthread_mutexattr_t;\n\
         typedef unsigned long pthread_t;\n\
         typedef struct { char __s[64]; } pthread_cond_t;\n\
         typedef struct { char __s[8]; } pthread_condattr_t;\n\
         typedef struct { char __s[8]; } pthread_once_t;\n\
         typedef int pthread_key_t;\n"
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
         typedef void *va_list;\n\
         typedef long off_t;\n\
         typedef int pid_t;\n\
         typedef unsigned long time_t;\n\
         {stdio_syms}\
         {pthread_stubs}"
    );
    let mut out = out_prefix;
    preprocess_into(src, include_dir, &mut macros, &mut out, true)?;
    Ok(out)
}

/// Shared-macro recursive preprocess so `#include` exports `#define`s to the parent.
fn preprocess_into(
    src: &str,
    include_dir: Option<&std::path::Path>,
    macros: &mut HashMap<String, MacroBody>,
    out: &mut String,
    emit_body: bool,
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
                        preprocess_into(&inc, include_dir, macros, out, emit_body)?;
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
                macros.remove(name);
                continue;
            }
            if dir.starts_with("ifdef") {
                let name = dir["ifdef".len()..].trim();
                let parent = is_active(&cond_stack);
                let cur = parent && macros.contains_key(name);
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
                let cur = parent && !macros.contains_key(name);
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
                    let v = eval_pp_expr(expr, &macros)?;
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
                    eval_pp_expr(expr, macros)?
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

        let expanded = expand_line(&logical, macros)?;
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

fn eval_pp_expr(expr: &str, macros: &HashMap<String, MacroBody>) -> Result<i64, String> {
    let e = expr.trim();
    if e.is_empty() {
        return Ok(0);
    }
    // Rewrite defined(X) / defined X to 0/1 first
    let rewritten = rewrite_defined(e, macros);
    let expanded = expand_pp_tokens(&rewritten, macros, 0)?;
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
            out.push(if macros.contains_key(&name) { '1' } else { '0' });
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn expand_pp_tokens(s: &str, macros: &HashMap<String, MacroBody>, depth: usize) -> Result<String, String> {
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
            if let Some(MacroBody::Object(body)) = macros.get(id) {
                out.push_str(&expand_pp_tokens(body, macros, depth + 1)?);
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

fn expand_line(line: &str, macros: &HashMap<String, MacroBody>) -> Result<String, String> {
    expand_macros_in_text(line, macros, 0)
}

fn body_needs_reexpand(body: &str) -> bool {
    // If body has no identifier-like tokens, paste as-is (huge speed win).
    body.bytes().any(|c| c.is_ascii_alphabetic() || c == b'_')
}

fn expand_macros_in_text(
    text: &str,
    macros: &HashMap<String, MacroBody>,
    depth: usize,
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
                            let exp = expand_macros_in_text(body, macros, depth + 1)?;
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
                                exp_args.push(expand_macros_in_text(a, macros, depth + 1)?);
                            }
                        }
                        // pad missing args
                        while exp_args.len() < params.len() {
                            exp_args.push(String::new());
                        }
                        let replaced = substitute_macro(params, *variadic, body, &exp_args)?;
                        if !body_needs_reexpand(&replaced) {
                            out.push_str(&replaced);
                        } else {
                            let exp = expand_macros_in_text(&replaced, macros, depth + 1)?;
                            out.push_str(&exp);
                        }
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
    while i < bytes.len() {
        // stringify #param
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] != b'#' {
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
                out.push_str(v);
            } else {
                out.push_str(id);
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
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
}
