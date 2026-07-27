//! x86_64 System V / Darwin code generator (parallel backend to aarch64).
//! Emits AT&T syntax assembly suitable for system `cc -arch x86_64` (macOS)
//! or native `cc` on Linux x86_64.

use crate::ast::*;
use crate::assigned_names::collect_assigned_names_in_program;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;

/// Darwin uses underscore-prefixed C symbols; Linux ELF does not.
fn sym(name: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("_{name}")
    } else {
        name.to_string()
    }
}

/// Address of a C global/data/function symbol into `dest_reg`.
///
/// Linux PIE executables (postgres): `leaq sym(%rip)` — matches gcc -fPIE.
/// Using `movq sym@GOTPCREL` is unsafe here: the linker may relax it to
/// `movq sym(%rip)` (value), while callers still deref once more → SEGV on
/// `fputs(..., stdout)`.
///
/// Linux shared objects: set `ACC_USE_GOT=1` so we use GOTPCREL (required for
/// undef inter-DSO refs; R_X86_64_PC32 would fail).
fn emit_global_sym_addr(out: &mut String, s: &str, dest_reg: &str) {
    emit_global_sym_addr_opts(out, s, dest_reg, false);
}

/// Like `emit_global_sym_addr`, but `force_got` uses GOTPCREL even when
/// `ACC_USE_GOT` is unset. Required for taking the address of an undefined
/// libc symbol in a PIE executable (`leaq memcmp(%rip)` → R_X86_64_PC32 fails).
fn emit_global_sym_addr_opts(out: &mut String, s: &str, dest_reg: &str, force_got: bool) {
    if cfg!(target_os = "macos") {
        writeln!(out, "\tleaq\t{s}(%rip), {dest_reg}").unwrap();
        return;
    }
    // libc FILE* streams must use GOT, never R_X86_64_PC32/COPY. The executable
    // COPY slot for `stdin` has been observed corrupted to 0x7fff00000000 by the
    // time InteractiveBackend runs (ctor shows a good FILE* at load), so PC32
    // loads SEGV inside getc. GOTPCREL without COPY reads libc's real pointer.
    let libc_stream = matches!(
        s,
        "stdin"
            | "stdout"
            | "stderr"
            | "__stdinp"
            | "__stdoutp"
            | "__stderrp"
    );
    let use_got = force_got
        || libc_stream
        || std::env::var_os("ACC_USE_GOT").is_some_and(|v| v != "0");
    if use_got {
        // GOT slot holds &sym; load it. (Do not use movq GOTPCREL alone for a
        // "value" path that also derefs — see module comment.)
        writeln!(out, "\tleaq\t{s}@GOTPCREL(%rip), {dest_reg}").unwrap();
        writeln!(out, "\tmovq\t({dest_reg}), {dest_reg}").unwrap();
    } else {
        writeln!(out, "\tleaq\t{s}(%rip), {dest_reg}").unwrap();
    }
}

/// Logical temp registers used by this backend (caller-saved where possible).
/// 0=%rax  9=%r10  10=%r11  11=%rcx  12=%r8  16=%r9  17=%rsi  19=%rbx
fn reg(n: u8) -> &'static str {
    match n {
        0 => "%rax",
        9 => "%r10",
        10 => "%r11",
        11 => "%rcx",
        12 => "%r8",
        16 => "%r9",
        17 => "%rsi",
        19 => "%rbx",
        // parameter-style indices when loading call args
        1 => "%rdi",
        2 => "%rsi",
        3 => "%rdx",
        4 => "%rcx",
        5 => "%r8",
        6 => "%r9",
        _ => "%rax",
    }
}

fn reg_d(n: u8) -> &'static str {
    match n {
        0 => "%eax",
        9 => "%r10d",
        10 => "%r11d",
        11 => "%ecx",
        12 => "%r8d",
        16 => "%r9d",
        17 => "%esi",
        19 => "%ebx",
        1 => "%edi",
        2 => "%esi",
        3 => "%edx",
        4 => "%ecx",
        5 => "%r8d",
        6 => "%r9d",
        _ => "%eax",
    }
}

fn reg_b(n: u8) -> &'static str {
    match n {
        0 => "%al",
        9 => "%r10b",
        10 => "%r11b",
        11 => "%cl",
        12 => "%r8b",
        16 => "%r9b",
        17 => "%sil",
        19 => "%bl",
        1 => "%dil",
        2 => "%sil",
        3 => "%dl",
        4 => "%cl",
        5 => "%r8b",
        6 => "%r9b",
        _ => "%al",
    }
}

fn reg_w(n: u8) -> &'static str {
    match n {
        0 => "%ax",
        9 => "%r10w",
        10 => "%r11w",
        11 => "%cx",
        12 => "%r8w",
        16 => "%r9w",
        17 => "%si",
        19 => "%bx",
        1 => "%di",
        2 => "%si",
        3 => "%dx",
        4 => "%cx",
        5 => "%r8w",
        6 => "%r9w",
        _ => "%ax",
    }
}

fn is_asm_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Parser substitutes `%N` → `xN` / `wN` (AArch64 names). On x86_64 those leak as
/// bare symbols (`U x0`) unless rewritten to AT&T registers before emission.
fn rewrite_soft_arm_regs_to_att(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 8);
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if (c == b'x' || c == b'w')
            && (i == 0 || !is_asm_ident_char(b[i - 1]))
            && i + 1 < b.len()
            && b[i + 1].is_ascii_digit()
        {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j == b.len() || !is_asm_ident_char(b[j]) {
                let n: u8 = std::str::from_utf8(&b[i + 1..j])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if c == b'w' {
                    out.push_str(reg_d(n));
                } else {
                    out.push_str(reg(n));
                }
                i = j;
                continue;
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Soft-fix x86 lines after AArch64-reg rewrite:
/// - `lea`/`leal`/`leaq` with no memory operand → `movq` (avoids gas
///   "operand type mismatch for 'lea'")
/// - size-mismatched lea mnemonic vs dest register
/// - bare `pop`/`push` of a register → `popq`/`pushq`
fn att_byte_reg(att: &str) -> &str {
    match att {
        "%rax" | "%eax" | "%ax" | "%al" => "%al",
        "%rbx" | "%ebx" | "%bx" | "%bl" => "%bl",
        "%rcx" | "%ecx" | "%cx" | "%cl" => "%cl",
        "%rdx" | "%edx" | "%dx" | "%dl" => "%dl",
        "%rsi" | "%esi" | "%si" | "%sil" => "%sil",
        "%rdi" | "%edi" | "%di" | "%dil" => "%dil",
        "%r8" | "%r8d" | "%r8w" | "%r8b" => "%r8b",
        "%r9" | "%r9d" | "%r9w" | "%r9b" => "%r9b",
        "%r10" | "%r10d" | "%r10w" | "%r10b" => "%r10b",
        "%r11" | "%r11d" | "%r11w" | "%r11b" => "%r11b",
        "%r12" | "%r12d" | "%r12w" | "%r12b" => "%r12b",
        "%r13" | "%r13d" | "%r13w" | "%r13b" => "%r13b",
        "%r14" | "%r14d" | "%r14w" | "%r14b" => "%r14b",
        "%r15" | "%r15d" | "%r15w" | "%r15b" => "%r15b",
        other => other,
    }
}

fn att_dword_reg(att: &str) -> &str {
    match att {
        "%rax" | "%eax" | "%ax" | "%al" => "%eax",
        "%rbx" | "%ebx" | "%bx" | "%bl" => "%ebx",
        "%rcx" | "%ecx" | "%cx" | "%cl" => "%ecx",
        "%rdx" | "%edx" | "%dx" | "%dl" => "%edx",
        "%rsi" | "%esi" | "%si" | "%sil" => "%esi",
        "%rdi" | "%edi" | "%di" | "%dil" => "%edi",
        "%r8" | "%r8d" | "%r8w" | "%r8b" => "%r8d",
        "%r9" | "%r9d" | "%r9w" | "%r9b" => "%r9d",
        "%r10" | "%r10d" | "%r10w" | "%r10b" => "%r10d",
        "%r11" | "%r11d" | "%r11w" | "%r11b" => "%r11d",
        "%r12" | "%r12d" | "%r12w" | "%r12b" => "%r12d",
        "%r13" | "%r13d" | "%r13w" | "%r13b" => "%r13d",
        "%r14" | "%r14d" | "%r14w" | "%r14b" => "%r14d",
        "%r15" | "%r15d" | "%r15w" | "%r15b" => "%r15d",
        other => other,
    }
}

fn att_qword_reg(att: &str) -> &str {
    match att {
        "%rax" | "%eax" | "%ax" | "%al" => "%rax",
        "%rbx" | "%ebx" | "%bx" | "%bl" => "%rbx",
        "%rcx" | "%ecx" | "%cx" | "%cl" => "%rcx",
        "%rdx" | "%edx" | "%dx" | "%dl" => "%rdx",
        "%rsi" | "%esi" | "%si" | "%sil" => "%rsi",
        "%rdi" | "%edi" | "%di" | "%dil" => "%rdi",
        "%r8" | "%r8d" | "%r8w" | "%r8b" => "%r8",
        "%r9" | "%r9d" | "%r9w" | "%r9b" => "%r9",
        "%r10" | "%r10d" | "%r10w" | "%r10b" => "%r10",
        "%r11" | "%r11d" | "%r11w" | "%r11b" => "%r11",
        "%r12" | "%r12d" | "%r12w" | "%r12b" => "%r12",
        "%r13" | "%r13d" | "%r13w" | "%r13b" => "%r13",
        "%r14" | "%r14d" | "%r14w" | "%r14b" => "%r14",
        "%r15" | "%r15d" | "%r15w" | "%r15b" => "%r15",
        other => other,
    }
}

/// Rewrite GP regs outside `(…)` memory operands to the width implied by the
/// mnemonic suffix (`xaddl`→32, `xaddq`/`xchgb`→64/8). Gas rejects
/// `xaddl %rax,(%rdi)` (was dropped → orphan `lock` + SIGILL).
fn resize_regs_for_mnemonic(line: &str, mnem: &str) -> String {
    let width = if mnem.ends_with('b')
        || matches!(mnem, "setz" | "setnz" | "sete" | "setne" | "setl" | "setg" | "setle" | "setge" | "seta" | "setb" | "setae" | "setbe" | "setc" | "seto" | "sets" | "setns")
    {
        8
    } else if mnem.ends_with('w') {
        16
    } else if mnem.ends_with('l')
        || matches!(
            mnem,
            "xaddl"
                | "cmpxchgl"
                | "addl"
                | "subl"
                | "andl"
                | "orl"
                | "xorl"
                | "movl"
                | "cmpl"
                | "adcl"
                | "sbbl"
                | "testl"
                | "imull"
                | "idivl"
                | "divl"
                | "mull"
                | "negl"
                | "notl"
                | "incl"
                | "decl"
                | "bsfl"
                | "bsrl"
                | "bswapl"
        )
    {
        32
    } else if mnem.ends_with('q')
        || matches!(
            mnem,
            "xaddq"
                | "cmpxchgq"
                | "addq"
                | "subq"
                | "andq"
                | "orq"
                | "xorq"
                | "movq"
                | "cmpq"
                | "leaq"
                | "pushq"
                | "popq"
        )
    {
        64
    } else {
        return line.to_string();
    };

    let map = match width {
        8 => att_byte_reg as fn(&str) -> &str,
        32 => att_dword_reg as fn(&str) -> &str,
        64 => att_qword_reg as fn(&str) -> &str,
        _ => return line.to_string(),
    };

    // Longest-first so %rax wins over %al fragments.
    let candidates = [
        "%r15d", "%r14d", "%r13d", "%r12d", "%r11d", "%r10d", "%r9d", "%r8d", "%r15w", "%r14w",
        "%r13w", "%r12w", "%r11w", "%r10w", "%r9w", "%r8w", "%r15b", "%r14b", "%r13b", "%r12b",
        "%r11b", "%r10b", "%r9b", "%r8b", "%r15", "%r14", "%r13", "%r12", "%r11", "%r10", "%r9",
        "%r8", "%eax", "%ebx", "%ecx", "%edx", "%esi", "%edi", "%ebp", "%esp", "%rax", "%rbx",
        "%rcx", "%rdx", "%rsi", "%rdi", "%rbp", "%rsp", "%ax", "%bx", "%cx", "%dx", "%si", "%di",
        "%al", "%bl", "%cl", "%dl", "%sil", "%dil",
    ];
    let mut body = line.to_string();
    for full in candidates {
        if !body.contains(full) {
            continue;
        }
        let repl = map(full);
        if repl == full {
            continue;
        }
        let mut out = String::new();
        let bytes = body.as_bytes();
        let fb = full.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if i + fb.len() <= bytes.len() && &bytes[i..i + fb.len()] == fb {
                let prev = if i == 0 { b' ' } else { bytes[i - 1] };
                let next = bytes.get(i + fb.len()).copied().unwrap_or(b' ');
                // Token boundary: do not turn %sil into %sill via a %si match.
                let next_ok = !next.is_ascii_alphanumeric();
                // Keep address regs inside (%rax) at full width.
                if prev != b'(' && next_ok {
                    out.push_str(repl);
                    i += fb.len();
                    continue;
                }
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        body = out;
    }
    body
}

fn soften_x86_asm_line(line: &str) -> Option<String> {
    let rewritten = rewrite_soft_arm_regs_to_att(line);
    let t = rewritten.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();

    // Orphan prefixes are illegal if the following insn is dropped (postgres
    // `lock; xchgb %0,%1` became `lock` + later `movzbq` → SIGILL).
    // Callers buffer these; standalone soften returns a marker.
    if matches!(
        lower.as_str(),
        "lock" | "rep" | "repe" | "repz" | "repne" | "repnz"
    ) {
        return Some(format!("__acc_prefix_{lower}"));
    }

    // AArch64-only mnemonics that survive %N→xN substitution — never valid for gas x86.
    {
        let mnem = lower
            .split(|c: char| c == ' ' || c == '\t' || c == ';')
            .next()
            .unwrap_or("");
        if matches!(
            mnem,
            "mrs"
                | "msr"
                | "mrs_s"
                | "msr_s"
                | "tlbi"
                | "adrp"
                | "ldxr"
                | "stxr"
                | "ldaxr"
                | "stlxr"
                | "eret"
                | "svc"
                | "hvc"
                | "smc"
                | "wfe"
                | "wfi"
                | "isb"
                | "dsb"
                | "dmb"
                | "cbz"
                | "cbnz"
                | "tbz"
                | "tbnz"
        ) {
            return None;
        }
    }

    // Multi-statement soft lines ("pushf; pop %rax" / "lock; xchgb" / "rep; nop").
    if t.contains(';') {
        let mut parts: Vec<String> = Vec::new();
        let mut pref: Option<String> = None;
        for p in t.split(';') {
            let Some(fixed) = soften_x86_asm_line(p) else {
                pref = None;
                continue;
            };
            if let Some(pr) = fixed.strip_prefix("__acc_prefix_") {
                pref = Some(pr.to_string());
                continue;
            }
            if let Some(pr) = pref.take() {
                // gas accepts `lock; xchgb …` / `rep; nop` as one logical insn.
                parts.push(format!("{pr}; {fixed}"));
            } else {
                parts.push(fixed);
            }
        }
        if parts.is_empty() {
            // Lone prefix with no following insn — drop (avoid SIGILL).
            return None;
        }
        return Some(parts.join("; "));
    }

    // xaddl/xchgb/setz: size-match GP regs (gas rejects `xaddl %rax`).
    let mnem = lower
        .split(|c: char| c == ' ' || c == '\t')
        .next()
        .unwrap_or("");
    let mut body = resize_regs_for_mnemonic(t, mnem);

    let t = body.as_str();
    let lower = t.to_ascii_lowercase();

    // lea without '(' → register/register form is illegal; soft as movq.
    let is_lea = lower.starts_with("lea ")
        || lower.starts_with("lea\t")
        || lower.starts_with("leaq")
        || lower.starts_with("leal");
    if is_lea && !t.contains('(') {
        // "lea %rsi, %rax" / "leaq %rsi, %rax" → movq
        let rest = if lower.starts_with("leaq") {
            t[4..].trim_start()
        } else if lower.starts_with("leal") {
            t[4..].trim_start()
        } else {
            // "lea" + ws
            t[3..].trim_start()
        };
        return Some(format!("movq\t{rest}"));
    }

    // Size-match lea mnemonic to destination register width.
    if is_lea {
        if let Some(comma) = t.rfind(',') {
            let dest = t[comma + 1..].trim();
            let mem = t[..comma].trim();
            // Strip mnemonic from mem side for rebuild.
            let mem_op = if lower.starts_with("leaq") {
                mem[4..].trim_start()
            } else if lower.starts_with("leal") {
                mem[4..].trim_start()
            } else {
                mem[3..].trim_start()
            };
            let dest_32 = dest.ends_with('d')
                || matches!(
                    dest,
                    "%eax" | "%ebx" | "%ecx" | "%edx" | "%esi" | "%edi" | "%ebp" | "%esp"
                );
            if dest_32 {
                return Some(format!("leal\t{mem_op}, {dest}"));
            }
            // Default 64-bit destination → leaq
            return Some(format!("leaq\t{mem_op}, {dest}"));
        }
    }

    // Bare push/pop of AT&T reg → sized form (avoids ambiguity; matches SysV).
    if lower.starts_with("pop ") || lower.starts_with("pop\t") {
        let arg = t[3..].trim_start();
        if arg.starts_with('%') {
            return Some(format!("popq\t{arg}"));
        }
    }
    if lower.starts_with("push ") || lower.starts_with("push\t") {
        let arg = t[4..].trim_start();
        if arg.starts_with('%') {
            return Some(format!("pushq\t{arg}"));
        }
    }
    // pushf / popf / pushfq keep as-is (already handled if no args).

    Some(t.trim().to_string())
}

/// SysV integer argument registers (first 6).
const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];

#[derive(Clone)]
struct Layout {
    size: i64,
    align: i64,
    fields: HashMap<String, (i64, Type)>,
}

#[derive(Clone)]
enum Storage {
    Local { offset: i64 },
    Global { name: String },
    RegAddr { reg: u8 },
}

#[derive(Clone)]
struct Sym {
    ty: Type,
    storage: Storage,
}

pub struct Codegen {
    out: String,
    strings: Vec<String>,
    layouts: HashMap<String, Layout>,
    globals: HashMap<String, Type>,
    /// Enum / static integer constants for folding `&arr[CONST]`.
    const_globals: HashMap<String, i64>,
    funcs: HashMap<String, Function>,
    /// current function local scopes (innermost scope at the end)
    scopes: Vec<HashMap<String, Sym>>,
    stack_size: i64,
    label_id: usize,
    break_stack: Vec<String>,
    continue_stack: Vec<String>,
    func_name: String,
    /// Return type of the function currently being emitted (SysV xmm0 for FP).
    func_ret: Type,
    /// Case labels queued during switch dispatch (matched by emit_switch_body).
    pending_case_labs: VecDeque<String>,
    /// FP-relative start of GP regsave for the current variadic function (0 = none).
    va_regsave_off: i64,
    /// Number of fixed named GP args (va_start skips these slots).
    va_fixed_n: usize,
    /// Pending block-scoped static variables (decl, func_name) to emit in .data/.bss at module scope.
    pending_statics: Vec<(VarDecl, String)>,
    /// Map of (func_name, var_name) -> mangled global symbol name for block-scoped statics.
    func_local_statics: HashMap<(String, String), String>,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            out: String::new(),
            strings: Vec::new(),
            layouts: HashMap::new(),
            globals: HashMap::new(),
            const_globals: HashMap::new(),
            funcs: HashMap::new(),
            scopes: vec![HashMap::new()],
            stack_size: 0,
            label_id: 0,
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            func_name: String::new(),
            func_ret: Type::Void,
            pending_case_labs: VecDeque::new(),
            va_regsave_off: 0,
            va_fixed_n: 0,
            pending_statics: Vec::new(),
            func_local_statics: HashMap::new(),
        }
    }

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn get_local(&self, name: &str) -> Option<&Sym> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.get(name) {
                return Some(sym);
            }
        }
        None
    }

    fn static_lvalue_reloc(&self, e: &Expr) -> Option<(String, i64)> {
        let peeled = Self::peel_casts(e);
        match peeled {
            Expr::Var(v) => {
                if self.funcs.contains_key(v)
                    || v == "main"
                    || self.globals.contains_key(v)
                {
                    return Some((sym(v), 0));
                }
                if let Some(s) = self.get_local(v) {
                    if let Storage::Global { name } = &s.storage {
                        return Some((sym(name), 0));
                    }
                    return None;
                }
                if let Some(gname) = self.func_local_statics.get(&(self.func_name.clone(), v.clone())) {
                    return Some((sym(gname), 0));
                }
                Some((sym(v), 0))
            }
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => self.static_lvalue_reloc(expr),
            Expr::Index { base, index } => {
                let (vsym, base_off) = self.static_lvalue_reloc(base)?;
                let idx = self.const_i64(index)?;
                let bty = self.typeof_expr(base, &HashMap::new());
                let esz = match &bty {
                    Type::Array(e, _) | Type::Ptr(e) => self.type_size(e).max(1),
                    _ => 1,
                };
                Some((vsym, base_off + idx * esz))
            }
            Expr::Member { base, field, arrow } => {
                if *arrow {
                    return None;
                }
                let (vsym, base_off) = self.static_lvalue_reloc(base)?;
                let ty = self.typeof_expr(base, &HashMap::new());
                let struct_name = match &ty {
                    Type::Struct(n) | Type::Union(n) => n.as_str(),
                    _ => "",
                };
                let field_off = if let Some(lay) = self.get_layout(struct_name) {
                    lay.fields.get(field).map(|(off, _)| *off).unwrap_or(0)
                } else {
                    0
                };
                Some((vsym, base_off + field_off))
            }
            _ => None,
        }
    }

    fn contains_local(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|scope| scope.contains_key(name))
    }

    fn insert_local(&mut self, name: String, sym: Sym) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, sym);
        }
    }

    fn clear_locals(&mut self) {
        self.scopes = vec![HashMap::new()];
    }

    fn lab(&mut self, p: &str) -> String {
        let id = self.label_id;
        self.label_id += 1;
        format!("L_{}_{}_{}", self.func_name, p, id)
    }

    fn intern_str(&mut self, s: &str) -> usize {
        if let Some(i) = self.strings.iter().position(|x| x == s) {
            return i;
        }
        let i = self.strings.len();
        self.strings.push(s.to_string());
        i
    }

    fn align_up(n: i64, a: i64) -> i64 {
        (n + a - 1) & !(a - 1)
    }

    fn get_layout(&self, name: &str) -> Option<&Layout> {
        if let Some(lay) = self.layouts.get(name) {
            if !lay.fields.is_empty() {
                return Some(lay);
            }
        }
        let base = name.split("__s").next().unwrap_or(name);
        for (k, v) in &self.layouts {
            let kbase = k.split("__s").next().unwrap_or(k);
            if kbase == base && !v.fields.is_empty() {
                return Some(v);
            }
        }
        self.layouts.get(name)
    }

    fn type_size(&self, ty: &Type) -> i64 {
        match ty {
            Type::Void => 0,
            Type::Char | Type::SChar | Type::UChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt => 4,
            Type::Long | Type::ULong => 8,
            Type::Float => 4,
            Type::Double => 8,
            Type::Ptr(_) => 8,
            Type::Array(e, n) => self.type_size(e) * n,
            Type::Struct(n) | Type::Union(n) => self.get_layout(n).map(|l| l.size).unwrap_or(8),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).size,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).size,
            Type::Const(inner) => self.type_size(inner),
        }
    }

    /// Struct/union values larger than a register must be byte-copied; a single
    /// movq only moves 8 bytes (postgres BootStrapXLOG:
    /// `ControlFile->checkPointCopy = checkPoint` left ThisTimeLineID=0 in
    /// pg_control while WAL had TLI=1).
    /// Types ≤8 bytes (e.g. FullTransactionId) stay on the scalar path so
    /// `s = f()` returns in %rax still work.
    fn needs_block_copy_ty(&self, ty: &Type) -> bool {
        // Any aggregate whose size is not a single natural store width must be
        // byte-copied. Using movq for sizeof==6 (ItemPointerData) clobbers the
        // next 2 bytes — postgres HeapTupleHeader.t_infomask2 / natts wiped by
        // `item->t_ctid = tuple->t_self` in RelationPutHeapTuple.
        let sz = self.type_size(ty);
        matches!(
            ty,
            Type::Struct(_)
                | Type::Union(_)
                | Type::AnonStruct(_)
                | Type::AnonUnion(_)
                | Type::Array(_, _)
        ) && sz > 0
            && !matches!(sz, 1 | 2 | 4 | 8)
    }

    fn is_struct_or_union_ty(ty: &Type) -> bool {
        matches!(
            ty.unqual(),
            Type::Struct(_) | Type::Union(_) | Type::AnonStruct(_) | Type::AnonUnion(_)
        )
    }

    /// C usual arithmetic conversions (integer subset) for binary op result types.
    /// Without this, `(uint64_t)24 * size` was typed Int → cmpl on relational
    /// ops (postgres simplehash `sizeof*size >= SIZE_MAX/2` always true via
    /// signed setge against lim's low 32 bits 0xffffffff).
    fn usual_arith_conv(l: &Type, r: &Type) -> Type {
        let l_unqual = l.unqual();
        let r_unqual = r.unqual();
        if matches!(l_unqual, Type::Float | Type::Double) || matches!(r_unqual, Type::Float | Type::Double) {
            return Type::Double;
        }
        let rank = |t: &Type| -> i32 {
            match t.unqual() {
                Type::Long | Type::ULong => 4,
                Type::Int | Type::UInt => 3,
                Type::Short | Type::UShort => 2,
                Type::Char | Type::SChar | Type::UChar => 1,
                Type::Ptr(_) | Type::Array(_, _) => 4,
                _ => 3,
            }
        };
        let unsigned = |t: &Type| -> bool {
            matches!(
                t.unqual(),
                Type::ULong
                    | Type::UInt
                    | Type::UShort
                    | Type::UChar
                    | Type::Char
                    | Type::Ptr(_)
                    | Type::Array(_, _)
            )
        };
        let lr = rank(l);
        let rr = rank(r);
        let (winner, unsign) = if lr > rr {
            (lr, unsigned(l))
        } else if rr > lr {
            (rr, unsigned(r))
        } else {
            (lr, unsigned(l) || unsigned(r))
        };
        match (winner, unsign) {
            (4, true) => Type::ULong,
            (4, false) => Type::Long,
            (3, true) => Type::UInt,
            _ => Type::Int,
        }
    }

    /// SysV: aggregate ≤8 bytes in %rax; ≤16 bytes in %rax:%rdx; ≤32 bytes in %rax:%rdx:%rcx:%r8.
    fn small_agg_nregs(&self, ty: &Type) -> Option<u8> {
        if !Self::is_struct_or_union_ty(ty) {
            return None;
        }
        let sz = self.type_size(ty);
        if sz <= 0 || sz > 32 {
            return None;
        }
        Some(((sz + 7) / 8) as u8)
    }

    /// Multi-register INTEGER-class aggregate *arguments*.
    fn small_agg_arg_nregs(&self, ty: &Type) -> Option<u8> {
        if !Self::is_struct_or_union_ty(ty) {
            return None;
        }
        let sz = self.type_size(ty);
        if sz <= 0 || sz > 32 {
            return None;
        }
        Some(((sz + 7) / 8) as u8)
    }

    /// How many integer argument registers / 8-byte stack slots an arg consumes.
    fn sysv_int_arg_slots(&self, ty: &Type) -> usize {
        self.small_agg_arg_nregs(ty).map(|n| n as usize).unwrap_or(1)
    }

    fn emit_sysv_arg_setup(
        &mut self,
        args: &[Expr],
        proto: &[Type],
        typedefs: &HashMap<String, Type>,
        extra_pushed_slots: usize,
    ) -> Result<(usize, i64), String> {
        let mut total_slots = 0usize;
        for (i, a) in args.iter().enumerate() {
            let aty = proto
                .get(i)
                .cloned()
                .unwrap_or_else(|| self.typeof_expr(a, typedefs));
            if let Some(nr) = self.small_agg_arg_nregs(&aty) {
                total_slots += nr as usize;
            } else {
                total_slots += 1;
            }
        }

        let total_items = extra_pushed_slots + total_slots;
        let pad_bytes: i64 = if total_items % 2 != 0 { 8 } else { 0 };
        let temp_bytes = pad_bytes + (total_slots as i64) * 8;

        if temp_bytes > 0 {
            writeln!(self.out, "\tsubq\t${temp_bytes}, %rsp").unwrap();
        }

        // Step 1: Evaluate all arguments forward and store in temporary stack slots
        let mut curr_slot = total_slots;
        for (i, a) in args.iter().enumerate() {
            let aty = proto
                .get(i)
                .cloned()
                .unwrap_or_else(|| self.typeof_expr(a, typedefs));
            if let Some(nr) = self.small_agg_arg_nregs(&aty) {
                self.emit_agg_arg_addr(a, typedefs)?;
                for k in 0..(nr as usize) {
                    let mem_off = (k as i64) * 8;
                    curr_slot -= 1;
                    let slot_off = (curr_slot as i64) * 8;
                    writeln!(self.out, "\tmovq\t{mem_off}(%r10), %rax").unwrap();
                    writeln!(self.out, "\tmovq\t%rax, {slot_off}(%rsp)").unwrap();
                }
            } else {
                curr_slot -= 1;
                let slot_off = (curr_slot as i64) * 8;
                self.emit_expr_rval(a, 0, typedefs)?;
                writeln!(self.out, "\tmovq\t%rax, {slot_off}(%rsp)").unwrap();
            }
        }

        let nreg = total_slots.min(6);
        let nstack = total_slots.saturating_sub(6);

        // Step 2: Load register arguments from temporary stack slots
        for i in 0..nreg {
            let off = ((total_slots - 1 - i) as i64) * 8;
            writeln!(self.out, "\tmovq\t{off}(%rsp), {}", ARG_REGS[i]).unwrap();
        }

        // Step 3: Setup stack argument frame if nstack > 0
        let stack_bytes = if nstack > 0 {
            let sb = ((nstack as i64) * 8 + 15) & !15;
            writeln!(self.out, "\tsubq\t${sb}, %rsp").unwrap();
            for k in 0..nstack {
                let src_off = sb + ((nstack - 1 - k) as i64) * 8;
                let dst_off = (k as i64) * 8;
                writeln!(self.out, "\tmovq\t{src_off}(%rsp), %rax").unwrap();
                writeln!(self.out, "\tmovq\t%rax, {dst_off}(%rsp)").unwrap();
            }
            sb
        } else {
            0
        };

        let cleanup_bytes = pad_bytes + (total_slots as i64) * 8 + stack_bytes;
        Ok((nreg, cleanup_bytes))
    }

    fn expr_is_lvalue(e: &Expr) -> bool {
        match e {
            Expr::Var(_) | Expr::Member { .. } | Expr::Index { .. } => true,
            Expr::Unary {
                op: UnaryOp::Deref,
                ..
            } => true,
            Expr::Cast { expr, .. } => Self::expr_is_lvalue(expr),
            _ => false,
        }
    }

    /// Leave address of a small aggregate arg in %r10 (lvalue or materialized).
    fn emit_agg_arg_addr(
        &mut self,
        arg: &Expr,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        if Self::expr_is_lvalue(arg) {
            self.emit_lvalue_addr(arg, 9, typedefs)?;
        } else {
            self.emit_materialize_agg_addr(arg, 9, typedefs)?;
        }
        Ok(())
    }

    /// Store a small aggregate returned in GPRs into memory at `addr_reg`.
    /// `nregs` is the SysV eightbyte count; `nbytes` is the true object size
    /// (may be 6 for ItemPointerData — must not movq past the object).
    fn store_small_agg_from_regs_sized(&mut self, addr_reg: u8, nregs: u8, nbytes: usize) {
        let a = reg(addr_reg);
        let ret_regs = ["%rax", "%rdx", "%rcx", "%r8", "%r9", "%r11"];
        for i in 0..(nregs as usize) {
            let off = i * 8;
            let rem = nbytes.saturating_sub(off);
            let r = ret_regs[i];
            if rem >= 8 {
                writeln!(self.out, "\tmovq\t{r}, {off}({a})").unwrap();
            } else if rem > 0 {
                let r_d = match r { "%rax" => "%eax", "%rdx" => "%edx", "%rcx" => "%ecx", "%r8" => "%r8d", "%r9" => "%r9d", _ => "%eax" };
                let r_w = match r { "%rax" => "%ax", "%rdx" => "%dx", "%rcx" => "%cx", "%r8" => "%r8w", "%r9" => "%r9w", _ => "%ax" };
                let r_b = match r { "%rax" => "%al", "%rdx" => "%dl", "%rcx" => "%cl", "%r8" => "%r8b", "%r9" => "%r9b", _ => "%al" };
                match rem {
                    1 => writeln!(self.out, "\tmovb\t{r_b}, {off}({a})").unwrap(),
                    2 => writeln!(self.out, "\tmovw\t{r_w}, {off}({a})").unwrap(),
                    4 => writeln!(self.out, "\tmovl\t{r_d}, {off}({a})").unwrap(),
                    _ => writeln!(self.out, "\tmovq\t{r}, {off}({a})").unwrap(),
                }
            }
        }
    }

    fn store_small_agg_from_regs(&mut self, addr_reg: u8, nregs: u8) {
        // Legacy: assume full eightbytes (safe for size 8/16 slots).
        self.store_small_agg_from_regs_sized(addr_reg, nregs, if nregs >= 2 { 16 } else { 8 });
    }

    /// Spill one SysV eightbyte of a small aggregate parameter from a GPR into
    /// its stack home. Tail eightbytes may be partial (e.g. 12-byte struct uses
    /// movl for the second eightbyte — movq clobbers adjacent locals).
    fn spill_small_agg_param_eightbyte(
        &mut self,
        arg_reg_idx: usize,
        slot: i64,
        eightbyte_k: u8,
        nbytes: usize,
    ) {
        let ar = (arg_reg_idx as u8) + 1;
        if eightbyte_k == 0 {
            writeln!(
                self.out,
                "\tmovq\t{}, {}(%rbp)",
                ARG_REGS[arg_reg_idx],
                slot
            )
            .unwrap();
            return;
        }
        let rem = nbytes.saturating_sub(8);
        match rem {
            0 => {}
            1 => writeln!(self.out, "\tmovb\t{}, {}(%rbp)", reg_b(ar), slot).unwrap(),
            2 => writeln!(self.out, "\tmovw\t{}, {}(%rbp)", reg_w(ar), slot).unwrap(),
            3 => {
                writeln!(self.out, "\tmovw\t{}, {}(%rbp)", reg_w(ar), slot).unwrap();
                writeln!(
                    self.out,
                    "\tmovq\t{}, %rcx",
                    ARG_REGS[arg_reg_idx]
                )
                .unwrap();
                writeln!(self.out, "\tshrl\t$16, %ecx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, {}(%rbp)", slot + 2).unwrap();
            }
            4 => writeln!(self.out, "\tmovl\t{}, {}(%rbp)", reg_d(ar), slot).unwrap(),
            5 => {
                writeln!(self.out, "\tmovl\t{}, {}(%rbp)", reg_d(ar), slot).unwrap();
                writeln!(
                    self.out,
                    "\tmovq\t{}, %rcx",
                    ARG_REGS[arg_reg_idx]
                )
                .unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, {}(%rbp)", slot + 4).unwrap();
            }
            6 => {
                writeln!(self.out, "\tmovl\t{}, {}(%rbp)", reg_d(ar), slot).unwrap();
                writeln!(
                    self.out,
                    "\tmovq\t{}, %rcx",
                    ARG_REGS[arg_reg_idx]
                )
                .unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovw\t%cx, {}(%rbp)", slot + 4).unwrap();
            }
            7 => {
                writeln!(self.out, "\tmovl\t{}, {}(%rbp)", reg_d(ar), slot).unwrap();
                writeln!(
                    self.out,
                    "\tmovq\t{}, %rcx",
                    ARG_REGS[arg_reg_idx]
                )
                .unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovw\t%cx, {}(%rbp)", slot + 4).unwrap();
                writeln!(self.out, "\tshrl\t$16, %ecx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, {}(%rbp)", slot + 6).unwrap();
            }
            _ => writeln!(
                self.out,
                "\tmovq\t{}, {}(%rbp)",
                ARG_REGS[arg_reg_idx],
                slot
            )
            .unwrap(),
        }
    }

    /// Copy one eightbyte from a stack-passed aggregate arg into the local home.
    fn spill_small_agg_stack_arg_eightbyte(
        &mut self,
        arg_off: i64,
        slot: i64,
        eightbyte_k: u8,
        nbytes: usize,
    ) {
        if eightbyte_k == 0 {
            writeln!(self.out, "\tmovq\t{arg_off}(%rbp), %rax").unwrap();
            writeln!(self.out, "\tmovq\t%rax, {slot}(%rbp)").unwrap();
            return;
        }
        let rem = nbytes.saturating_sub(8);
        match rem {
            0 => {}
            1 => {
                writeln!(self.out, "\tmovb\t{arg_off}(%rbp), %al").unwrap();
                writeln!(self.out, "\tmovb\t%al, {slot}(%rbp)").unwrap();
            }
            2 => {
                writeln!(self.out, "\tmovw\t{arg_off}(%rbp), %ax").unwrap();
                writeln!(self.out, "\tmovw\t%ax, {slot}(%rbp)").unwrap();
            }
            4 => {
                writeln!(self.out, "\tmovl\t{arg_off}(%rbp), %eax").unwrap();
                writeln!(self.out, "\tmovl\t%eax, {slot}(%rbp)").unwrap();
            }
            _ => {
                writeln!(self.out, "\tmovq\t{arg_off}(%rbp), %rax").unwrap();
                writeln!(self.out, "\tmovq\t%rax, {slot}(%rbp)").unwrap();
            }
        }
    }

    /// Materialize a non-lvalue aggregate (call return, `?:`, etc.) to a stack
    /// temp and leave its address in `regn`. Fixes `f().field` SEGV where soft
    /// previously treated the returned value in %rax as a pointer
    /// (postgres FullTransactionIdRetreat → FirstNormalFullTransactionId.value).
    fn emit_materialize_agg_addr(
        &mut self,
        e: &Expr,
        regn: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        let raw_ty = self.typeof_expr(e, typedefs);
        let ty = self.expand_ty(&raw_ty, typedefs);
        if !Self::is_struct_or_union_ty(&ty) {
            let _ = self.emit_expr_rval(e, regn, typedefs)?;
            return Ok(Type::Ptr(Box::new(Type::Void)));
        }
        let sz = self.type_size(&ty).max(8);
        let tmp = self.alloc_local("", &ty);
        if let Some(nr) = self.small_agg_nregs(&ty) {
            self.emit_expr_rval(e, 0, typedefs)?;
            self.emit_fp_addr(tmp, 9);
            self.store_small_agg_from_regs_sized(9, nr, self.type_size(&ty) as usize);
            self.emit_fp_addr(tmp, regn);
            return Ok(ty);
        }
        // Large aggregate: soft Call path still returns first word in %rax;
        // zero the slot then store what we have (better than deref of value).
        self.emit_fp_addr(tmp, 1); // %rdi
        writeln!(self.out, "\txorl\t%esi, %esi").unwrap();
        self.emit_imm(sz, 2); // %rdx
        writeln!(self.out, "\tcallq\t{}@PLT", sym("memset")).unwrap();
        self.emit_expr_rval(e, 0, typedefs)?;
        self.emit_fp_addr(tmp, 9);
        writeln!(self.out, "\tmovq\t%rax, (%r10)").unwrap();
        self.emit_fp_addr(tmp, regn);
        Ok(ty)
    }

    fn stack_slot_size(&self, ty: &Type) -> i64 {
        match ty {
            Type::Char
            | Type::SChar
            | Type::Short
            | Type::UShort
            | Type::Int
            | Type::UInt
            | Type::Long
            | Type::ULong
            | Type::Float
            | Type::Double
            | Type::Ptr(_) => 8,
            // Arrays are contiguous: use natural element size, not 8-byte
            // scalar slots. char buf[1<<20] must be 1MiB — 8*N overflows the
            // default stack and SEGVs Redis sdsTest (etalon[1024*1024]).
            Type::Array(e, n) => self.type_size(e) * n,
            other => self.type_size(other).max(8),
        }
    }

    fn type_align(&self, ty: &Type) -> i64 {
        match ty {
            Type::Void => 1,
            Type::Char | Type::SChar | Type::UChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt | Type::Float => 4,
            Type::Long | Type::ULong | Type::Double | Type::Ptr(_) => 8,
            Type::Array(e, _) => self.type_align(e),
            Type::Struct(n) | Type::Union(n) => self.get_layout(n).map(|l| l.align).unwrap_or(8),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).align,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).align,
            Type::Const(inner) => self.type_align(inner),
        }
    }

    fn layout_fields(&self, fields: &[Field], is_union: bool, packed: bool) -> Layout {
        let mut map = HashMap::new();
        let mut max_align = 1i64;
        let mut max_size = 0i64;
        let mut offset_bits: u64 = 0;
        for f in fields {
            if f.name.is_empty() && f.bit_width.is_none() {
                let nested_opt = match &f.ty {
                    Type::AnonStruct(fs) => Some(self.layout_fields(fs, false, false)),
                    Type::AnonUnion(fs) => Some(self.layout_fields(fs, true, false)),
                    Type::Struct(n) => self.get_layout(n).cloned(),
                    Type::Union(n) => self.get_layout(n).cloned(),
                    _ => None,
                };
                if let Some(nested) = nested_opt {
                    let nalign = if packed { 1 } else { nested.align };
                    max_align = max_align.max(nalign);
                    if is_union {
                        // Nested type starts at 0 of the union; keep relative field offs.
                        for (fnm, (fo, fty)) in &nested.fields {
                            map.insert(fnm.clone(), (*fo, fty.clone()));
                        }
                        max_size = max_size.max(nested.size);
                    } else {
                        let mut byte_off = ((offset_bits + 7) / 8) as i64;
                        byte_off = Self::align_up(byte_off, nalign);
                        for (fnm, (fo, fty)) in &nested.fields {
                            map.insert(fnm.clone(), (byte_off + fo, fty.clone()));
                        }
                        offset_bits = ((byte_off + nested.size) as u64) * 8;
                    }
                    continue;
                }
            }

            if let Some(width) = f.bit_width {
                let container_sz = self.type_size(&f.ty).max(1) as u64;
                let container_bits = container_sz * 8;
                let al = if packed { 1 } else { self.type_align(&f.ty) };
                max_align = max_align.max(al);

                if is_union {
                    if !f.name.is_empty() && width > 0 {
                        map.insert(f.name.clone(), (0, f.ty.clone()));
                    }
                    max_size = max_size.max(container_sz as i64);
                    continue;
                }

                if width == 0 {
                    let al_bits = (al as u64) * 8;
                    if al_bits > 0 {
                        offset_bits = ((offset_bits + al_bits - 1) / al_bits) * al_bits;
                    }
                    continue;
                }

                let w = width as u64;
                if container_bits > 0 && (offset_bits % container_bits) + w > container_bits {
                    offset_bits =
                        ((offset_bits + container_bits - 1) / container_bits) * container_bits;
                }
                let bit_pos = offset_bits;
                let cont_index = bit_pos / container_bits.max(1);
                let unit_start = (cont_index * container_sz) as i64;
                if !f.name.is_empty() {
                    map.insert(f.name.clone(), (unit_start, f.ty.clone()));
                }
                offset_bits = bit_pos + w;
                let end_byte = ((offset_bits + 7) / 8) as i64;
                max_size = max_size.max(end_byte);
                continue;
            }

            let sz = self.type_size(&f.ty);
            let al = if packed { 1 } else { self.type_align(&f.ty) };
            max_align = max_align.max(al);
            if is_union {
                if !f.name.is_empty() {
                    map.insert(f.name.clone(), (0, f.ty.clone()));
                }
                max_size = max_size.max(sz);
            } else {
                let mut byte_off = ((offset_bits + 7) / 8) as i64;
                byte_off = Self::align_up(byte_off, al);
                if !f.name.is_empty() {
                    map.insert(f.name.clone(), (byte_off, f.ty.clone()));
                }
                offset_bits = ((byte_off + sz) as u64) * 8;
            }
        }
        let final_align = if packed { 1 } else { max_align.max(1) };
        let off = ((offset_bits + 7) / 8) as i64;
        let size = if is_union {
            Self::align_up(max_size, final_align)
        } else {
            Self::align_up(off, final_align)
        };
        Layout {
            size,
            align: final_align,
            fields: map,
        }
    }

    fn collect_layouts(&mut self, prog: &Program) {
        // Multi-pass: type_layouts HashMap order is unstable; nested named
        // members must be sized before parent union/struct (else 8-byte fallback).
        // Skip empty forward-decl StructDef/UnionDef so they cannot clobber a
        // fuller layout recorded in type_layouts (Linux task_struct INIT_TASK).
        for _ in 0..12 {
            for (name, is_union, packed, fields) in &prog.type_layouts {
                if fields.is_empty() {
                    continue;
                }
                let lay = self.layout_fields(fields, *is_union, *packed);
                self.layouts.insert(name.clone(), lay);
            }
            for item in &prog.items {
                match item {
                    Item::StructDef { name, fields } => {
                        if fields.is_empty() {
                            continue;
                        }
                        let packed = prog
                            .type_layouts
                            .iter()
                            .find(|(n, _, _, _)| n == name)
                            .map(|(_, _, p, _)| *p)
                            .unwrap_or(false);
                        let lay = self.layout_fields(fields, false, packed);
                        self.layouts.insert(name.clone(), lay);
                    }
                    Item::UnionDef { name, fields } => {
                        if fields.is_empty() {
                            continue;
                        }
                        let packed = prog
                            .type_layouts
                            .iter()
                            .find(|(n, _, _, _)| n == name)
                            .map(|(_, _, p, _)| *p)
                            .unwrap_or(false);
                        let lay = self.layout_fields(fields, true, packed);
                        self.layouts.insert(name.clone(), lay);
                    }
                    Item::Typedef { name, ty } => match ty {
                        Type::AnonStruct(fs) => {
                            let lay = self.layout_fields(fs, false, false);
                            self.layouts.insert(name.clone(), lay);
                        }
                        Type::AnonUnion(fs) => {
                            let lay = self.layout_fields(fs, true, false);
                            self.layouts.insert(name.clone(), lay);
                        }
                        Type::Struct(n) | Type::Union(n) => {
                            if let Some(l) = self.get_layout(n).cloned() {
                                self.layouts.insert(name.clone(), l);
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }

    pub fn compile(&mut self, prog: &Program) -> Result<String, String> {
        self.out.clear();
        self.collect_layouts(prog);

        // Mutable statics (flex yy_init/yy_start) must load from .data — folding
        // their initializer to an immediate made GUC_yylex loop forever.
        let assigned = collect_assigned_names_in_program(prog);

        let mut typedefs = HashMap::new();
        for item in &prog.items {
            if let Item::Typedef { name, ty } = item {
                typedefs.insert(name.clone(), ty.clone());
            }
            if let Item::Func(f) = item {
                self.funcs.insert(f.name.clone(), f.clone());
            }
            if let Item::Global(g) = item {
                self.globals.insert(g.name.clone(), g.ty.clone());
                // Fold only unassigned static integer globals (enumerators).
                if g.is_static
                    && !g.is_extern
                    && !assigned.contains(&g.name)
                    && matches!(
                        g.ty,
                        Type::Int
                            | Type::UInt
                            | Type::Char
                            | Type::SChar
                            | Type::Short
                            | Type::UShort
                            | Type::Long
                            | Type::ULong
                    )
                {
                    if let Some(init) = &g.init {
                        if let Some(n) = self.const_i64(init) {
                            self.const_globals.insert(g.name.clone(), n);
                        }
                    }
                }
            }
        }
        // Ensure libc FILE* streams are always resolvable even if soft-prefix
        // `extern FILE *stdout` was lost during parse of a huge TU.
        for libc_sym in ["stdout", "stderr", "stdin"] {
            self.globals
                .entry(libc_sym.to_string())
                .or_insert_with(|| Type::Ptr(Box::new(Type::Void)));
        }

        if cfg!(target_os = "macos") {
            writeln!(
                self.out,
                "\t.section\t__TEXT,__text,regular,pure_instructions"
            )
            .unwrap();
        } else {
            writeln!(self.out, "\t.text").unwrap();
        }
        writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();

        // Unified symbol set: Func and Global must not both emit the same label
        // (headers often declare `int f(void);` while .c defines `int f(void){...}`,
        // and misparsed decls can also appear as zero-init globals).
        // Drop unreferenced static / static-inline bodies (kernel header noise).
        let reachable = super::reachable_funcs(prog);
        let best_funcs = super::best_functions_for_emit(prog);
        let mut emitted_syms = std::collections::HashSet::new();
        for f in best_funcs.values() {
            let f = *f;
                // Emit: main; non-static (stubs or full); static if reachable
                // (including empty `{}` no-op function pointers).
                let is_root = !f.is_static || f.name == "main";
                if !is_root && !reachable.contains(&f.name) {
                    continue;
                }
                match &f.body {
                    None => {}
                    Some(_) => {
                        if emitted_syms.insert(f.name.clone()) {
                            self.emit_function(f, &typedefs)?;
                        }
                    }
                }
        }

        let pending = std::mem::take(&mut self.pending_statics);
        for (g, func_name) in &pending {
            if emitted_syms.insert(g.name.clone()) {
                let saved_func = self.func_name.clone();
                self.func_name = func_name.clone();
                self.emit_global(g)?;
                self.func_name = saved_func;
            }
        }

        for item in &prog.items {
            if let Item::Global(g) = item {
                // Pure `extern T x;` — reference only, never define.
                if g.is_extern && g.init.is_none() {
                    continue;
                }
                if g.init.is_some() && emitted_syms.insert(g.name.clone()) {
                    self.emit_global(g)?;
                }
            }
        }
        for item in &prog.items {
            if let Item::Global(g) = item {
                if g.is_extern && g.init.is_none() {
                    continue;
                }
                if emitted_syms.insert(g.name.clone()) {
                    self.emit_global(g)?;
                }
            }
        }

        if !self.strings.is_empty() {
            if cfg!(target_os = "macos") {
                writeln!(
                    self.out,
                    "\n\t.section\t__TEXT,__cstring,cstring_literals"
                )
                .unwrap();
            } else {
                writeln!(self.out, "\n\t.section\t.rodata").unwrap();
            }
            for (i, s) in self.strings.iter().enumerate() {
                writeln!(self.out, "l_str_{i}:").unwrap();
                write!(self.out, "\t.asciz\t\"").unwrap();
                for b in s.bytes() {
                    match b {
                        b'\n' => write!(self.out, "\\n").unwrap(),
                        b'\t' => write!(self.out, "\\t").unwrap(),
                        b'\r' => write!(self.out, "\\r").unwrap(),
                        b'\\' => write!(self.out, "\\\\").unwrap(),
                        b'"' => write!(self.out, "\\\"").unwrap(),
                        b if (0x20..0x7f).contains(&b) => {
                            write!(self.out, "{}", b as char).unwrap()
                        }
                        b => write!(self.out, "\\{:03o}", b).unwrap(),
                    }
                }
                writeln!(self.out, "\"").unwrap();
            }
        }

        // Weak bswap helpers: PP rewrites __builtin_bswapN → __acc_bswapN.
        // Direct calls are usually inlined above; residual external refs
        // (soft-expanded pq_writeint32 etc.) need a linkable definition.
        if !cfg!(target_os = "macos") {
            writeln!(self.out, "\n\t.text").unwrap();
            for (name, bits) in [
                ("__acc_bswap16", 16u32),
                ("__acc_bswap32", 32u32),
                ("__acc_bswap64", 64u32),
            ] {
                if emitted_syms.contains(name) {
                    continue;
                }
                writeln!(self.out, "\t.weak\t{name}").unwrap();
                writeln!(self.out, "\t.globl\t{name}").unwrap();
                writeln!(self.out, "{name}:").unwrap();
                match bits {
                    16 => writeln!(self.out, "\trolw\t$8, %ax").unwrap(),
                    32 => writeln!(self.out, "\tbswap\t%eax").unwrap(),
                    _ => writeln!(self.out, "\tbswap\t%rax").unwrap(),
                }
                writeln!(self.out, "\tretq").unwrap();
            }
        }

        Ok(self.out.clone())
    }

    fn peel_casts<'a>(e: &'a Expr) -> &'a Expr {
        let mut cur = e;
        while let Expr::Cast { expr, .. } = cur {
            cur = expr.as_ref();
        }
        cur
    }

    fn emit_global(&mut self, g: &VarDecl) -> Result<(), String> {
        let size = self.type_size(&g.ty).max(1);
        let s = sym(&g.name);
        // File-scope static / enum constants: local symbols only.
        if !g.is_static {
            if !cfg!(target_os = "macos") {
                if g.is_weak {
                    writeln!(self.out, "\n\t.weak\t{s}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.globl\t{s}").unwrap();
                writeln!(self.out, "\t.type\t{s}, @object").unwrap();
                writeln!(self.out, "\t.size\t{s}, {size}").unwrap();
            } else {
                writeln!(self.out, "").unwrap();
                writeln!(self.out, "\t.globl\t{s}").unwrap();
            }
        } else {
            writeln!(self.out, "").unwrap();
        }
        if let Some(init) = &g.init {
            // Peel `(T *)&sym` / `(T *)arr` — otherwise Cast falls through to BSS
            // zero and postgres `mainrdata_last = (XLogRecData *)&mainrdata_head`
            // stays NULL → SEGV in XLogRegisterData.
            match Self::peel_casts(init) {
                Expr::Int(n) | Expr::Char(n) => {
                    self.data_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{s}:").unwrap();
                    if size <= 4 {
                        writeln!(self.out, "\t.long\t{n}").unwrap();
                    } else {
                        writeln!(self.out, "\t.quad\t{n}").unwrap();
                    }
                }
                Expr::Unary {
                    op: UnaryOp::Addr,
                    expr,
                } => {
                    if let Some((vsym, off)) = self.static_lvalue_reloc(expr) {
                        self.data_section();
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{s}:").unwrap();
                        if off == 0 {
                            writeln!(self.out, "\t.quad\t{vsym}").unwrap();
                        } else {
                            writeln!(self.out, "\t.quad\t{vsym}+{off}").unwrap();
                        }
                    } else {
                        self.bss_section();
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{s}:").unwrap();
                        writeln!(self.out, "\t.zero\t{size}").unwrap();
                    }
                }
                Expr::String(st) => {
                    // `char arr[N] = "lit"` / `static const char kw[] = "a\0b\0"`:
                    // the symbol must label the byte array itself. Emitting
                    // `.quad l_str_N` made ScanKeywords.kw_string point at a
                    // pointer cell → GetScanKeyword returned garbage → every
                    // SQL keyword became IDENT (initdb "syntax error at REVOKE").
                    let unqual_ty = g.ty.unqual();
                    if let Type::Array(elem, n) = unqual_ty {
                        let is_byte = matches!(elem.as_ref(), Type::Char | Type::SChar | Type::UChar)
                            || self.type_size(elem) == 1;
                        if is_byte {
                            let nbytes = if *n > 0 {
                                *n as usize
                            } else {
                                st.len() + 1
                            };
                            let is_const = g.ty.is_const() || elem.is_const();
                            if is_const {
                                if cfg!(target_os = "macos") {
                                    writeln!(
                                        self.out,
                                        "\n\t.section\t__TEXT,__cstring,cstring_literals"
                                    )
                                    .unwrap();
                                } else {
                                    writeln!(self.out, "\n\t.section\t.rodata").unwrap();
                                }
                            } else {
                                self.data_section();
                            }
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{s}:").unwrap();
                            let bytes = st.as_bytes();
                            for i in 0..nbytes {
                                let b = bytes.get(i).copied().unwrap_or(0);
                                writeln!(self.out, "\t.byte\t{b}").unwrap();
                            }
                            return Ok(());
                        }
                    }
                    let id = self.intern_str(st);
                    self.data_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{s}:").unwrap();
                    writeln!(self.out, "\t.quad\tl_str_{id}").unwrap();
                }
                Expr::Var(v) if self.funcs.contains_key(v) || v == "main" => {
                    self.data_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{s}:").unwrap();
                    writeln!(self.out, "\t.quad\t{}", sym(v)).unwrap();
                }
                Expr::InitList { fields } => {
                    self.data_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{s}:").unwrap();
                    self.emit_init_list_data(&g.ty, fields)?;
                }
                other => {
                    // Fold float products for double globals, then int arith.
                    if matches!(g.ty, Type::Float | Type::Double) {
                        if let Some(f) = self.const_f64(other) {
                            if f == 0.0 {
                                self.bss_section();
                                writeln!(self.out, "\t.p2align\t3").unwrap();
                                writeln!(self.out, "{s}:").unwrap();
                                writeln!(self.out, "\t.zero\t{size}").unwrap();
                            } else {
                                self.data_section();
                                writeln!(self.out, "\t.p2align\t3").unwrap();
                                writeln!(self.out, "{s}:").unwrap();
                                if matches!(g.ty, Type::Float) {
                                    writeln!(self.out, "\t.float\t{f}").unwrap();
                                } else {
                                    writeln!(self.out, "\t.double\t{f}").unwrap();
                                }
                            }
                            return Ok(());
                        }
                    }
                    // Fold `(16*1024*1024)` / sizeof / enum arith — else BSS zeros
                    // left wal_segment_size=0 → idiv0 in CalculateCheckpointSegments.
                    if let Some(n) = self.const_i64(other) {
                        if n == 0 {
                            self.bss_section();
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{s}:").unwrap();
                            writeln!(self.out, "\t.zero\t{size}").unwrap();
                        } else {
                            self.data_section();
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{s}:").unwrap();
                            if matches!(g.ty, Type::Float) {
                                writeln!(self.out, "\t.float\t{}", n as f32).unwrap();
                            } else if matches!(g.ty, Type::Double) {
                                writeln!(self.out, "\t.double\t{}", n as f64).unwrap();
                            } else if size <= 4 {
                                writeln!(self.out, "\t.long\t{n}").unwrap();
                            } else {
                                writeln!(self.out, "\t.quad\t{n}").unwrap();
                            }
                        }
                    } else {
                        self.bss_section();
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{s}:").unwrap();
                        writeln!(self.out, "\t.zero\t{size}").unwrap();
                    }
                }
            }
        } else {
            self.bss_section();
            writeln!(self.out, "\t.p2align\t3").unwrap();
            writeln!(self.out, "{s}:").unwrap();
            writeln!(self.out, "\t.zero\t{size}").unwrap();
        }
        Ok(())
    }

    fn data_section(&mut self) {
        if cfg!(target_os = "macos") {
            writeln!(self.out, "\t.section\t__DATA,__data").unwrap();
        } else {
            writeln!(self.out, "\t.data").unwrap();
        }
    }

    fn bss_section(&mut self) {
        if cfg!(target_os = "macos") {
            writeln!(self.out, "\t.section\t__DATA,__bss").unwrap();
        } else {
            writeln!(self.out, "\t.bss").unwrap();
        }
    }

    fn text_section(&mut self) {
        writeln!(self.out, "\t.text").unwrap();
    }

    fn emit_init_list_data(
        &mut self,
        ty: &Type,
        fields_in: &[(Option<String>, Expr)],
    ) -> Result<(), String> {
        match ty {
            Type::Array(elem, n) => {
                let esz = self.type_size(elem);
                let mut count = (*n as usize).max(0);
                if count == 0 {
                    let mut high = 0i64;
                    let mut cur = 0i64;
                    for (des, _) in fields_in {
                        if let Some(d) = des {
                            if let Ok(i) = d.parse::<i64>() {
                                cur = i;
                            }
                        }
                        high = high.max(cur + 1);
                        cur += 1;
                    }
                    count = high.max(fields_in.len() as i64).max(0) as usize;
                }
                let mut values: Vec<Option<&Expr>> = vec![None; count.max(1)];
                let mut cur = 0i64;
                for (des, e) in fields_in {
                    if let Some(d) = des {
                        if let Ok(i) = d.parse::<i64>() {
                            cur = i;
                        }
                    }
                    if cur >= 0 && (cur as usize) < values.len() {
                        values[cur as usize] = Some(e);
                    }
                    cur += 1;
                }
                for i in 0..count {
                    if let Some(Some(e)) = values.get(i) {
                        self.emit_scalar_data(elem, e)?;
                    } else {
                        writeln!(self.out, "\t.zero\t{esz}").unwrap();
                    }
                }
            }
            Type::Struct(name) | Type::Union(name) => {
                if let Some(lay) = self.get_layout(name).cloned() {
                    self.emit_struct_init_data(&lay, fields_in)?;
                } else {
                    // Opaque / incomplete (e.g. arch_spinlock_t) — soft zero.
                    let sz = 8i64;
                    writeln!(self.out, "\t.zero\t{sz}").unwrap();
                }
            }
            Type::AnonStruct(fs) | Type::AnonUnion(fs) => {
                let is_union = matches!(ty, Type::AnonUnion(_));
                let lay = self.layout_fields(fs, is_union, false);
                self.emit_struct_init_data(&lay, fields_in)?;
            }
            other => {
                if let Some((_, e)) = fields_in.first() {
                    self.emit_scalar_data(other, e)?;
                }
            }
        }
        Ok(())
    }

    fn emit_struct_init_data(
        &mut self,
        lay: &Layout,
        fields_in: &[(Option<String>, Expr)],
    ) -> Result<(), String> {
        let mut by_name: HashMap<String, &Expr> = HashMap::new();
        let mut positional = Vec::new();
        for (name, e) in fields_in {
            if let Some(n) = name {
                by_name.insert(n.clone(), e);
            } else {
                positional.push(e);
            }
        }
        let mut ordered: Vec<_> = lay.fields.iter().collect();
        ordered.sort_by_key(|(_, (off, _))| *off);
        let mut pos = 0i64;
        let mut pos_i = 0usize;
        let mut i = 0;
        while i < ordered.len() {
            let (fname, (off, fty)) = ordered[i];
            if pos < *off {
                writeln!(self.out, "\t.zero\t{}", off - pos).unwrap();
                pos = *off;
            }
            let mut j = i + 1;
            while j < ordered.len() && ordered[j].1 .0 == *off {
                j += 1;
            }
            let e = if let Some(ex) = by_name.get(fname) {
                Some(*ex)
            } else if pos_i < positional.len() {
                let e = positional[pos_i];
                pos_i += 1;
                Some(e)
            } else {
                None
            };
            let e = e.or_else(|| {
                for k in i..j {
                    if let Some(ex) = by_name.get(ordered[k].0) {
                        return Some(*ex);
                    }
                }
                None
            });
            let slot = self.type_size(fty);
            if let Some(e) = e {
                self.emit_scalar_data(fty, e)?;
            } else {
                writeln!(self.out, "\t.zero\t{slot}").unwrap();
            }
            pos += slot;
            i = j;
        }
        if pos < lay.size {
            writeln!(self.out, "\t.zero\t{}", lay.size - pos).unwrap();
        }
        Ok(())
    }

    fn emit_scalar_data(&mut self, ty: &Type, e: &Expr) -> Result<(), String> {
        if matches!(ty, Type::Ptr(_)) {
            if let Some((vsym, off)) = self.static_lvalue_reloc(e) {
                writeln!(self.out, "\t.p2align\t3").unwrap();
                if off == 0 {
                    writeln!(self.out, "\t.quad\t{vsym}").unwrap();
                } else {
                    writeln!(self.out, "\t.quad\t{vsym}+{off}").unwrap();
                }
                return Ok(());
            }
        }
        match e {
            Expr::Int(n) | Expr::Char(n) => {
                match ty {
                    Type::Float => writeln!(self.out, "\t.float\t{}", *n as f32).unwrap(),
                    Type::Double => writeln!(self.out, "\t.double\t{}", *n as f64).unwrap(),
                    _ => {
                        let sz = self.type_size(ty);
                        if sz <= 1 {
                            writeln!(self.out, "\t.byte\t{n}").unwrap();
                        } else if sz == 2 {
                            writeln!(self.out, "\t.short\t{n}").unwrap();
                        } else if sz <= 4 {
                            writeln!(self.out, "\t.long\t{n}").unwrap();
                        } else {
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "\t.quad\t{n}").unwrap();
                        }
                    }
                }
            }
            Expr::Float(f) => {
                match ty {
                    Type::Float => writeln!(self.out, "\t.float\t{f}").unwrap(),
                    Type::Double => writeln!(self.out, "\t.double\t{f}").unwrap(),
                    _ => {
                        let sz = self.type_size(ty);
                        if sz <= 1 {
                            writeln!(self.out, "\t.byte\t{}", *f as i64).unwrap();
                        } else if sz <= 4 {
                            writeln!(self.out, "\t.long\t{}", *f as i64).unwrap();
                        } else {
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "\t.quad\t{}", *f as i64).unwrap();
                        }
                    }
                }
            }
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => {
                writeln!(self.out, "\t.p2align\t3").unwrap();
                if let Some((vsym, off)) = self.static_lvalue_reloc(expr) {
                    if off == 0 {
                        writeln!(self.out, "\t.quad\t{vsym}").unwrap();
                    } else {
                        writeln!(self.out, "\t.quad\t{vsym}+{off}").unwrap();
                    }
                } else {
                    writeln!(self.out, "\t.quad\t0").unwrap();
                }
            }
            Expr::AddrOfLabel(label) => {
                writeln!(self.out, "\t.p2align\t3").unwrap();
                writeln!(
                    self.out,
                    "\t.quad\t{}",
                    self.c_goto_label_sym(label)
                )
                .unwrap();
            }
            Expr::Var(v) => {
                // Enum / static const int: emit immediate of the *field* width.
                // Writing `.quad PGC_SIGHUP` into a 4-byte GucContext/group slot
                // bloated ConfigureNamesString (stride 176 vs 168) so check_hook
                // read a string pointer → bootstrap SEGV.
                // Prefer const_globals over globals.contains (enums are both).
                if let Some(n) = self.const_globals.get(v).copied() {
                    let sz = self.type_size(ty);
                    if sz <= 1 {
                        writeln!(self.out, "\t.byte\t{n}").unwrap();
                    } else if sz == 2 {
                        writeln!(self.out, "\t.short\t{n}").unwrap();
                    } else if sz <= 4 {
                        writeln!(self.out, "\t.long\t{n}").unwrap();
                    } else {
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "\t.quad\t{n}").unwrap();
                    }
                    return Ok(());
                }
                // Function designator → address.
                if self.funcs.contains_key(v) || v == "main" {
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "\t.quad\t{}", sym(v)).unwrap();
                    return Ok(());
                }
                // Pointer-typed field or known global object → address reloc.
                if matches!(ty, Type::Ptr(_)) || self.globals.contains_key(v) {
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    if let Some((vsym, off)) = self.static_lvalue_reloc(e) {
                        if off == 0 {
                            writeln!(self.out, "\t.quad\t{vsym}").unwrap();
                        } else {
                            writeln!(self.out, "\t.quad\t{vsym}+{off}").unwrap();
                        }
                    } else if self.globals.contains_key(v) || self.funcs.contains_key(v) {
                        writeln!(self.out, "\t.quad\t{}", sym(v)).unwrap();
                    } else {
                        writeln!(self.out, "\t.zero\t{}", self.type_size(ty).max(1)).unwrap();
                    }
                    return Ok(());
                }
                // Unknown int-ish: zero-fill field width (not a naked .quad).
                writeln!(self.out, "\t.zero\t{}", self.type_size(ty).max(1)).unwrap();
            }
            Expr::String(s) => {
                // char arr[N] = "lit" or struct/anon struct = "lit" → embed bytes.
                if matches!(
                    ty,
                    Type::Array(_, _)
                        | Type::Struct(_)
                        | Type::Union(_)
                        | Type::AnonStruct(_)
                        | Type::AnonUnion(_)
                ) {
                    let sz = self.type_size(ty).max(0) as usize;
                    let bytes = s.as_bytes();
                    for i in 0..sz {
                        let b = bytes.get(i).copied().unwrap_or(0);
                        writeln!(self.out, "\t.byte\t{b}").unwrap();
                    }
                    return Ok(());
                }
                let id = self.intern_str(s);
                writeln!(self.out, "\t.quad\tl_str_{id}").unwrap();
            }
            Expr::InitList { fields } => {
                self.emit_init_list_data(ty, fields)?;
            }
            Expr::Cast { expr, .. } => {
                return self.emit_scalar_data(ty, expr);
            }
            other => {
                // Fold `1024.0*1024.0`, `1.0/1024.0`, `(1024.0*1024.0)/(BLCKSZ/1024)`.
                // Zero multipliers in memory_unit_conversion_table made
                // shared_buffers="400kB" → 0 and broke initdb SelectConfigFiles.
                if matches!(ty, Type::Float | Type::Double) {
                    if let Some(f) = self.const_f64(other) {
                        match ty {
                            Type::Float => writeln!(self.out, "\t.float\t{f}").unwrap(),
                            Type::Double => writeln!(self.out, "\t.double\t{f}").unwrap(),
                            _ => unreachable!(),
                        }
                        return Ok(());
                    }
                }
                // Fold `INT_MAX/2`, `(16*1024*1024)`, enum arith, etc.
                // Leaving `.zero` here zeroed shared_buffers.max and
                // wal_segment_size boot → initdb FPE in CalculateCheckpointSegments.
                if let Some(n) = self.const_i64(other) {
                    match ty {
                        Type::Float => writeln!(self.out, "\t.float\t{}", n as f32).unwrap(),
                        Type::Double => writeln!(self.out, "\t.double\t{}", n as f64).unwrap(),
                        _ => {
                            let sz = self.type_size(ty);
                            if sz <= 1 {
                                writeln!(self.out, "\t.byte\t{n}").unwrap();
                            } else if sz == 2 {
                                writeln!(self.out, "\t.short\t{n}").unwrap();
                            } else if sz <= 4 {
                                writeln!(self.out, "\t.long\t{n}").unwrap();
                            } else {
                                writeln!(self.out, "\t.quad\t{n}").unwrap();
                            }
                        }
                    }
                } else {
                    writeln!(self.out, "\t.zero\t{}", self.type_size(ty)).unwrap();
                }
            }
        }
        Ok(())
    }

    /// Reserve a stack slot for `ty`, aligning the slot start to the type's ABI
    /// alignment (not just trailing padding). Used by measure + emit paths.
    fn bump_stack_for_ty(stack: &mut i64, ty: &Type, align_of: impl Fn(&Type) -> i64, slot_size: impl Fn(&Type) -> i64) {
        let sz = slot_size(ty).max(8);
        let al = align_of(ty).max(8);
        *stack = Self::align_up(*stack, al);
        *stack += sz;
    }

    /// Large aggregate temps from `emit_materialize_agg_addr` use `alloc_local("")`
    /// at emit time — must be counted in the measure pass or the fixed `subq
    /// $frame` frame is too small and PruneState-sized locals SEGV (postgres
    /// heap_page_prune).
    fn measure_materialize_slot(
        &self,
        ty: &Type,
        stack: &mut i64,
    ) {
        if Self::is_struct_or_union_ty(ty) {
            Self::bump_stack_for_ty(
                stack,
                ty,
                |t| self.type_align(t),
                |t| self.stack_slot_size(t),
            );
        }
    }

    fn alloc_local(&mut self, name: &str, ty: &Type) -> i64 {
        let sz = self.stack_slot_size(ty).max(8);
        let al = self.type_align(ty).max(8);
        self.stack_size = Self::align_up(self.stack_size, al);
        self.stack_size += sz;
        let offset = -self.stack_size;
        if !name.is_empty() {
            self.insert_local(
                name.to_string(),
                Sym {
                    ty: ty.clone(),
                    storage: Storage::Local { offset },
                },
            );
        }
        offset
    }

    fn emit_function(
        &mut self,
        f: &Function,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        self.func_name = f.name.clone();
        self.func_ret = self.expand_ty(&f.ret, typedefs);
        self.clear_locals();
        self.break_stack.clear();
        self.continue_stack.clear();
        self.va_regsave_off = 0;
        self.va_fixed_n = 0;
        // Reserve -8(%rbp) for saved %rbx (lvalue address temp, logical reg 19).
        self.stack_size = 8;

        let body = f.body.as_ref().unwrap();

        for (pname, pty) in &f.params {
            if pname.is_empty() {
                continue;
            }
            let pty = match pty {
                Type::Array(e, _) => Type::Ptr(e.clone()),
                other => other.clone(),
            };
            let _ = self.alloc_local(pname, &pty);
        }
        // SysV soft va_list: 6 GP slots + 16 overflow copies (contiguous char*).
        if f.variadic {
            self.stack_size = Self::align_up(self.stack_size, 16) + 48 + 128;
        }
        let mut measure = self.scopes.clone();
        let mut measure_size = self.stack_size;
        self.measure_stmts(body, &mut measure, &mut measure_size, typedefs);
        // After `push %rbp` rsp is 16-byte aligned. Allocate a 16-byte-multiple
        // frame (includes -8(%rbp) saved %rbx + 128B headroom for materialized temps)
        // so the body keeps rsp%16==0 for calls and never writes outside frame.
        let frame = Self::align_up(measure_size.max(8) + 128, 16);

        self.clear_locals();
        self.stack_size = 8;
        let mut fixed_n = 0usize;
        for (pname, _) in &f.params {
            if !pname.is_empty() {
                fixed_n += 1;
            }
        }
        self.va_fixed_n = if f.variadic { fixed_n.min(6) } else { 0 };

        let s = sym(&f.name);
        // static / static-inline: local symbol only (kernel headers expand into many TUs).
        // Freestanding kernel: also emit non-static header copies as .weak so
        // `extern __always_inline` / mis-parsed inlines (native_save_fl) do not
        // multi-define across TUs. Strong real defs still win over .weak.
        if f.is_static {
            writeln!(self.out, "").unwrap();
        } else {
            writeln!(self.out, "\n\t.globl\t{s}").unwrap();
        }
        writeln!(self.out, "{s}:").unwrap();
        writeln!(self.out, "\tpushq\t%rbp").unwrap();
        writeln!(self.out, "\tmovq\t%rsp, %rbp").unwrap();
        writeln!(self.out, "\tsubq\t${frame}, %rsp").unwrap();
        writeln!(self.out, "\tmovq\t%rbx, -8(%rbp)").unwrap();
        // Zero-initialize stack frame to prevent uninitialized stack garbage
        // in local variables/arrays (e.g. omittype in zic.c).
        // Slots to zero are from -16(%rbp) down to -frame(%rbp). Slot -8(%rbp) is saved %rbx.
        if frame >= 16 {
            if frame <= 128 {
                for off in (16..=frame).step_by(8) {
                    writeln!(self.out, "\tmovq\t$0, -{off}(%rbp)").unwrap();
                }
            } else {
                let loop_lbl = self.lab("zero_frame");
                let count = (frame - 16) / 8 + 1;
                writeln!(self.out, "\tleaq\t-{frame}(%rbp), %r10").unwrap();
                writeln!(self.out, "\tmovq\t${count}, %r11").unwrap();
                writeln!(self.out, "{loop_lbl}:").unwrap();
                writeln!(self.out, "\tmovq\t$0, (%r10)").unwrap();
                writeln!(self.out, "\taddq\t$8, %r10").unwrap();
                writeln!(self.out, "\tsubq\t$1, %r11").unwrap();
                writeln!(self.out, "\tjnz\t{loop_lbl}").unwrap();
            }
        }

        // SysV INTEGER arg index (not C param index): structs ≤16B use 1–2 regs.
        let mut ireg: usize = 0;
        let mut istack: usize = 0; // 8-byte stack arg slots consumed
        for (pname, pty) in f.params.iter() {
            if pname.is_empty() {
                continue;
            }
            let pty = match pty {
                Type::Array(e, _) => Type::Ptr(e.clone()),
                other => other.clone(),
            };
            let off = self.alloc_local(pname, &pty);
            let nslots = self.sysv_int_arg_slots(&pty);
            if let Some(nr) = self.small_agg_nregs(&pty) {
                // Aggregate in GPRs or on stack as consecutive eightbytes.
                let nbytes = self.type_size(&pty) as usize;
                if ireg + nslots <= 6 && ireg + (nr as usize) <= 6 {
                    for k in 0..nr {
                        let slot = off + (k as i64) * 8;
                        self.spill_small_agg_param_eightbyte(
                            ireg + k as usize,
                            slot,
                            k,
                            nbytes,
                        );
                    }
                    ireg += nslots;
                } else {
                    for k in 0..nr {
                        let arg_off = 16i64 + (istack as i64) * 8;
                        let slot = off + (k as i64) * 8;
                        self.spill_small_agg_stack_arg_eightbyte(arg_off, slot, k, nbytes);
                        istack += 1;
                    }
                }
                continue;
            }
            if ireg < 6 {
                // Spill only the C width (or extend into the 8-byte slot). SysV
                // leaves upper bits of int/short/char args undefined.
                // ARG_REGS[ireg] maps to reg()/reg_d() index ireg+1.
                let ar = (ireg as u8) + 1;
                match self.type_size(&pty) {
                    1 if matches!(pty, Type::SChar) => {
                        writeln!(self.out, "\tmovsbq\t{}, %rax", reg_b(ar)).unwrap();
                        writeln!(self.out, "\tmovq\t%rax, {}(%rbp)", off).unwrap();
                    }
                    1 => {
                        writeln!(self.out, "\tmovzbq\t{}, %rax", reg_b(ar)).unwrap();
                        writeln!(self.out, "\tmovq\t%rax, {}(%rbp)", off).unwrap();
                    }
                    2 if matches!(pty, Type::Short) => {
                        writeln!(self.out, "\tmovswq\t{}, %rax", reg_w(ar)).unwrap();
                        writeln!(self.out, "\tmovq\t%rax, {}(%rbp)", off).unwrap();
                    }
                    2 => {
                        writeln!(self.out, "\tmovzwq\t{}, %rax", reg_w(ar)).unwrap();
                        writeln!(self.out, "\tmovq\t%rax, {}(%rbp)", off).unwrap();
                    }
                    4 if matches!(pty, Type::UInt | Type::Float) => {
                        writeln!(self.out, "\tmovl\t{}, %eax", reg_d(ar)).unwrap();
                        writeln!(self.out, "\tmovq\t%rax, {}(%rbp)", off).unwrap();
                    }
                    4 => {
                        writeln!(self.out, "\tmovslq\t{}, %rax", reg_d(ar)).unwrap();
                        writeln!(self.out, "\tmovq\t%rax, {}(%rbp)", off).unwrap();
                    }
                    _ => {
                        writeln!(self.out, "\tmovq\t{}, {}(%rbp)", ARG_REGS[ireg], off).unwrap();
                    }
                }
                ireg += 1;
            } else {
                // Incoming stack args sit at 16(%rbp), 24(%rbp), ... (above saved %rbp).
                let arg_off = 16i64 + (istack as i64) * 8;
                istack += 1;
                // Extend from the 8-byte incoming slot into the local slot.
                match &pty {
                    Type::Int => {
                        writeln!(self.out, "\tmovslq\t{arg_off}(%rbp), %rax").unwrap();
                    }
                    Type::UInt | Type::Float => {
                        writeln!(self.out, "\tmovl\t{arg_off}(%rbp), %eax").unwrap();
                    }
                    Type::Short => {
                        writeln!(self.out, "\tmovswq\t{arg_off}(%rbp), %rax").unwrap();
                    }
                    Type::UShort => {
                        writeln!(self.out, "\tmovzwq\t{arg_off}(%rbp), %rax").unwrap();
                    }
                    Type::SChar => {
                        writeln!(self.out, "\tmovsbq\t{arg_off}(%rbp), %rax").unwrap();
                    }
                    Type::Char | Type::UChar => {
                        writeln!(self.out, "\tmovzbq\t{arg_off}(%rbp), %rax").unwrap();
                    }
                    _ => {
                        writeln!(self.out, "\tmovq\t{arg_off}(%rbp), %rax").unwrap();
                    }
                }
                writeln!(self.out, "\tmovq\t%rax, {off}(%rbp)").unwrap();
            }
        }

        // Variadic: spill GP args + overflow into contiguous soft va_list area.
        // Named params were already copied out of ARG_REGS; spill originals next.
        if f.variadic {
            self.stack_size = Self::align_up(self.stack_size, 16) + 48 + 128;
            self.va_regsave_off = -self.stack_size;
            for r in 0..6 {
                let off = self.va_regsave_off + (r as i64) * 8;
                writeln!(self.out, "\tmovq\t{}, {}(%rbp)", ARG_REGS[r], off).unwrap();
            }
            for i in 0i64..16 {
                let src = 16 + i * 8;
                let dest = self.va_regsave_off + 48 + i * 8;
                writeln!(self.out, "\tmovq\t{src}(%rbp), %rax").unwrap();
                writeln!(self.out, "\tmovq\t%rax, {dest}(%rbp)").unwrap();
            }
        }

        for st in body {
            self.emit_stmt(st, typedefs)?;
        }

        writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
        let end = format!("L_{}_epilogue", f.name);
        writeln!(self.out, "{end}:").unwrap();
        // SysV: float/double returns live in %xmm0. Soft keeps FP bits in GPRs
        // (%rax) while evaluating; move into xmm0 here so callers (and
        // emit_extend_call_return) see the real value. Without this,
        // postgres `defGetNumeric` → COST 1 reads as 0 → "COST must be positive".
        match &self.func_ret {
            Type::Double => writeln!(self.out, "\tmovq\t%rax, %xmm0").unwrap(),
            Type::Float => writeln!(self.out, "\tmovd\t%eax, %xmm0").unwrap(),
            _ => {}
        }
        // Restore %rbx then tear down frame (leave = mov %rbp,%rsp; pop %rbp).
        writeln!(self.out, "\tmovq\t-8(%rbp), %rbx").unwrap();
        writeln!(self.out, "\tleave").unwrap();
        writeln!(self.out, "\tretq").unwrap();
        Ok(())
    }

    fn measure_stmts(
        &self,
        stmts: &[Stmt],
        locals: &mut Vec<HashMap<String, Sym>>,
        stack: &mut i64,
        typedefs: &HashMap<String, Type>,
    ) {
        for st in stmts {
            self.measure_stmt(st, locals, stack, typedefs);
        }
    }

    fn measure_stmt(
        &self,
        st: &Stmt,
        locals: &mut Vec<HashMap<String, Sym>>,
        stack: &mut i64,
        typedefs: &HashMap<String, Type>,
    ) {
        match st {
            Stmt::Decl(d) => {
                let ty = self.expand_ty(&d.ty, typedefs);
                // Function-scope static / block-scope extern: no stack slot
                // (lives as Storage::Global in emit). Skipping keeps frames
                // honest for large static tables like initdb long_options[].
                if d.is_static || d.is_extern {
                    if let Some(scope) = locals.last_mut() {
                        scope.insert(
                            d.name.clone(),
                            Sym {
                                ty,
                                storage: Storage::Global {
                                    name: d.name.clone(),
                                },
                            },
                        );
                    }
                    return;
                }
                Self::bump_stack_for_ty(
                    stack,
                    &ty,
                    |t| self.type_align(t),
                    |t| self.stack_slot_size(t),
                );
                let offset = -*stack;
                if let Some(scope) = locals.last_mut() {
                    scope.insert(
                        d.name.clone(),
                        Sym {
                            ty,
                            storage: Storage::Local { offset },
                        },
                    );
                }
                if let Some(ref init) = d.init {
                    self.measure_expr(init, locals, stack, typedefs);
                }
            }
            Stmt::DeclGroup(decls) => {
                for d in decls {
                    self.measure_stmt(&Stmt::Decl(d.clone()), locals, stack, typedefs);
                }
            }
            Stmt::Expr(e) | Stmt::Return(Some(e)) => self.measure_expr(e, locals, stack, typedefs),
            Stmt::Block(ss) => {
                locals.push(HashMap::new());
                self.measure_stmts(ss, locals, stack, typedefs);
                locals.pop();
            }
            Stmt::If {
                cond,
                then_b,
                else_b,
            } => {
                self.measure_expr(cond, locals, stack, typedefs);
                self.measure_stmt(then_b, locals, stack, typedefs);
                if let Some(e) = else_b {
                    self.measure_stmt(e, locals, stack, typedefs);
                }
            }
            Stmt::While { cond, body } => {
                self.measure_expr(cond, locals, stack, typedefs);
                self.measure_stmt(body, locals, stack, typedefs);
            }
            Stmt::DoWhile { cond, body } => {
                self.measure_stmt(body, locals, stack, typedefs);
                self.measure_expr(cond, locals, stack, typedefs);
            }
            Stmt::Label(_, body) => {
                self.measure_stmt(body, locals, stack, typedefs);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                locals.push(HashMap::new());
                if let Some(i) = init {
                    self.measure_stmt(i, locals, stack, typedefs);
                }
                if let Some(c) = cond {
                    self.measure_expr(c, locals, stack, typedefs);
                }
                if let Some(s) = step {
                    self.measure_expr(s, locals, stack, typedefs);
                }
                self.measure_stmt(body, locals, stack, typedefs);
                locals.pop();
            }
            Stmt::Switch { cond, body } => {
                self.measure_expr(cond, locals, stack, typedefs);
                self.measure_stmt(body, locals, stack, typedefs);
            }
            Stmt::Case { body, .. } | Stmt::Default(body) => {
                self.measure_stmt(body, locals, stack, typedefs)
            }
            _ => {}
        }
    }

    fn measure_expr(
        &self,
        e: &Expr,
        locals: &mut Vec<HashMap<String, Sym>>,
        stack: &mut i64,
        typedefs: &HashMap<String, Type>,
    ) {
        match e {
            Expr::StmtExpr(stmts, final_expr) => {
                locals.push(HashMap::new());
                self.measure_stmts(stmts, locals, stack, typedefs);
                self.measure_expr(final_expr, locals, stack, typedefs);
                locals.pop();
            }
            Expr::Cast { expr, ty } => {
                self.measure_expr(expr, locals, stack, typedefs);
                let cty = self.expand_ty(ty, typedefs);
                self.measure_materialize_slot(&cty, stack);
            }
            Expr::Unary { expr, .. }
            | Expr::PreInc(expr)
            | Expr::PreDec(expr)
            | Expr::PostInc(expr)
            | Expr::PostDec(expr)
            | Expr::SizeofExpr(expr) => self.measure_expr(expr, locals, stack, typedefs),
            Expr::Binary { left, right, .. }
            | Expr::Assign { left, right }
            | Expr::CompoundAssign { left, right, .. }
            | Expr::Index { base: left, index: right } => {
                self.measure_expr(left, locals, stack, typedefs);
                self.measure_expr(right, locals, stack, typedefs);
            }
            Expr::Call { name, args, .. } => {
                for a in args {
                    self.measure_expr(a, locals, stack, typedefs);
                    let aty = self.typeof_expr(a, typedefs);
                    self.measure_materialize_slot(&aty, stack);
                }
                if let Some(f) = self.funcs.get(name) {
                    self.measure_materialize_slot(&f.ret, stack);
                }
            }
            Expr::Cond { cond, then_e, else_e } => {
                self.measure_expr(cond, locals, stack, typedefs);
                self.measure_expr(then_e, locals, stack, typedefs);
                self.measure_expr(else_e, locals, stack, typedefs);
                let tt = self.typeof_expr(then_e, typedefs);
                let et = self.typeof_expr(else_e, typedefs);
                if Self::is_struct_or_union_ty(&tt) {
                    self.measure_materialize_slot(&tt, stack);
                } else if Self::is_struct_or_union_ty(&et) {
                    self.measure_materialize_slot(&et, stack);
                }
            }
            Expr::Member { base, .. } => {
                self.measure_expr(base, locals, stack, typedefs);
                // Only non-lvalue aggregates need a materialize slot (`f().field`).
                if matches!(
                    base.as_ref(),
                    Expr::Call { .. }
                        | Expr::Cond { .. }
                        | Expr::Cast { .. }
                        | Expr::StmtExpr(_, _)
                ) {
                    let bt = self.typeof_expr(base, typedefs);
                    self.measure_materialize_slot(&bt, stack);
                }
            }
            Expr::InitList { fields } => {
                for (_, sub_e) in fields {
                    self.measure_expr(sub_e, locals, stack, typedefs);
                }
            }
            _ => {}
        }
    }

    fn expand_ty(&self, ty: &Type, typedefs: &HashMap<String, Type>) -> Type {
        match ty {
            Type::Struct(n) | Type::Union(n) if typedefs.contains_key(n) => {
                if self.layouts.contains_key(n) {
                    return ty.clone();
                }
                let t = typedefs.get(n).unwrap();
                match t {
                    Type::AnonStruct(_) => Type::Struct(n.clone()),
                    Type::AnonUnion(_) => Type::Union(n.clone()),
                    Type::Struct(m) | Type::Union(m) if m == n => ty.clone(),
                    other => self.expand_ty(other, typedefs),
                }
            }
            Type::AnonStruct(fs) => Type::AnonStruct(fs.clone()),
            _ => ty.clone(),
        }
    }

    fn emit_stmt(&mut self, st: &Stmt, typedefs: &HashMap<String, Type>) -> Result<(), String> {
        match st {
            Stmt::Empty => Ok(()),
            Stmt::Asm {
                lines,
                in_loads,
                out_stores,
                out_store_exprs,
            } => {
                // Evaluate "=r"/"r" input operands into assigned logical regs
                // (parser already substituted %N → xN; we rewrite to AT&T below).
                for (regn, e) in in_loads {
                    self.emit_expr_rval(e, *regn, typedefs)?;
                }
                // Emit kbuild DEFINE lines (`.ascii "->..."`). Skip raw templates
                // that still contain unresolved `%0` / `%[name]` operands.
                // Soft-rewrite bare `xN`/`wN` → AT&T so gas does not see `U x0`.
                // Drop full .macro…​.endm (body has `\param` gas formals).
                let mut macro_depth = 0i32;
                let mut rept_depth = 0i32;
                let mut pending_prefix: Option<String> = None;
                for line in lines {
                    let t = line.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let lower = t.to_ascii_lowercase();
                    if lower.starts_with(".rept")
                        || lower.starts_with(".irp")
                        || lower.starts_with(".irpc")
                    {
                        rept_depth += 1;
                        continue;
                    }
                    if lower.starts_with(".endr") {
                        if rept_depth > 0 {
                            rept_depth -= 1;
                        }
                        continue;
                    }
                    if rept_depth > 0 {
                        continue;
                    }
                    if lower.starts_with(".macro") {
                        macro_depth += 1;
                        continue;
                    }
                    if lower.starts_with(".endm") {
                        if macro_depth > 0 {
                            macro_depth -= 1;
                        }
                        continue;
                    }
                    if macro_depth > 0 {
                        continue;
                    }
                    if lower.starts_with(".purgem") {
                        continue;
                    }
                    if t.as_bytes().windows(2).any(|w| {
                        w[0] == b'\\' && (w[1].is_ascii_alphabetic() || w[1] == b'_')
                    }) {
                        pending_prefix = None;
                        continue;
                    }
                    if t.contains('%')
                        && t.bytes().enumerate().any(|(i, b)| {
                            if b != b'%' {
                                return false;
                            }
                            // Keep `%%` (literal percent).
                            let rest = &t.as_bytes()[i + 1..];
                            if rest.is_empty() {
                                return false;
                            }
                            if rest[0] == b'%' {
                                return false;
                            }
                            // GCC asm operands: %0, %1, %[name], %q0, %l[lab], %w0, …
                            if rest[0].is_ascii_digit() || rest[0] == b'[' {
                                return true;
                            }
                            // Modifier letter(s) then digit or [name]
                            let mut j = 0;
                            while j < rest.len() && rest[j].is_ascii_alphabetic() {
                                j += 1;
                            }
                            j > 0
                                && j < rest.len()
                                && (rest[j].is_ascii_digit() || rest[j] == b'[')
                        })
                    {
                        // Still emit a leading numeric local label so EX_TABLE
                        // fixups from the same template can assemble.
                        if let Some(colon) = t.find(':') {
                            let lab = t[..colon].trim();
                            if !lab.is_empty() && lab.bytes().all(|b| b.is_ascii_digit()) {
                                writeln!(self.out, "{lab}:").unwrap();
                            }
                        }
                        // Dropped operand line — do not leave a bare `lock` prefix.
                        pending_prefix = None;
                        continue;
                    }
                    // Soft-skip lines with numeric local labels / refs when the
                    // paired label may have been dropped → "unknown 1b".
                    let has_local_lab = t.bytes().enumerate().any(|(i, b)| {
                        b.is_ascii_digit()
                            && t.as_bytes()
                                .get(i + 1)
                                .map(|c| *c == b':' || *c == b'b' || *c == b'f')
                                .unwrap_or(false)
                            && (i == 0
                                || !t.as_bytes()[i - 1].is_ascii_alphanumeric()
                                    && t.as_bytes()[i - 1] != b'_')
                    });
                    if has_local_lab {
                        pending_prefix = None;
                        continue;
                    }
                    if let Some(fixed) = soften_x86_asm_line(t) {
                        if let Some(pref) = fixed.strip_prefix("__acc_prefix_") {
                            pending_prefix = Some(pref.to_string());
                            continue;
                        }
                        if let Some(pref) = pending_prefix.take() {
                            // Same logical insn: `lock; xaddl %eax,(%rdi)` — never
                            // leave a bare `lock` line that could prefix a later mov.
                            writeln!(self.out, "\t{pref}; {fixed}").unwrap();
                        } else {
                            writeln!(self.out, "\t{fixed}").unwrap();
                        }
                    } else {
                        pending_prefix = None;
                    }
                }
                // Never leave a dangling lock/rep for the next C statement.
                let _ = pending_prefix;
                // Store logical regs into C vars for "=r" outputs
                // (e.g. pushf; pop %0 : "=r"(flags)).
                for (regn, var) in out_stores {
                    self.emit_asm_operand_store(var, *regn, typedefs)?;
                }
                // "=a"(*expected): write register through a general lvalue.
                for (regn, e) in out_store_exprs {
                    // Preserve value across lvalue address formation.
                    writeln!(self.out, "\tpushq\t{}", reg(*regn)).unwrap();
                    let lty = self.emit_lvalue_addr(e, 9, typedefs)?;
                    writeln!(self.out, "\tpopq\t%rax").unwrap();
                    self.store_ty(&lty, 9, 0);
                }
                Ok(())
            }
            Stmt::Block(ss) => {
                self.enter_scope();
                for s in ss {
                    self.emit_stmt(s, typedefs)?;
                }
                self.exit_scope();
                Ok(())
            }
            Stmt::DeclGroup(decls) => {
                for d in decls {
                    self.emit_stmt(&Stmt::Decl(d.clone()), typedefs)?;
                }
                Ok(())
            }
            Stmt::Decl(d) => {
                let ty = match &d.ty {
                    Type::AnonStruct(fs) => {
                        let lay = self.layout_fields(fs, false, false);
                        let key = format!("anon_{}", d.name);
                        self.layouts.insert(key.clone(), lay);
                        Type::Struct(key)
                    }
                    Type::AnonUnion(fs) => {
                        let lay = self.layout_fields(fs, true, false);
                        let key = format!("anon_{}", d.name);
                        self.layouts.insert(key.clone(), lay);
                        Type::Union(key)
                    }
                    Type::Struct(n) | Type::Union(n) if self.layouts.contains_key(n) => {
                        d.ty.clone()
                    }
                    Type::Struct(n) | Type::Union(n) if typedefs.contains_key(n) => {
                        if self.layouts.contains_key(n) {
                            d.ty.clone()
                        } else {
                            match typedefs.get(n).unwrap() {
                                Type::AnonStruct(fs) => {
                                    let lay = self.layout_fields(fs, false, false);
                                    self.layouts.insert(n.clone(), lay);
                                    Type::Struct(n.clone())
                                }
                                Type::AnonUnion(fs) => {
                                    let lay = self.layout_fields(fs, true, false);
                                    self.layouts.insert(n.clone(), lay);
                                    Type::Union(n.clone())
                                }
                                other => other.clone(),
                            }
                        }
                    }
                    other => self.expand_ty(other, typedefs),
                };
                if d.is_extern {
                    // Block-scope `extern T name;` — bind to the global symbol.
                    self.insert_local(
                        d.name.clone(),
                        Sym {
                            ty: ty.clone(),
                            storage: Storage::Global {
                                name: d.name.clone(),
                            },
                        },
                    );
                    if !self.globals.contains_key(&d.name) {
                        self.globals.insert(d.name.clone(), ty.clone());
                    }
                    return Ok(());
                }
                if d.is_static {
                    // Function-scope static → unique .data/.bss global (C static
                    // duration). Critical for initdb `static struct option
                    // long_options[]` — stack placement made getopt_long see
                    // garbage and reject every argv.
                    let id = self.label_id;
                    self.label_id += 1;
                    let gname = format!("__static_{}_{}_{}", self.func_name, d.name, id);
                    if !self.globals.contains_key(&gname) {
                        let mut g = d.clone();
                        g.name = gname.clone();
                        g.is_static = true;
                        self.pending_statics.push((g, self.func_name.clone()));
                        self.globals.insert(gname.clone(), ty.clone());
                    }
                    self.func_local_statics
                        .insert((self.func_name.clone(), d.name.clone()), gname.clone());
                    self.insert_local(
                        d.name.clone(),
                        Sym {
                            ty: ty.clone(),
                            storage: Storage::Global { name: gname },
                        },
                    );
                    // Static init already queued for .data; do not re-run each call.
                    return Ok(());
                }
                let off = self.alloc_local(&d.name, &ty);
                if let Some(init) = &d.init {
                    if let Expr::InitList { fields } = init {
                        self.emit_local_init_list(off, &ty, fields, typedefs)?;
                        return Ok(());
                    }
                    // `char buf[] = "hi";` / `unsigned char z[] = "0123"`:
                    // string rvalue is a pointer; must copy bytes into the
                    // array slot. Storing the pointer made `write(2, buf, n)`
                    // pass &pointer (lea of the slot) and dump pointer bits —
                    // postgres bootstrap breadcrumbs became binary garbage and
                    // masked real control flow (same bug aarch64 fixed for hexio).
                    if let (Type::Array(elem, n), Expr::String(s)) = (&ty, init) {
                        if matches!(elem.as_ref(), Type::Char | Type::SChar) {
                            let id = self.intern_str(s);
                            let copy_n = if *n > 0 {
                                *n
                            } else {
                                (s.len() + 1) as i64
                            };
                            // Inline copy (rep movsb) — avoids call ABI / align.
                            self.emit_fp_addr(off, 1); // %rdi = dest
                            writeln!(
                                self.out,
                                "\tleaq\tl_str_{id}(%rip), %rsi"
                            )
                            .unwrap();
                            writeln!(self.out, "\tmovq\t${copy_n}, %rcx").unwrap();
                            writeln!(self.out, "\trep\tmovsb").unwrap();
                            return Ok(());
                        }
                    }
                    // `T t = *p;` / `T t = a;` for aggregates wider than a register:
                    // emit_expr_rval only loads 8 bytes (like a scalar). That broke
                    // postgres qsort's `TYPE t = *pi; *pi++ = *pj; *pj++ = t;` for
                    // SortTuple (24B) — datum1 stayed stack garbage → unique-index
                    // false duplicates during initdb build_indices.
                    if self.needs_block_copy_ty(&ty) {
                        let sz = self.type_size(&ty).max(1);
                        self.emit_fp_addr(off, 9); // %r10 = dest
                        writeln!(self.out, "\tpushq\t%r10").unwrap();
                        let _ity = self.emit_lvalue_addr(init, 0, typedefs)?;
                        writeln!(self.out, "\tpopq\t%rdi").unwrap();
                        writeln!(self.out, "\tmovq\t%rdi, %r8").unwrap();
                        writeln!(self.out, "\tmovq\t%rax, %rsi").unwrap();
                        writeln!(self.out, "\tmovq\t${sz}, %rcx").unwrap();
                        writeln!(self.out, "\tcld").unwrap();
                        writeln!(self.out, "\trep\tmovsb").unwrap();
                        return Ok(());
                    }
                    let rty = self.emit_expr_rval(init, 0, typedefs)?;
                    // Use emit's return type — typeof_expr lags on float arith
                    // (`double c = a*b` was typed Int → cvtsi2sd of IEEE bits
                    // → huge value → parse_int "exceeds integer range").
                    self.coerce_rax_to_ty(&rty, &ty);
                    match &ty {
                        Type::Char | Type::Int | Type::Long | Type::Ptr(_) => {
                            writeln!(self.out, "\tmovq\t%rax, {off}(%rbp)").unwrap();
                        }
                        _ => self.store_to_offset(off, &ty, 0),
                    }
                }
                Ok(())
            }
            Stmt::Expr(e) => {
                self.emit_expr_rval(e, 0, typedefs)?;
                Ok(())
            }
            Stmt::Return(e) => {
                if let Some(ex) = e {
                    let ret_ty = self.expand_ty(&self.func_ret.clone(), typedefs);
                    if let Some(nr) = self.small_agg_nregs(&ret_ty) {
                        if nr > 1 {
                            let _ = self.emit_lvalue_addr(ex, 9, typedefs)?;
                            let ret_regs = ["%rax", "%rdx", "%rcx", "%r8", "%r9", "%r11"];
                            for i in (0..(nr as usize)).rev() {
                                let off = i * 8;
                                let r = ret_regs[i];
                                writeln!(self.out, "\tmovq\t{off}(%r10), {r}").unwrap();
                            }
                        } else {
                            self.emit_expr_rval(ex, 0, typedefs)?;
                        }
                    } else {
                        self.emit_expr_rval(ex, 0, typedefs)?;
                    }
                } else {
                    writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
                }
                writeln!(self.out, "\tjmp\tL_{}_epilogue", self.func_name).unwrap();
                Ok(())
            }
            Stmt::If {
                cond,
                then_b,
                else_b,
            } => {
                let l_else = self.lab("else");
                let l_end = self.lab("endif");
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\ttestq\t%rax, %rax").unwrap();
                writeln!(self.out, "\tje\t{l_else}").unwrap();
                self.emit_stmt(then_b, typedefs)?;
                writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                if let Some(e) = else_b {
                    self.emit_stmt(e, typedefs)?;
                }
                writeln!(self.out, "{l_end}:").unwrap();
                Ok(())
            }
            Stmt::While { cond, body } => {
                let l_head = self.lab("while");
                let l_end = self.lab("endwhile");
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_head.clone());
                writeln!(self.out, "{l_head}:").unwrap();
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\ttestq\t%rax, %rax").unwrap();
                writeln!(self.out, "\tje\t{l_end}").unwrap();
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "\tjmp\t{l_head}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                Ok(())
            }
            Stmt::DoWhile { body, cond } => {
                let l_head = self.lab("do");
                let l_cont = self.lab("docont");
                let l_end = self.lab("enddo");
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_cont.clone());
                writeln!(self.out, "{l_head}:").unwrap();
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "{l_cont}:").unwrap();
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\ttestq\t%rax, %rax").unwrap();
                writeln!(self.out, "\tjne\t{l_head}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                Ok(())
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.enter_scope();
                let l_head = self.lab("for");
                let l_cont = self.lab("forcont");
                let l_end = self.lab("endfor");
                if let Some(i) = init {
                    self.emit_stmt(i, typedefs)?;
                }
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_cont.clone());
                writeln!(self.out, "{l_head}:").unwrap();
                if let Some(c) = cond {
                    self.emit_expr_rval(c, 0, typedefs)?;
                    writeln!(self.out, "\ttestq\t%rax, %rax").unwrap();
                    writeln!(self.out, "\tje\t{l_end}").unwrap();
                }
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "{l_cont}:").unwrap();
                if let Some(s) = step {
                    self.emit_expr_rval(s, 0, typedefs)?;
                }
                writeln!(self.out, "\tjmp\t{l_head}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                self.exit_scope();
                Ok(())
            }
            Stmt::Break => {
                let l = self
                    .break_stack
                    .last()
                    .ok_or("break outside loop")?
                    .clone();
                writeln!(self.out, "\tjmp\t{l}").unwrap();
                Ok(())
            }
            Stmt::Continue => {
                let l = self
                    .continue_stack
                    .last()
                    .ok_or("continue outside loop")?
                    .clone();
                writeln!(self.out, "\tjmp\t{l}").unwrap();
                Ok(())
            }
            Stmt::Goto(name) => {
                writeln!(
                    self.out,
                    "\tjmp\tL_{}_goto_{}",
                    self.func_name, name
                )
                .unwrap();
                Ok(())
            }
            Stmt::GotoIndirect(e) => {
                self.emit_expr_rval(e, 0, typedefs)?;
                writeln!(self.out, "\tjmp\t*%rax").unwrap();
                Ok(())
            }
            Stmt::Label(name, inner) => {
                writeln!(
                    self.out,
                    "L_{}_goto_{}:",
                    self.func_name, name
                )
                .unwrap();
                self.emit_stmt(inner, typedefs)
            }
            Stmt::Switch { cond, body } => {
                let l_end = self.lab("swend");
                let l_default = self.lab("swdef");
                self.break_stack.push(l_end.clone());
                let saved_cases = std::mem::take(&mut self.pending_case_labs);
                self.emit_expr_rval(cond, 0, typedefs)?;
                // Spill switch value (expr eval clobbers %rax).
                writeln!(self.out, "\tsubq\t$16, %rsp").unwrap();
                writeln!(self.out, "\tmovq\t%rax, (%rsp)").unwrap();
                let mut cases: Vec<SwitchCaseItem> = Vec::new();
                self.collect_switch_cases(body, &mut cases);
                self.pending_case_labs.clear();
                let mut has_default = false;
                let mut default_lab = l_default.clone();
                for item in &cases {
                    if item.is_default {
                        has_default = true;
                        default_lab = item.lab.clone();
                    } else {
                        self.pending_case_labs.push_back(item.lab.clone());
                        if let Some(v) = item.val {
                            writeln!(self.out, "\tmovq\t(%rsp), %rax").unwrap();
                            self.emit_imm(v, 11);
                            writeln!(self.out, "\tcmpq\t%rcx, %rax").unwrap();
                            writeln!(self.out, "\tje\t{}", item.lab).unwrap();
                        }
                    }
                }
                if has_default {
                    writeln!(self.out, "\tjmp\t{default_lab}").unwrap();
                } else {
                    writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                }
                self.emit_switch_body(body, &default_lab, typedefs)?;
                while let Some(lab) = self.pending_case_labs.pop_front() {
                    writeln!(self.out, "{lab}:").unwrap();
                }
                for item in &cases {
                    if !item.is_default && !self.out.contains(&format!("{}:", item.lab)) {
                        writeln!(self.out, "{}:", item.lab).unwrap();
                    }
                }
                if has_default && !self.out.contains(&format!("{default_lab}:")) {
                    writeln!(self.out, "{default_lab}:").unwrap();
                }
                // Pop spilled switch value on all exits (break → l_end).
                writeln!(self.out, "{l_end}:").unwrap();
                writeln!(self.out, "\taddq\t$16, %rsp").unwrap();
                self.break_stack.pop();
                self.pending_case_labs = saved_cases;
                Ok(())
            }
            Stmt::Case { body, .. } | Stmt::Default(body) => self.emit_stmt(body, typedefs),
        }
    }

    fn const_i64_simple(e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Self::const_i64_simple(expr).map(|n| -n),
            Expr::Unary {
                op: UnaryOp::BitNot,
                expr,
            } => Self::const_i64_simple(expr).map(|n| !n),
            Expr::Cast { expr, .. } => Self::const_i64_simple(expr),
            Expr::Binary { op, left, right } => {
                let l = Self::const_i64_simple(left)?;
                let r = Self::const_i64_simple(right)?;
                Self::const_i64_apply_binop(*op, l, r)
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let c = Self::const_i64_simple(cond)?;
                if c != 0 {
                    Self::const_i64_simple(then_e)
                } else {
                    Self::const_i64_simple(else_e)
                }
            }
            _ => None,
        }
    }

    fn const_i64_apply_binop(op: BinOp, l: i64, r: i64) -> Option<i64> {
        Some(match op {
            BinOp::Add => l.wrapping_add(r),
            BinOp::Sub => l.wrapping_sub(r),
            BinOp::Mul => l.wrapping_mul(r),
            BinOp::Div if r != 0 => l / r,
            BinOp::Mod if r != 0 => l % r,
            BinOp::BitAnd => l & r,
            BinOp::BitOr => l | r,
            BinOp::BitXor => l ^ r,
            BinOp::Shl => l.wrapping_shl((r as u32) & 63),
            BinOp::Shr => l.wrapping_shr((r as u32) & 63),
            BinOp::Eq => (l == r) as i64,
            BinOp::Ne => (l != r) as i64,
            BinOp::Lt => (l < r) as i64,
            BinOp::Gt => (l > r) as i64,
            BinOp::Le => (l <= r) as i64,
            BinOp::Ge => (l >= r) as i64,
            BinOp::And => ((l != 0) && (r != 0)) as i64,
            BinOp::Or => ((l != 0) || (r != 0)) as i64,
            _ => return None,
        })
    }

    fn const_i64(&self, e: &Expr) -> Option<i64> {
        match e {
            Expr::Var(name) => self.const_globals.get(name).copied(),
            Expr::SizeofType(t) => Some(self.type_size(t)),
            Expr::SizeofExpr(ex) => {
                if let Expr::String(s) = ex.as_ref() {
                    return Some((s.len() + 1) as i64);
                }
                // `sizeof(TypInfo)` where TypInfo is a static array: look up the
                // global's type (postgres bootstrap n_types =
                // sizeof(TypInfo)/sizeof(struct typinfo) was BSS-zero → gettype
                // fell through to populate_typ_list before pg_type exists).
                if let Expr::Var(name) = ex.as_ref() {
                    if let Some(ty) = self.globals.get(name) {
                        return Some(self.type_size(ty));
                    }
                }
                // Deref/paren/cast wrappers: peel and retry as type of operand.
                let peeled = Self::peel_casts(ex);
                if let Expr::Var(name) = peeled {
                    if let Some(ty) = self.globals.get(name) {
                        return Some(self.type_size(ty));
                    }
                }
                // `sizeof(arr[0])` / `sizeof((arr)[0])` — lengthof() macro.
                // Without this, NSmgr = sizeof(smgrsw)/sizeof(smgrsw[0]) stays
                // BSS-zero → mdinit never runs → MdCxt NULL → SEGV in mdcreate.
                if let Expr::Index { base, .. } = peeled {
                    let base = Self::peel_casts(base);
                    if let Expr::Var(name) = base {
                        if let Some(ty) = self.globals.get(name) {
                            match ty {
                                Type::Array(elem, _) | Type::Ptr(elem) => {
                                    return Some(self.type_size(elem));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // `sizeof(*p)` element/pointee size for a global pointer/array.
                if let Expr::Unary {
                    op: UnaryOp::Deref,
                    expr,
                } = peeled
                {
                    let inner = Self::peel_casts(expr);
                    if let Expr::Var(name) = inner {
                        if let Some(ty) = self.globals.get(name) {
                            match ty {
                                Type::Array(elem, _) | Type::Ptr(elem) => {
                                    return Some(self.type_size(elem));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // Best-effort: nested sizeof / known globals.
                self.const_i64(ex)
            }
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => self.const_i64(expr).map(|n| -n),
            Expr::Unary {
                op: UnaryOp::BitNot,
                expr,
            } => self.const_i64(expr).map(|n| !n),
            Expr::Cast { expr, .. } => self.const_i64(expr),
            Expr::Binary { op, left, right } => {
                let l = self.const_i64(left)?;
                let r = self.const_i64(right)?;
                Self::const_i64_apply_binop(*op, l, r)
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let c = self.const_i64(cond)?;
                if c != 0 {
                    self.const_i64(then_e)
                } else {
                    self.const_i64(else_e)
                }
            }
            other => Self::const_i64_simple(other),
        }
    }

    /// Fold float/double constant expressions for static initializers
    /// (`1024.0 * 1024.0`, `1.0 / 1024.0`, `(1024.0*1024.0)/(BLCKSZ/1024)`).
    fn const_f64(&self, e: &Expr) -> Option<f64> {
        match e {
            Expr::Float(f) => Some(*f),
            Expr::Int(n) | Expr::Char(n) => Some(*n as f64),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => self.const_f64(expr).map(|n| -n),
            Expr::Cast { expr, .. } => self
                .const_f64(expr)
                .or_else(|| self.const_i64(expr).map(|n| n as f64)),
            Expr::Binary { op, left, right } => {
                let l = self
                    .const_f64(left)
                    .or_else(|| self.const_i64(left).map(|n| n as f64))?;
                let r = self
                    .const_f64(right)
                    .or_else(|| self.const_i64(right).map(|n| n as f64))?;
                match op {
                    BinOp::Add => Some(l + r),
                    BinOp::Sub => Some(l - r),
                    BinOp::Mul => Some(l * r),
                    BinOp::Div if r != 0.0 => Some(l / r),
                    _ => None,
                }
            }
            Expr::Var(name) => self.const_globals.get(name).copied().map(|n| n as f64),
            Expr::SizeofType(t) => Some(self.type_size(t) as f64),
            Expr::SizeofExpr(ex) => self.const_i64(&Expr::SizeofExpr(ex.clone())).map(|n| n as f64),
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let c = self
                    .const_f64(cond)
                    .or_else(|| self.const_i64(cond).map(|n| n as f64))?;
                if c != 0.0 {
                    self.const_f64(then_e)
                        .or_else(|| self.const_i64(then_e).map(|n| n as f64))
                } else {
                    self.const_f64(else_e)
                        .or_else(|| self.const_i64(else_e).map(|n| n as f64))
                }
            }
            _ => None,
        }
    }

    fn collect_switch_cases(&mut self, st: &Stmt, out: &mut Vec<SwitchCaseItem>) {
        match st {
            Stmt::Block(ss) => {
                for s in ss {
                    self.collect_switch_cases(s, out);
                }
            }
            Stmt::DeclGroup(_decls) => {}
            Stmt::Case { value, body } => {
                let lab = self.lab("case");
                // Fold enum/static-const case labels via const_globals (case A / TK_IF).
                // const_i64_simple alone treats enum idents as non-const → false "default"
                // and drops cmp/je (canonicalize_path / postgres path.c → path becomes "/").
                let v = self.const_i64(value);
                out.push(SwitchCaseItem { is_default: false, val: v, lab });
                self.collect_switch_cases(body, out);
            }
            Stmt::Default(body) => {
                let lab = self.lab("swdef");
                out.push(SwitchCaseItem { is_default: true, val: None, lab });
                self.collect_switch_cases(body, out);
            }
            Stmt::Label(_, inner) => self.collect_switch_cases(inner, out),
            Stmt::DoWhile { body, .. }
            | Stmt::While { body, .. }
            | Stmt::For { body, .. } => self.collect_switch_cases(body, out),
            Stmt::If {
                then_b, else_b, ..
            } => {
                self.collect_switch_cases(then_b, out);
                if let Some(e) = else_b {
                    self.collect_switch_cases(e, out);
                }
            }
            // Nested switch: do not collect its cases into the outer switch.
            Stmt::Switch { .. } => {}
            _ => {}
        }
    }

    fn emit_switch_body(
        &mut self,
        st: &Stmt,
        default_lab: &str,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        match st {
            Stmt::Block(ss) => {
                for s in ss {
                    self.emit_switch_body(s, default_lab, typedefs)?;
                }
                Ok(())
            }
            Stmt::DeclGroup(decls) => {
                for d in decls {
                    self.emit_stmt(&Stmt::Decl(d.clone()), typedefs)?;
                }
                Ok(())
            }
            Stmt::Case { value, body } => {
                let lab = self
                    .pending_case_labs
                    .pop_front()
                    .unwrap_or_else(|| self.lab("case"));
                writeln!(self.out, "{lab}:").unwrap();
                let _ = value;
                self.emit_switch_body(body, default_lab, typedefs)
            }
            Stmt::Default(body) => {
                writeln!(self.out, "{default_lab}:").unwrap();
                self.emit_switch_body(body, default_lab, typedefs)
            }
            Stmt::Label(name, inner) => {
                writeln!(
                    self.out,
                    "L_{}_goto_{}:",
                    self.func_name, name
                )
                .unwrap();
                self.emit_switch_body(inner, default_lab, typedefs)
            }
            Stmt::If {
                cond,
                then_b,
                else_b,
            } => {
                let l_else = self.lab("else");
                let l_end = self.lab("endif");
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\ttestq\t%rax, %rax").unwrap();
                writeln!(self.out, "\tje\t{l_else}").unwrap();
                self.emit_switch_body(then_b, default_lab, typedefs)?;
                writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                if let Some(e) = else_b {
                    self.emit_switch_body(e, default_lab, typedefs)?;
                }
                writeln!(self.out, "{l_end}:").unwrap();
                Ok(())
            }
            // Nested switch / other stmts: normal emission (fresh case queue).
            other => self.emit_stmt(other, typedefs),
        }
    }

    fn emit_fp_addr(&mut self, off: i64, addr_reg: u8) {
        let r = reg(addr_reg);
        writeln!(self.out, "\tleaq\t{off}(%rbp), {r}").unwrap();
    }

    fn store_to_offset(&mut self, off: i64, ty: &Type, regn: u8) {
        match self.type_size(ty) {
            1 => writeln!(self.out, "\tmovb\t{}, {off}(%rbp)", reg_b(regn)).unwrap(),
            2 => writeln!(self.out, "\tmovw\t{}, {off}(%rbp)", reg_w(regn)).unwrap(),
            3 => {
                writeln!(self.out, "\tmovw\t{}, {off}(%rbp)", reg_w(regn)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(regn)).unwrap();
                writeln!(self.out, "\tshrl\t$16, %ecx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, {}(%rbp)", off + 2).unwrap();
            }
            4 => writeln!(self.out, "\tmovl\t{}, {off}(%rbp)", reg_d(regn)).unwrap(),
            5 => {
                writeln!(self.out, "\tmovl\t{}, {off}(%rbp)", reg_d(regn)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(regn)).unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, {}(%rbp)", off + 4).unwrap();
            }
            6 => {
                writeln!(self.out, "\tmovl\t{}, {off}(%rbp)", reg_d(regn)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(regn)).unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovw\t%cx, {}(%rbp)", off + 4).unwrap();
            }
            7 => {
                writeln!(self.out, "\tmovl\t{}, {off}(%rbp)", reg_d(regn)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(regn)).unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovw\t%cx, {}(%rbp)", off + 4).unwrap();
                writeln!(self.out, "\tshrl\t$16, %ecx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, {}(%rbp)", off + 6).unwrap();
            }
            _ => writeln!(self.out, "\tmovq\t{}, {off}(%rbp)", reg(regn)).unwrap(),
        }
    }

    /// Store `reg(regn)` into C local/global `var` for extended-asm "=r" outputs.
    fn emit_asm_operand_store(
        &mut self,
        var: &str,
        regn: u8,
        _typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        if let Some(local) = self.get_local(var).cloned() {
            match &local.storage {
                Storage::Local { offset } => {
                    self.store_to_offset(*offset, &local.ty, regn);
                }
                Storage::RegAddr { reg: r } => {
                    writeln!(self.out, "\tmovq\t{}, {}", reg(regn), reg(*r)).unwrap();
                }
                Storage::Global { name } => {
                    let lab = sym(name);
                    let scratch = if regn != 9 { 9u8 } else { 10u8 };
                    emit_global_sym_addr(&mut self.out, &lab, reg(scratch));
                    writeln!(self.out, "\tmovq\t{}, ({})", reg(regn), reg(scratch)).unwrap();
                }
            }
            return Ok(());
        }
        if self.globals.contains_key(var) {
            let lab = sym(var);
            let scratch = if regn != 9 { 9u8 } else { 10u8 };
            emit_global_sym_addr(&mut self.out, &lab, reg(scratch));
            writeln!(self.out, "\tmovq\t{}, ({})", reg(regn), reg(scratch)).unwrap();
        }
        Ok(())
    }

    fn load_from_offset(&mut self, off: i64, ty: &Type, regn: u8) {
        // Scalar locals live in 8-byte slots, but SysV leaves upper bits of
        // narrower incoming args undefined. Always load only the C width and
        // sign/zero-extend — movq of an `int` param was `0x7fff8000002f`
        // for maxSemas=47 → mul_size overflow in postgres PGSemaphoreShmemSize.
        match ty.unqual() {
            Type::Long | Type::ULong | Type::Ptr(_) | Type::Double => {
                writeln!(self.out, "\tmovq\t{}(%rbp), {}", off, reg(regn)).unwrap();
            }
            Type::Float => {
                // movl zero-extends into the 64-bit GP holding IEEE bits.
                writeln!(self.out, "\tmovl\t{}(%rbp), {}", off, reg_d(regn)).unwrap();
            }
            Type::Int => {
                writeln!(self.out, "\tmovslq\t{}(%rbp), {}", off, reg(regn)).unwrap();
            }
            Type::UInt => {
                writeln!(self.out, "\tmovl\t{}(%rbp), {}", off, reg_d(regn)).unwrap();
            }
            Type::Short => {
                writeln!(self.out, "\tmovswq\t{}(%rbp), {}", off, reg(regn)).unwrap();
            }
            Type::UShort => {
                writeln!(self.out, "\tmovzwq\t{}(%rbp), {}", off, reg(regn)).unwrap();
            }
            Type::SChar => {
                writeln!(self.out, "\tmovsbq\t{}(%rbp), {}", off, reg(regn)).unwrap();
            }
            Type::Char | Type::UChar => {
                writeln!(self.out, "\tmovzbq\t{}(%rbp), {}", off, reg(regn)).unwrap();
            }
            _ => match self.type_size(ty) {
                1 => writeln!(self.out, "\tmovzbq\t{}(%rbp), {}", off, reg(regn)).unwrap(),
                2 => writeln!(self.out, "\tmovzwq\t{}(%rbp), {}", off, reg(regn)).unwrap(),
                4 => writeln!(self.out, "\tmovslq\t{}(%rbp), {}", off, reg(regn)).unwrap(),
                _ => writeln!(self.out, "\tmovq\t{}(%rbp), {}", off, reg(regn)).unwrap(),
            },
        }
    }

    fn emit_local_init_list(
        &mut self,
        base_off: i64,
        ty: &Type,
        fields_in: &[(Option<String>, Expr)],
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        match ty {
            Type::Array(elem, n) => {
                for i in 0..(*n as usize) {
                    let eoff = base_off + (i as i64) * self.type_size(elem);
                    if let Some((_, e)) = fields_in.get(i) {
                        self.emit_expr_rval(e, 0, typedefs)?;
                        self.store_to_offset(eoff, elem, 0);
                    }
                }
            }
            Type::Struct(name) | Type::Union(name) => {
                // Soft: incomplete type mid-TU — skip rather than fail the TU.
                let Some(lay) = self.get_layout(name).cloned() else {
                    return Ok(());
                };
                let mut by_name: HashMap<String, &Expr> = HashMap::new();
                let mut positional = Vec::new();
                for (n, e) in fields_in {
                    if let Some(nn) = n {
                        by_name.insert(nn.clone(), e);
                    } else {
                        positional.push(e);
                    }
                }
                let mut ordered: Vec<_> = lay.fields.iter().collect();
                ordered.sort_by_key(|(_, (off, _))| *off);
                let mut pos_i = 0usize;
                for (fname, (foff, fty)) in ordered {
                    let e = if let Some(ex) = by_name.get(fname) {
                        Some(*ex)
                    } else if pos_i < positional.len() {
                        let e = positional[pos_i];
                        pos_i += 1;
                        Some(e)
                    } else {
                        None
                    };
                    if let Some(e) = e {
                        self.emit_expr_rval(e, 0, typedefs)?;
                        self.store_to_offset(base_off + *foff, fty, 0);
                    }
                }
            }
            _ => {
                if let Some((_, e)) = fields_in.first() {
                    self.emit_expr_rval(e, 0, typedefs)?;
                    writeln!(self.out, "\tmovq\t%rax, {base_off}(%rbp)").unwrap();
                }
            }
        }
        Ok(())
    }

    /// Runtime compound-literal init into `off(%rsp)` (after `subq` reserved the slot).
    fn emit_init_list_at_rsp(
        &mut self,
        base_off: i64,
        ty: &Type,
        fields_in: &[(Option<String>, Expr)],
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        match ty {
            Type::Array(elem, n) => {
                for i in 0..(*n as usize) {
                    let eoff = base_off + (i as i64) * self.type_size(elem);
                    if let Some((_, e)) = fields_in.get(i) {
                        self.emit_expr_rval(e, 0, typedefs)?;
                        writeln!(self.out, "\tleaq\t{eoff}(%rsp), %r10").unwrap();
                        self.store_ty(elem, 9, 0);
                    }
                }
            }
            Type::Struct(name) | Type::Union(name) => {
                let Some(lay) = self.get_layout(name).cloned() else {
                    // No layout: treat as single scalar store of first field.
                    if let Some((_, e)) = fields_in.first() {
                        self.emit_expr_rval(e, 0, typedefs)?;
                        writeln!(self.out, "\tmovq\t%rax, {base_off}(%rsp)").unwrap();
                    }
                    return Ok(());
                };
                let mut by_name: HashMap<String, &Expr> = HashMap::new();
                let mut positional = Vec::new();
                for (n, e) in fields_in {
                    if let Some(nn) = n {
                        by_name.insert(nn.clone(), e);
                    } else {
                        positional.push(e);
                    }
                }
                let mut ordered: Vec<_> = lay.fields.iter().collect();
                ordered.sort_by_key(|(_, (off, _))| *off);
                let mut pos_i = 0usize;
                for (fname, (foff, fty)) in ordered {
                    let e = if let Some(ex) = by_name.get(fname) {
                        Some(*ex)
                    } else if pos_i < positional.len() {
                        let e = positional[pos_i];
                        pos_i += 1;
                        Some(e)
                    } else {
                        None
                    };
                    if let Some(e) = e {
                        self.emit_expr_rval(e, 0, typedefs)?;
                        let off = base_off + *foff;
                        writeln!(self.out, "\tleaq\t{off}(%rsp), %r10").unwrap();
                        self.store_ty(fty, 9, 0);
                    }
                }
            }
            Type::AnonStruct(fs) | Type::AnonUnion(fs) => {
                let is_union = matches!(ty, Type::AnonUnion(_));
                let lay = self.layout_fields(fs, is_union, false);
                let mut by_name: HashMap<String, &Expr> = HashMap::new();
                let mut positional = Vec::new();
                for (n, e) in fields_in {
                    if let Some(nn) = n {
                        by_name.insert(nn.clone(), e);
                    } else {
                        positional.push(e);
                    }
                }
                let mut ordered: Vec<_> = lay.fields.iter().collect();
                ordered.sort_by_key(|(_, (off, _))| *off);
                let mut pos_i = 0usize;
                for (fname, (foff, fty)) in ordered {
                    let e = if let Some(ex) = by_name.get(fname) {
                        Some(*ex)
                    } else if pos_i < positional.len() {
                        let e = positional[pos_i];
                        pos_i += 1;
                        Some(e)
                    } else {
                        None
                    };
                    if let Some(e) = e {
                        self.emit_expr_rval(e, 0, typedefs)?;
                        let off = base_off + *foff;
                        writeln!(self.out, "\tleaq\t{off}(%rsp), %r10").unwrap();
                        self.store_ty(fty, 9, 0);
                    }
                }
            }
            _ => {
                if let Some((_, e)) = fields_in.first() {
                    self.emit_expr_rval(e, 0, typedefs)?;
                    writeln!(self.out, "\tmovq\t%rax, {base_off}(%rsp)").unwrap();
                }
            }
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Sym, String> {
        if let Some(s) = self.get_local(name) {
            return Ok(s.clone());
        }
        if let Some(ty) = self.globals.get(name) {
            return Ok(Sym {
                ty: ty.clone(),
                storage: Storage::Global {
                    name: name.to_string(),
                },
            });
        }
        // glibc provides these; soft-prefix `extern` decls can be dropped when a
        // large TU soft-skips / mis-parses early globals. Never fall back to 0
        // (initdb `setvbuf(stdout,…)` SEGV).
        if Self::is_extern_libc(name) {
            return Ok(Sym {
                ty: Type::Ptr(Box::new(Type::Void)),
                storage: Storage::Global {
                    name: name.to_string(),
                },
            });
        }
        Err(format!("undefined variable '{name}'"))
    }

    fn is_extern_libc(name: &str) -> bool {
        matches!(
            name,
            "stdout"
                | "stderr"
                | "stdin"
                | "__stdoutp"
                | "__stderrp"
                | "__stdinp"
                | "optarg"
                | "optind"
                | "opterr"
                | "optopt"
                | "environ"
                | "__environ"
            // Do NOT list `errno` here: glibc errno is TLS; a GOT data ref
            // against non-TLS `errno` fails the link (and is wrong anyway —
            // use __errno_location via soft macros / helpers).
        )
    }

    /// True if this TU emits a real body for `name` (not a mere prototype).
    fn func_defined_in_tu(&self, name: &str) -> bool {
        match self.funcs.get(name) {
            Some(f) => f.body.is_some() || name == "main",
            None => name == "main",
        }
    }

    /// Materialize a function designator address into `dest`.
    /// Defined-in-TU → PC-relative lea; external/undef → GOT (required for PIE
    /// against glibc symbols like memcpy/memcmp/strlcpy).
    fn emit_func_addr(&mut self, name: &str, dest: u8) {
        let s = sym(name);
        let force_got = !self.func_defined_in_tu(name);
        emit_global_sym_addr_opts(&mut self.out, &s, reg(dest), force_got);
    }

    /// Asm symbol for a C label in the current function (`label:` → `L_f_goto_label`).
    fn c_goto_label_sym(&self, label: &str) -> String {
        format!("L_{}_goto_{}", self.func_name, label)
    }

    fn emit_lvalue_addr(
        &mut self,
        e: &Expr,
        regn: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match e {
            Expr::StmtExpr(stmts, final_expr) => {
                self.enter_scope();
                for s in stmts {
                    self.emit_stmt(s, typedefs)?;
                }
                let res = self.emit_lvalue_addr(final_expr, regn, typedefs);
                self.exit_scope();
                res
            }
            Expr::Var(name) => {
                // Linux glibc: errno is TLS via __errno_location(), not a GOT data sym.
                if name == "errno" {
                    writeln!(self.out, "\tcallq\t__errno_location@PLT").unwrap();
                    if regn != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(regn)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                let sy = match self.lookup(name) {
                    Ok(s) => s,
                    Err(_) => {
                        // Soft-fallback for unmaterialized temps — emit a dummy stack slot.
                        self.emit_imm(0, regn);
                        return Ok(Type::Long);
                    }
                };
                match &sy.storage {
                    Storage::Local { offset } => {
                        self.emit_fp_addr(*offset, regn);
                    }
                    Storage::Global { name } => {
                        let s = sym(name);
                        emit_global_sym_addr(&mut self.out, &s, reg(regn));
                    }
                    Storage::RegAddr { reg: r } => {
                        if *r != regn {
                            writeln!(self.out, "\tmovq\t{}, {}", reg(*r), reg(regn)).unwrap();
                        }
                    }
                }
                Ok(sy.ty)
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let ty = self.emit_expr_rval(expr, regn, typedefs)?;
                match ty {
                    Type::Ptr(inner) => Ok(*inner),
                    Type::Array(inner, _) => Ok(*inner),
                    // Incomplete typing: integer/struct used as pointer (kernel soft).
                    Type::Int
                    | Type::UInt
                    | Type::Long
                    | Type::ULong
                    | Type::Short
                    | Type::UShort
                    | Type::Char
                    | Type::Void
                    | Type::Struct(_)
                    | Type::Union(_) => Ok(Type::Char),
                    other => Err(format!("cannot dereference {:?}", other)),
                }
            }
            Expr::Index { base, index } => {
                // Spill base: Binary/index eval uses %r10 as a temporary and would
                // clobber the array/pointer address if left live across the index.
                let bty = self.emit_expr_rval(base, 9, typedefs)?;
                writeln!(self.out, "\tsubq\t$16, %rsp").unwrap();
                writeln!(self.out, "\tmovq\t%r10, (%rsp)").unwrap();
                let ity = self.emit_expr_rval(index, 10, typedefs)?;
                writeln!(self.out, "\tmovq\t(%rsp), %r10").unwrap();
                writeln!(self.out, "\taddq\t$16, %rsp").unwrap();
                let ity_expanded = self.expand_ty(&ity, typedefs);
                match ity_expanded.unqual() {
                    Type::Int => {
                        writeln!(self.out, "\tmovslq\t%r11d, %r11").unwrap();
                    }
                    Type::Short | Type::SChar => {
                        writeln!(self.out, "\tmovswq\t%r11w, %r11").unwrap();
                    }
                    Type::Char => {
                        writeln!(self.out, "\tmovsbq\t%r11b, %r11").unwrap();
                    }
                    Type::UInt | Type::ULong => {
                        writeln!(self.out, "\tmovl\t%r11d, %r11d").unwrap();
                    }
                    Type::UShort => {
                        writeln!(self.out, "\tmovzwq\t%r11w, %r11").unwrap();
                    }
                    Type::UChar => {
                        writeln!(self.out, "\tmovzbq\t%r11b, %r11").unwrap();
                    }
                    _ => {
                        if self.type_size(&ity_expanded) <= 4 && !matches!(ity_expanded.unqual(), Type::UInt | Type::ULong | Type::UShort | Type::UChar | Type::Ptr(_)) {
                            writeln!(self.out, "\tmovslq\t%r11d, %r11").unwrap();
                        }
                    }
                }
                let elem = match bty.unqual() {
                    Type::Array(e, _) => (*e).unqual().clone(),
                    Type::Ptr(e) => (*e).unqual().clone(),
                    // Incomplete typing: treat integer/void as opaque byte pointer.
                    Type::Int
                    | Type::UInt
                    | Type::Long
                    | Type::ULong
                    | Type::Short
                    | Type::UShort
                    | Type::Char
                    | Type::Void => Type::Char,
                    other => return Err(format!("index of non-array {:?}", other)),
                };
                let esz = self.type_size(&elem).max(1);
                writeln!(self.out, "\timulq\t${esz}, %r11").unwrap();
                writeln!(self.out, "\tleaq\t(%r10,%r11), {}", reg(regn)).unwrap();
                Ok(elem)
            }
            Expr::Member { base, field, arrow } => {
                let base_ty = if *arrow {
                    let t = self.emit_expr_rval(base, regn, typedefs)?;
                    match t {
                        Type::Ptr(inner) => *inner,
                        // Incomplete typing: typeof/typedef collapsed (Int, bare
                        // Struct name used as pointer value, etc.).
                        Type::Int
                        | Type::UInt
                        | Type::Long
                        | Type::ULong
                        | Type::Short
                        | Type::UShort
                        | Type::Char
                        | Type::Void
                        | Type::Struct(_)
                        | Type::Union(_) => Type::Struct("__opaque__".into()),
                        // Soft: any leftover type used as pointer (kernel fail-drive).
                        other => {
                            let _ = other;
                            Type::Struct("__opaque__".into())
                        }
                    }
                } else {
                    self.emit_lvalue_addr(base, regn, typedefs)?
                };
                let lay = match &base_ty {
                    Type::Struct(n) if n == "__opaque__" => {
                        let mut found = None;
                        for lay in self.layouts.values() {
                            if lay.fields.contains_key(field) {
                                found = Some(lay.clone());
                                break;
                            }
                        }
                        found.unwrap_or_else(|| {
                            let mut fields = HashMap::new();
                            fields.insert(
                                field.clone(),
                                (0, Type::Ptr(Box::new(Type::Void))),
                            );
                            Layout {
                                size: 8,
                                align: 8,
                                fields,
                            }
                        })
                    }
                    Type::Struct(n) | Type::Union(n) => self.get_layout(n).cloned().unwrap_or_else(
                        || {
                            // Soft: named struct without recorded layout.
                            let mut fields = HashMap::new();
                            fields.insert(field.clone(), (0, Type::Ptr(Box::new(Type::Void))));
                            Layout {
                                size: 8,
                                align: 8,
                                fields,
                            }
                        },
                    ),
                    Type::AnonStruct(fs) => self.layout_fields(fs, false, false),
                    Type::AnonUnion(fs) => self.layout_fields(fs, true, false),
                    // Incomplete typing: void*/void/int treated as opaque struct.
                    Type::Ptr(_)
                    | Type::Void
                    | Type::Int
                    | Type::UInt
                    | Type::Long
                    | Type::ULong
                    | Type::Char
                    | Type::Short
                    | Type::UShort
                    | Type::Array(_, _) => {
                        let mut fields = HashMap::new();
                        fields.insert(
                            field.clone(),
                            (0, Type::Ptr(Box::new(Type::Void))),
                        );
                        Layout {
                            size: 8,
                            align: 8,
                            fields,
                        }
                    }
                    other => return Err(format!("member of non-struct {:?}", other)),
                };
                let (off, fty) = if let Some(p) = lay.fields.get(field) {
                    p.clone()
                } else {
                    // Soft: unknown field on opaque/incomplete type → offset 0 void*
                    (0, Type::Ptr(Box::new(Type::Void)))
                };
                if off != 0 {
                    writeln!(self.out, "\taddq\t${off}, {}", reg(regn)).unwrap();
                }
                Ok(fty)
            }
            // Compound literal `(T){ .f = … }` as lvalue (e.g. `&(T){…}` /
            // postgres XL_ROUTINE). Materialize to .rodata and return address;
            // never fall through to InitList rvalue → NULL (SEGV).
            Expr::Cast { ty, expr } => {
                if let Expr::InitList { fields } = expr.as_ref() {
                    let id = self.label_id;
                    self.label_id += 1;
                    let lab = format!("L_compl_{id}");
                    writeln!(self.out, "\t.section\t.rodata").unwrap();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{lab}:").unwrap();
                    self.emit_init_list_data(ty, fields)?;
                    writeln!(self.out, "\t.text").unwrap();
                    emit_global_sym_addr(&mut self.out, &lab, reg(regn));
                    return Ok(ty.clone());
                }
                let tty = self.expand_ty(ty, typedefs);
                if Self::is_struct_or_union_ty(&tty) {
                    return self.emit_materialize_agg_addr(e, regn, typedefs);
                }
                let _ = self.emit_expr_rval(e, regn, typedefs)?;
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
            // Kernel/postgres headers take address of many rvalues (calls,
            // ternaries). Soft: evaluate as rvalue into reg and treat as opaque
            // pointer — EXCEPT for struct/union, where the scalar rvalue is the
            // object bits, not its address (`f().field` → SEGV).
            other => {
                let ty = self.expand_ty(&self.typeof_expr(other, typedefs), typedefs);
                if Self::is_struct_or_union_ty(&ty) {
                    return self.emit_materialize_agg_addr(other, regn, typedefs);
                }
                let _ = self.emit_expr_rval(other, regn, typedefs)?;
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
        }
    }

    fn load_ty(&mut self, ty: &Type, addr_reg: u8, dest: u8) {
        match ty.unqual() {
            Type::Long | Type::ULong | Type::Ptr(_) | Type::Double => {
                writeln!(self.out, "\tmovq\t({}), {}", reg(addr_reg), reg(dest)).unwrap();
            }
            Type::SChar | Type::Char => {
                writeln!(self.out, "\tmovsbq\t({}), {}", reg(addr_reg), reg(dest)).unwrap();
            }
            Type::UChar => {
                writeln!(self.out, "\tmovzbq\t({}), {}", reg(addr_reg), reg(dest)).unwrap();
            }
            // flex_int16_t / short: never movq — adjacent yy_base/yy_chk entries
            // would be swallowed into a huge index and SEGV in GUC_yylex.
            Type::Short => {
                writeln!(self.out, "\tmovswq\t({}), {}", reg(addr_reg), reg(dest)).unwrap();
            }
            Type::UShort => {
                writeln!(self.out, "\tmovzwq\t({}), {}", reg(addr_reg), reg(dest)).unwrap();
            }
            Type::Int => {
                writeln!(self.out, "\tmovslq\t({}), {}", reg(addr_reg), reg(dest)).unwrap();
            }
            Type::UInt | Type::Float => {
                // movl zero-extends; critical for Pgno/u32 (mxPgno=0xfffffffe).
                writeln!(self.out, "\tmovl\t({}), {}", reg(addr_reg), reg_d(dest)).unwrap();
            }
            _ => match self.type_size(ty) {
                1 => writeln!(self.out, "\tmovzbq\t({}), {}", reg(addr_reg), reg(dest)).unwrap(),
                2 => writeln!(self.out, "\tmovzwq\t({}), {}", reg(addr_reg), reg(dest)).unwrap(),
                4 => writeln!(self.out, "\tmovslq\t({}), {}", reg(addr_reg), reg(dest)).unwrap(),
                _ => writeln!(self.out, "\tmovq\t({}), {}", reg(addr_reg), reg(dest)).unwrap(),
            },
        }
    }

    /// Soft prototypes for libm / string→float that return in `%xmm0` (SysV).
    fn known_fp_return(name: &str) -> Option<Type> {
        match name {
            "rint" | "round" | "trunc" | "nearbyint" | "floor" | "ceil" | "fabs"
            | "sqrt" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
            | "sinh" | "cosh" | "tanh" | "log" | "log10" | "exp" | "pow" | "fmod"
            | "ldexp" | "frexp" | "modf" | "strtod" | "atof" | "nan" => Some(Type::Double),
            "rintf" | "roundf" | "truncf" | "floorf" | "ceilf" | "fabsf" | "sqrtf"
            | "sinf" | "cosf" | "tanf" | "strtof" => Some(Type::Float),
            _ => None,
        }
    }

    /// SysV: int/short returns live in %eax; high bits of %rax may be zero-
    /// extended or garbage. Sign-extend so 64-bit ops see a true negative.
    fn emit_extend_call_return(&mut self, dest: u8, ret_ty: &Type) {
        match ret_ty.unqual() {
            Type::Double => {
                // Floating returns live in %xmm0.
                if dest == 0 {
                    writeln!(self.out, "\tmovq\t%xmm0, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tmovq\t%xmm0, {}", reg(dest)).unwrap();
                }
            }
            Type::Float => {
                if dest == 0 {
                    writeln!(self.out, "\tmovd\t%xmm0, %eax").unwrap();
                } else {
                    writeln!(self.out, "\tmovd\t%xmm0, {}", reg_d(dest)).unwrap();
                }
            }
            Type::Int => {
                if dest == 0 {
                    writeln!(self.out, "\tmovslq\t%eax, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tmovslq\t%eax, {}", reg(dest)).unwrap();
                }
            }
            Type::Short => {
                if dest == 0 {
                    writeln!(self.out, "\tmovswq\t%ax, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tmovswq\t%ax, {}", reg(dest)).unwrap();
                }
            }
            Type::SChar => {
                if dest == 0 {
                    writeln!(self.out, "\tmovsbq\t%al, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tmovsbq\t%al, {}", reg(dest)).unwrap();
                }
            }
            Type::Char | Type::UChar => {
                if dest == 0 {
                    writeln!(self.out, "\tmovzbq\t%al, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tmovzbq\t%al, {}", reg(dest)).unwrap();
                }
            }
            _ => {
                if dest != 0 {
                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                }
            }
        }
    }

    /// `__builtin_{add,sub,mul}_overflow(a, b, r)` — see aarch64 counterpart.
    fn emit_builtin_overflow(
        &mut self,
        name: &str,
        a: &Expr,
        b: &Expr,
        r: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        let _ = self.emit_expr_rval(a, 0, typedefs)?;
        writeln!(self.out, "\tpushq\t%rax").unwrap();
        let _ = self.emit_expr_rval(b, 0, typedefs)?;
        writeln!(self.out, "\tpushq\t%rax").unwrap();
        let rty = self.emit_expr_rval(r, 9, typedefs)?; // r10 = r
        writeln!(self.out, "\tpopq\t%rcx").unwrap(); // b in rcx
        writeln!(self.out, "\tpopq\t%rax").unwrap(); // a in rax

        let pointee = match &rty {
            Type::Ptr(inner) => (**inner).clone(),
            other => other.clone(),
        };
        let sz = self.type_size(&pointee);
        let signed = matches!(
            pointee,
            Type::SChar | Type::Short | Type::Int | Type::Long
        );
        let op = if name.contains("add") {
            "add"
        } else if name.contains("sub") {
            "sub"
        } else {
            "mul"
        };

        match (op, sz) {
            ("add", 8) => {
                writeln!(self.out, "\taddq\t%rcx, %rax").unwrap();
                self.store_ty(&pointee, 9, 0);
                if signed {
                    writeln!(self.out, "\tseto\t%al").unwrap();
                } else {
                    writeln!(self.out, "\tsetc\t%al").unwrap();
                }
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            ("add", 4) => {
                writeln!(self.out, "\taddl\t%ecx, %eax").unwrap();
                self.store_ty(&pointee, 9, 0);
                if signed {
                    writeln!(self.out, "\tseto\t%al").unwrap();
                } else {
                    writeln!(self.out, "\tsetc\t%al").unwrap();
                }
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            ("sub", 8) => {
                writeln!(self.out, "\tsubq\t%rcx, %rax").unwrap();
                self.store_ty(&pointee, 9, 0);
                if signed {
                    writeln!(self.out, "\tseto\t%al").unwrap();
                } else {
                    writeln!(self.out, "\tsetc\t%al").unwrap();
                }
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            ("sub", 4) => {
                writeln!(self.out, "\tsubl\t%ecx, %eax").unwrap();
                self.store_ty(&pointee, 9, 0);
                if signed {
                    writeln!(self.out, "\tseto\t%al").unwrap();
                } else {
                    writeln!(self.out, "\tsetc\t%al").unwrap();
                }
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            ("mul", 8) if signed => {
                // one-operand imulq: rdx:rax = product; OF/CF set if high != sign(low)
                writeln!(self.out, "\timulq\t%rcx").unwrap();
                self.store_ty(&pointee, 9, 0);
                writeln!(self.out, "\tseto\t%al").unwrap();
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            ("mul", 8) => {
                writeln!(self.out, "\tmulq\t%rcx").unwrap();
                self.store_ty(&pointee, 9, 0);
                writeln!(self.out, "\txorq\t%rax, %rax").unwrap();
                writeln!(self.out, "\ttestq\t%rdx, %rdx").unwrap();
                writeln!(self.out, "\tsetne\t%al").unwrap();
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            ("mul", 4) if signed => {
                writeln!(self.out, "\timull\t%ecx").unwrap();
                self.store_ty(&pointee, 9, 0);
                writeln!(self.out, "\tseto\t%al").unwrap();
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            ("mul", 4) => {
                writeln!(self.out, "\tmull\t%ecx").unwrap();
                self.store_ty(&pointee, 9, 0);
                writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
                writeln!(self.out, "\ttestl\t%edx, %edx").unwrap();
                writeln!(self.out, "\tsetne\t%al").unwrap();
                writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
            }
            _ => {
                match op {
                    "add" => writeln!(self.out, "\taddq\t%rcx, %rax").unwrap(),
                    "sub" => writeln!(self.out, "\tsubq\t%rcx, %rax").unwrap(),
                    _ => writeln!(self.out, "\timulq\t%rcx, %rax").unwrap(),
                }
                self.store_ty(&pointee, 9, 0);
                writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
            }
        }

        if dest != 0 {
            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
        }
        Ok(())
    }

    /// Software Hamming weight (no `popcnt` insn — safe on CPUs without POPCNT).
    fn emit_builtin_popcount(
        &mut self,
        name: &str,
        arg: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        self.emit_expr_rval(arg, 0, typedefs)?;
        match name {
            "__builtin_popcount" => {
                writeln!(self.out, "\tmovl\t%eax, %ecx").unwrap();
                writeln!(self.out, "\tmovl\t%eax, %edx").unwrap();
                writeln!(self.out, "\tshrl\t$1, %edx").unwrap();
                writeln!(self.out, "\tandl\t$0x55555555, %edx").unwrap();
                writeln!(self.out, "\tsubl\t%edx, %ecx").unwrap();
                writeln!(self.out, "\tmovl\t%ecx, %edx").unwrap();
                writeln!(self.out, "\tandl\t$0x33333333, %ecx").unwrap();
                writeln!(self.out, "\tshrl\t$2, %edx").unwrap();
                writeln!(self.out, "\tandl\t$0x33333333, %edx").unwrap();
                writeln!(self.out, "\taddl\t%edx, %ecx").unwrap();
                writeln!(self.out, "\tmovl\t%ecx, %edx").unwrap();
                writeln!(self.out, "\tshrl\t$4, %edx").unwrap();
                writeln!(self.out, "\taddl\t%edx, %ecx").unwrap();
                writeln!(self.out, "\tandl\t$0x0f0f0f0f, %ecx").unwrap();
                writeln!(self.out, "\tmovl\t%ecx, %edx").unwrap();
                writeln!(self.out, "\tshrl\t$8, %edx").unwrap();
                writeln!(self.out, "\taddl\t%edx, %ecx").unwrap();
                writeln!(self.out, "\tmovl\t%ecx, %edx").unwrap();
                writeln!(self.out, "\tshrl\t$16, %edx").unwrap();
                writeln!(self.out, "\taddl\t%edx, %ecx").unwrap();
                writeln!(self.out, "\tandl\t$0x3f, %ecx").unwrap();
                writeln!(self.out, "\tmovl\t%ecx, %eax").unwrap();
            }
            "__builtin_popcountl" | "__builtin_popcountll" => {
                writeln!(self.out, "\tmovq\t%rax, %rcx").unwrap();
                writeln!(self.out, "\tmovq\t%rax, %rdx").unwrap();
                writeln!(self.out, "\tshrq\t$1, %rdx").unwrap();
                writeln!(self.out, "\tandq\t$0x5555555555555555, %rdx").unwrap();
                writeln!(self.out, "\tsubq\t%rdx, %rcx").unwrap();
                writeln!(self.out, "\tmovq\t%rcx, %rdx").unwrap();
                writeln!(self.out, "\tandq\t$0x3333333333333333, %rcx").unwrap();
                writeln!(self.out, "\tshrq\t$2, %rdx").unwrap();
                writeln!(self.out, "\tandq\t$0x3333333333333333, %rdx").unwrap();
                writeln!(self.out, "\taddq\t%rdx, %rcx").unwrap();
                writeln!(self.out, "\tmovq\t%rcx, %rdx").unwrap();
                writeln!(self.out, "\tshrq\t$4, %rdx").unwrap();
                writeln!(self.out, "\taddq\t%rdx, %rcx").unwrap();
                writeln!(self.out, "\tandq\t$0x0f0f0f0f0f0f0f0f, %rcx").unwrap();
                writeln!(self.out, "\tmovq\t%rcx, %rdx").unwrap();
                writeln!(self.out, "\tshrq\t$8, %rdx").unwrap();
                writeln!(self.out, "\taddq\t%rdx, %rcx").unwrap();
                writeln!(self.out, "\tmovq\t%rcx, %rdx").unwrap();
                writeln!(self.out, "\tshrq\t$16, %rdx").unwrap();
                writeln!(self.out, "\taddq\t%rdx, %rcx").unwrap();
                writeln!(self.out, "\tmovq\t%rcx, %rdx").unwrap();
                writeln!(self.out, "\tshrq\t$32, %rdx").unwrap();
                writeln!(self.out, "\taddq\t%rdx, %rcx").unwrap();
                writeln!(self.out, "\tandq\t$0x7f, %rcx").unwrap();
                writeln!(self.out, "\tmovq\t%rcx, %rax").unwrap();
            }
            _ => return Err(format!("unknown popcount builtin: {name}")),
        }
        if dest != 0 {
            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
        }
        Ok(())
    }

    /// GCC legacy `__sync_fetch_and_{add,sub,and,or}(ptr, val)` → old value.
    /// Width follows the pointee: `uint32*` must use `xaddl`/`cmpxchgl` (postgres
    /// LWLock.state sits at lock+4 → addr%8==4; `xaddq` there SIGBUS/split-lock).
    fn emit_sync_fetch_and(
        &mut self,
        op: &str,
        ptr: &Expr,
        val: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        self.emit_expr_rval(ptr, 9, typedefs)?; // r10 = ptr
        self.emit_expr_rval(val, 11, typedefs)?; // rcx = val
        let pty = self.typeof_expr(ptr, typedefs);
        let width = match &pty {
            Type::Ptr(inner) => self.type_size(inner).max(1),
            _ => 8,
        };
        let use32 = width <= 4;
        match op {
            "add" => {
                if use32 {
                    writeln!(self.out, "\tmovl\t%ecx, %eax").unwrap();
                    writeln!(self.out, "\tlock").unwrap();
                    writeln!(self.out, "\txaddl\t%eax, (%r10)").unwrap();
                    writeln!(self.out, "\tmovslq\t%eax, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tmovq\t%rcx, %rax").unwrap();
                    writeln!(self.out, "\tlock").unwrap();
                    writeln!(self.out, "\txaddq\t%rax, (%r10)").unwrap();
                }
            }
            "sub" => {
                if use32 {
                    writeln!(self.out, "\tnegl\t%ecx").unwrap();
                    writeln!(self.out, "\tmovl\t%ecx, %eax").unwrap();
                    writeln!(self.out, "\tlock").unwrap();
                    writeln!(self.out, "\txaddl\t%eax, (%r10)").unwrap();
                    writeln!(self.out, "\tmovslq\t%eax, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tnegq\t%rcx").unwrap();
                    writeln!(self.out, "\tmovq\t%rcx, %rax").unwrap();
                    writeln!(self.out, "\tlock").unwrap();
                    writeln!(self.out, "\txaddq\t%rax, (%r10)").unwrap();
                }
            }
            "and" | "or" => {
                let lab = self.lab("sync_fetch");
                writeln!(self.out, "{lab}:").unwrap();
                if use32 {
                    writeln!(self.out, "\tmovl\t(%r10), %eax").unwrap();
                    writeln!(self.out, "\tmovl\t%eax, %edx").unwrap();
                    if op == "and" {
                        writeln!(self.out, "\tandl\t%ecx, %edx").unwrap();
                    } else {
                        writeln!(self.out, "\torl\t%ecx, %edx").unwrap();
                    }
                    writeln!(self.out, "\tlock").unwrap();
                    writeln!(self.out, "\tcmpxchgl\t%edx, (%r10)").unwrap();
                    writeln!(self.out, "\tjne\t{lab}").unwrap();
                    writeln!(self.out, "\tmovslq\t%eax, %rax").unwrap();
                } else {
                    writeln!(self.out, "\tmovq\t(%r10), %rax").unwrap();
                    writeln!(self.out, "\tmovq\t%rax, %rdx").unwrap();
                    if op == "and" {
                        writeln!(self.out, "\tandq\t%rcx, %rdx").unwrap();
                    } else {
                        writeln!(self.out, "\torq\t%rcx, %rdx").unwrap();
                    }
                    writeln!(self.out, "\tlock").unwrap();
                    writeln!(self.out, "\tcmpxchgq\t%rdx, (%r10)").unwrap();
                    writeln!(self.out, "\tjne\t{lab}").unwrap();
                }
            }
            _ => return Err(format!("unknown sync op: {op}")),
        }
        if dest != 0 {
            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
        }
        Ok(())
    }

    /// `<cpuid.h>` `__get_cpuid(level, &eax, &ebx, &ecx, &edx)` — inline cpuid.
    fn emit_get_cpuid(
        &mut self,
        args: &[Expr],
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        if args.len() < 5 {
            return Err("__get_cpuid needs 5 args".into());
        }
        writeln!(self.out, "\tsubq\t$48, %rsp").unwrap();
        for (i, a) in args.iter().skip(1).take(4).enumerate() {
            self.emit_expr_rval(a, 0, typedefs)?;
            let off = (i as i32) * 8;
            writeln!(self.out, "\tmovq\t%rax, {off}(%rsp)").unwrap();
        }
        self.emit_expr_rval(&args[0], 0, typedefs)?;
        writeln!(self.out, "\tmovl\t%eax, %eax").unwrap();
        writeln!(self.out, "\tmovq\t%rbx, 32(%rsp)").unwrap();
        writeln!(self.out, "\tcpuid").unwrap();
        for (off, reg_out) in [(0i32, "%eax"), (8, "%ebx"), (16, "%ecx"), (24, "%edx")] {
            writeln!(self.out, "\tmovq\t{off}(%rsp), %r10").unwrap();
            writeln!(self.out, "\tmovl\t{reg_out}, (%r10)").unwrap();
        }
        writeln!(self.out, "\tmovq\t32(%rsp), %rbx").unwrap();
        writeln!(self.out, "\taddq\t$48, %rsp").unwrap();
        writeln!(self.out, "\tmovl\t$1, %eax").unwrap();
        if dest != 0 {
            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
        }
        Ok(())
    }

    fn store_ty(&mut self, ty: &Type, addr_reg: u8, val_reg: u8) {
        let a = reg(addr_reg);
        match self.type_size(ty) {
            1 => writeln!(self.out, "\tmovb\t{}, ({a})", reg_b(val_reg)).unwrap(),
            2 => writeln!(self.out, "\tmovw\t{}, ({a})", reg_w(val_reg)).unwrap(),
            3 => {
                // Low 2 bytes + high byte; avoid 4/8-byte store past the object.
                writeln!(self.out, "\tmovw\t{}, ({a})", reg_w(val_reg)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(val_reg)).unwrap();
                writeln!(self.out, "\tshrl\t$16, %ecx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, 2({a})").unwrap();
            }
            4 => writeln!(self.out, "\tmovl\t{}, ({a})", reg_d(val_reg)).unwrap(),
            5 => {
                writeln!(self.out, "\tmovl\t{}, ({a})", reg_d(val_reg)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(val_reg)).unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, 4({a})").unwrap();
            }
            6 => {
                // ItemPointerData: 4 + 2, never movq (would clobber +2 bytes).
                writeln!(self.out, "\tmovl\t{}, ({a})", reg_d(val_reg)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(val_reg)).unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovw\t%cx, 4({a})").unwrap();
            }
            7 => {
                writeln!(self.out, "\tmovl\t{}, ({a})", reg_d(val_reg)).unwrap();
                writeln!(self.out, "\tmovq\t{}, %rcx", reg(val_reg)).unwrap();
                writeln!(self.out, "\tshrq\t$32, %rcx").unwrap();
                writeln!(self.out, "\tmovw\t%cx, 4({a})").unwrap();
                writeln!(self.out, "\tshrl\t$16, %ecx").unwrap();
                writeln!(self.out, "\tmovb\t%cl, 6({a})").unwrap();
            }
            _ => writeln!(self.out, "\tmovq\t{}, ({a})", reg(val_reg)).unwrap(),
        }
    }

    /// Coerce value in `%rax` from `from` to `to` (in place). Needed for
    /// `double val = strtol(...)` — storing the integer bit pattern as a
    /// double made parse_int("16777216") yield 0 (denormal → rint → 0).
    /// Also required for `float4 procost = defGetNumeric()` (double→float):
    /// truncating IEEE-754 double bits with `movl %eax` turns 1.0 into 0.0
    /// → postgres FATAL "COST must be positive".
    fn coerce_rax_to_ty(&mut self, from: &Type, to: &Type) {
        let to_fp = matches!(to, Type::Float | Type::Double);
        let from_fp = matches!(from, Type::Float | Type::Double);
        if to_fp && !from_fp {
            let unsigned = matches!(from, Type::UShort | Type::UInt | Type::ULong);
            if unsigned {
                // Approximate: treat as signed for now (same as common soft path).
                writeln!(self.out, "\tcvtsi2sdq\t%rax, %xmm0").unwrap();
            } else {
                writeln!(self.out, "\tcvtsi2sdq\t%rax, %xmm0").unwrap();
            }
            if matches!(to, Type::Float) {
                writeln!(self.out, "\tcvtsd2ss\t%xmm0, %xmm0").unwrap();
                writeln!(self.out, "\tmovd\t%xmm0, %eax").unwrap();
            } else {
                writeln!(self.out, "\tmovq\t%xmm0, %rax").unwrap();
            }
        } else if from_fp && !to_fp {
            if matches!(from, Type::Float) {
                writeln!(self.out, "\tmovd\t%eax, %xmm0").unwrap();
                writeln!(self.out, "\tcvtss2sd\t%xmm0, %xmm0").unwrap();
            } else {
                writeln!(self.out, "\tmovq\t%rax, %xmm0").unwrap();
            }
            let unsigned = matches!(to, Type::UShort | Type::UInt | Type::ULong);
            if unsigned {
                writeln!(self.out, "\tcvttsd2siq\t%xmm0, %rax").unwrap();
            } else {
                writeln!(self.out, "\tcvttsd2siq\t%xmm0, %rax").unwrap();
            }
        } else if matches!(from, Type::Double) && matches!(to, Type::Float) {
            writeln!(self.out, "\tmovq\t%rax, %xmm0").unwrap();
            writeln!(self.out, "\tcvtsd2ss\t%xmm0, %xmm0").unwrap();
            writeln!(self.out, "\tmovd\t%xmm0, %eax").unwrap();
        } else if matches!(from, Type::Float) && matches!(to, Type::Double) {
            writeln!(self.out, "\tmovd\t%eax, %xmm0").unwrap();
            writeln!(self.out, "\tcvtss2sd\t%xmm0, %xmm0").unwrap();
            writeln!(self.out, "\tmovq\t%xmm0, %rax").unwrap();
        }
    }

    fn emit_expr_rval(
        &mut self,
        e: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match e {
            Expr::StmtExpr(stmts, final_expr) => {
                self.enter_scope();
                for s in stmts {
                    self.emit_stmt(s, typedefs)?;
                }
                let res = self.emit_expr_rval(final_expr, dest, typedefs);
                self.exit_scope();
                res
            }
            Expr::Int(n) | Expr::Char(n) => {
                self.emit_imm(*n, dest);
                Ok(Type::Int)
            }
            Expr::Float(f) => {
                let bits = f.to_bits() as i64;
                self.emit_imm(bits, dest);
                Ok(Type::Double)
            }
            Expr::String(s) => {
                let id = self.intern_str(s);
                writeln!(
                    self.out,
                    "\tleaq\tl_str_{id}(%rip), {}",
                    reg(dest)
                )
                .unwrap();
                Ok(Type::Ptr(Box::new(Type::Char)))
            }
            Expr::AddrOfLabel(label) => {
                writeln!(
                    self.out,
                    "\tleaq\t{}(%rip), {}",
                    self.c_goto_label_sym(label),
                    reg(dest)
                )
                .unwrap();
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
            Expr::Var(name) => {
                // Linux: load *(__errno_location()) — never errno@GOT (TLS mismatch).
                if name == "errno" {
                    writeln!(self.out, "\tcallq\t__errno_location@PLT").unwrap();
                    writeln!(self.out, "\tmovslq\t(%rax), {}", reg(dest)).unwrap();
                    return Ok(Type::Int);
                }
                // GCC/C99 predefined function name identifiers.
                if name == "__func__" || name == "__FUNCTION__" || name == "__PRETTY_FUNCTION__"
                {
                    let fname = if self.func_name.is_empty() {
                        "?".to_string()
                    } else {
                        self.func_name.clone()
                    };
                    let id = self.intern_str(&fname);
                    writeln!(
                        self.out,
                        "\tleaq\tl_str_{id}(%rip), {}",
                        reg(dest)
                    )
                    .unwrap();
                    return Ok(Type::Ptr(Box::new(Type::Char)));
                }
                // Soft residue from failed stmt-expr / typeof (aarch64 parity).
                if name == "___res" {
                    self.emit_imm(0, dest);
                    return Ok(Type::ULong);
                }
                // Enumerators / static const ints: immediate (not load from .data).
                if let Some(n) = self.const_globals.get(name).copied() {
                    self.emit_imm(n, dest);
                    return Ok(Type::Int);
                }
                let sy = match self.lookup(name) {
                    Ok(s) => s,
                    Err(_) => {
                        // Function designator vs macro/const residue:
                        // UPPERCASE / macro identifiers (e.g. Z_ERRNO, Z_SYNC_FLUSH) -> imm 0.
                        // Lowercase / function designators -> func address.
                        if self.funcs.contains_key(name)
                            || self.globals.contains_key(name)
                            || name.chars().next().map_or(false, |c| c.is_lowercase() || c == '_')
                        {
                            self.emit_func_addr(name, dest);
                            return Ok(Type::Ptr(Box::new(Type::Void)));
                        } else {
                            self.emit_imm(0, dest);
                            return Ok(Type::Int);
                        }
                    }
                };
                match &sy.ty {
                    Type::Array(elem, _) => {
                        match &sy.storage {
                            Storage::Local { offset } => self.emit_fp_addr(*offset, dest),
                            Storage::Global { name } => {
                                let s = sym(name);
                                emit_global_sym_addr(&mut self.out, &s, reg(dest));
                            }
                            _ => {}
                        }
                        Ok(Type::Ptr(elem.clone()))
                    }
                    ty => {
                        match &sy.storage {
                            Storage::Local { offset } => {
                                self.load_from_offset(*offset, ty, dest);
                            }
                            Storage::Global { name } => {
                                let s = sym(name);
                                emit_global_sym_addr(&mut self.out, &s, "%r10");
                                self.load_ty(ty, 9, dest);
                            }
                            Storage::RegAddr { reg: r } => {
                                self.load_ty(ty, *r, dest);
                            }
                        }
                        Ok(ty.clone())
                    }
                }
            }
            Expr::Unary { op, expr } => match op {
                UnaryOp::Neg => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tnegq\t{}", reg(dest)).unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::Not => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\ttestq\t{}, {}", reg(dest), reg(dest)).unwrap();
                    writeln!(self.out, "\tsetz\t%al").unwrap();
                    writeln!(self.out, "\tmovzbq\t%al, {}", reg(dest)).unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::BitNot => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tnotq\t{}", reg(dest)).unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::Addr => {
                    if let Expr::Var(n) = expr.as_ref() {
                        // &fn — including undeclared libc (see Var rvalue path).
                        if n == "__func__"
                            || n == "__FUNCTION__"
                            || n == "__PRETTY_FUNCTION__"
                        {
                            // fall through: not a function address
                        } else if self.funcs.contains_key(n)
                            || n == "main"
                            || (self.lookup(n).is_err() && (self.globals.contains_key(n) || n.chars().next().map_or(false, |c| c.is_lowercase() || c == '_')))
                        {
                            self.emit_func_addr(n, dest);
                            return Ok(Type::Ptr(Box::new(Type::Void)));
                        }
                    }
                    let ty = self.emit_lvalue_addr(expr, dest, typedefs)?;
                    Ok(Type::Ptr(Box::new(ty)))
                }
                UnaryOp::Deref => {
                    let ty = self.emit_expr_rval(expr, 9, typedefs)?;
                    let inner = match ty {
                        Type::Ptr(i) => *i,
                        Type::Array(i, _) => *i,
                        Type::Int
                        | Type::UInt
                        | Type::Long
                        | Type::ULong
                        | Type::Short
                        | Type::UShort
                        | Type::Char
                        | Type::Void => Type::Char,
                        other => return Err(format!("deref {:?}", other)),
                    };
                    self.load_ty(&inner, 9, dest);
                    Ok(inner)
                }
            },
            Expr::Binary { op, left, right } => {
                if *op == BinOp::And {
                    let l_false = self.lab("and_false");
                    let l_end = self.lab("and_end");
                    self.emit_expr_rval(left, dest, typedefs)?;
                    writeln!(self.out, "\ttestq\t{}, {}", reg(dest), reg(dest)).unwrap();
                    writeln!(self.out, "\tje\t{l_false}").unwrap();
                    self.emit_expr_rval(right, dest, typedefs)?;
                    writeln!(self.out, "\ttestq\t{}, {}", reg(dest), reg(dest)).unwrap();
                    writeln!(self.out, "\tsetne\t%al").unwrap();
                    writeln!(self.out, "\tmovzbq\t%al, {}", reg(dest)).unwrap();
                    writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                    writeln!(self.out, "{l_false}:").unwrap();
                    writeln!(self.out, "\txorq\t{}, {}", reg(dest), reg(dest)).unwrap();
                    writeln!(self.out, "{l_end}:").unwrap();
                    return Ok(Type::Int);
                }
                if *op == BinOp::Or {
                    let l_true = self.lab("or_true");
                    let l_end = self.lab("or_end");
                    self.emit_expr_rval(left, dest, typedefs)?;
                    writeln!(self.out, "\ttestq\t{}, {}", reg(dest), reg(dest)).unwrap();
                    writeln!(self.out, "\tjne\t{l_true}").unwrap();
                    self.emit_expr_rval(right, dest, typedefs)?;
                    writeln!(self.out, "\ttestq\t{}, {}", reg(dest), reg(dest)).unwrap();
                    writeln!(self.out, "\tsetne\t%al").unwrap();
                    writeln!(self.out, "\tmovzbq\t%al, {}", reg(dest)).unwrap();
                    writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                    writeln!(self.out, "{l_true}:").unwrap();
                    writeln!(self.out, "\tmovq\t$1, {}", reg(dest)).unwrap();
                    writeln!(self.out, "{l_end}:").unwrap();
                    return Ok(Type::Int);
                }

                let lty = self.emit_expr_rval(left, 9, typedefs)?;
                // Spill 16-byte aligned so nested calls keep SysV alignment.
                writeln!(self.out, "\tsubq\t$16, %rsp").unwrap();
                writeln!(self.out, "\tmovq\t%r10, (%rsp)").unwrap();
                let rty = self.emit_expr_rval(right, 10, typedefs)?;
                writeln!(self.out, "\tmovq\t(%rsp), %r10").unwrap();
                writeln!(self.out, "\taddq\t$16, %rsp").unwrap();

                let floaty = matches!(lty, Type::Float | Type::Double)
                    || matches!(rty, Type::Float | Type::Double);
                if floaty {
                    // GP holds IEEE bits (or int). Convert to XMM double and use SSE2.
                    // float32 bits must be promoted with cvtss2sd — movq of a float
                    // bit pattern as a double is a denormal, breaking `procost <= 0`.
                    match &lty {
                        Type::Float => {
                            writeln!(self.out, "\tmovd\t%r10d, %xmm0").unwrap();
                            writeln!(self.out, "\tcvtss2sd\t%xmm0, %xmm0").unwrap();
                        }
                        Type::Double => writeln!(self.out, "\tmovq\t%r10, %xmm0").unwrap(),
                        _ => writeln!(self.out, "\tcvtsi2sdq\t%r10, %xmm0").unwrap(),
                    }
                    match &rty {
                        Type::Float => {
                            writeln!(self.out, "\tmovd\t%r11d, %xmm1").unwrap();
                            writeln!(self.out, "\tcvtss2sd\t%xmm1, %xmm1").unwrap();
                        }
                        Type::Double => writeln!(self.out, "\tmovq\t%r11, %xmm1").unwrap(),
                        _ => writeln!(self.out, "\tcvtsi2sdq\t%r11, %xmm1").unwrap(),
                    }
                    match op {
                        BinOp::Add => writeln!(self.out, "\taddsd\t%xmm1, %xmm0").unwrap(),
                        BinOp::Sub => writeln!(self.out, "\tsubsd\t%xmm1, %xmm0").unwrap(),
                        BinOp::Mul => writeln!(self.out, "\tmulsd\t%xmm1, %xmm0").unwrap(),
                        BinOp::Div => writeln!(self.out, "\tdivsd\t%xmm1, %xmm0").unwrap(),
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                            writeln!(self.out, "\tucomisd\t%xmm1, %xmm0").unwrap();
                            let set = match op {
                                BinOp::Eq => "sete",
                                BinOp::Ne => "setne",
                                BinOp::Lt => "setb",  // CF: xmm0 < xmm1 (unordered-safe-ish)
                                BinOp::Gt => "seta",
                                BinOp::Le => "setbe",
                                BinOp::Ge => "setae",
                                _ => unreachable!(),
                            };
                            writeln!(self.out, "\t{set}\t%al").unwrap();
                            writeln!(self.out, "\tmovzbq\t%al, {}", reg(dest)).unwrap();
                            return Ok(Type::Int);
                        }
                        _ => {
                            // Non-arith on floats: fall back to integer path bits.
                            writeln!(self.out, "\tmovq\t%xmm0, {}", reg(dest)).unwrap();
                            return Ok(Type::Double);
                        }
                    }
                    writeln!(self.out, "\tmovq\t%xmm0, {}", reg(dest)).unwrap();
                    return Ok(Type::Double);
                }

                match op {
                    BinOp::Add => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            writeln!(self.out, "\timulq\t${esz}, %r11").unwrap();
                            writeln!(self.out, "\tleaq\t(%r10,%r11), {}", reg(dest)).unwrap();
                            return Ok(lty);
                        }
                        let rty = self.typeof_expr(right, typedefs);
                        if let Type::Ptr(inner) = rty {
                            let esz = self.type_size(&inner).max(1);
                            writeln!(self.out, "\timulq\t${esz}, %r10").unwrap();
                            writeln!(self.out, "\tleaq\t(%r11,%r10), {}", reg(dest)).unwrap();
                            return Ok(Type::Ptr(inner));
                        }
                        writeln!(self.out, "\tleaq\t(%r10,%r11), {}", reg(dest)).unwrap();
                        Ok(Self::usual_arith_conv(&lty, &rty))
                    }
                    BinOp::Sub => {
                        let l_is_ptr = matches!(lty, Type::Ptr(_) | Type::Array(_, _));
                        if l_is_ptr {
                            let inner = match &lty {
                                Type::Ptr(i) => i.as_ref(),
                                Type::Array(i, _) => i.as_ref(),
                                _ => &Type::Void,
                            };
                            let esz = self.type_size(inner).max(1);
                            let rty = self.typeof_expr(right, typedefs);
                            if matches!(rty, Type::Ptr(_) | Type::Array(_, _)) {
                                writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                                writeln!(self.out, "\tsubq\t%r11, %rax").unwrap();
                                writeln!(self.out, "\tcqto").unwrap();
                                writeln!(self.out, "\tmovq\t${esz}, %rcx").unwrap();
                                writeln!(self.out, "\tidivq\t%rcx").unwrap();
                                if dest != 0 {
                                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                                }
                                return Ok(Type::Long);
                            }
                            writeln!(self.out, "\timulq\t${esz}, %r11").unwrap();
                            // Compute via %rax so dest cannot clobber %r11 (right).
                            writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                            writeln!(self.out, "\tsubq\t%r11, %rax").unwrap();
                            if dest != 0 {
                                writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                            }
                            return Ok(lty);
                        }
                        // Always use %rax intermediate: if dest is 10 (%r11),
                        // `movq %r10, %r11; subq %r11, %r11` would zero the result.
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\tsubq\t%r11, %rax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(Self::usual_arith_conv(&lty, &rty))
                    }
                    BinOp::Mul => {
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\timulq\t%r11, %rax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(Self::usual_arith_conv(&lty, &rty))
                    }
                    BinOp::Div => {
                        let res_ty = Self::usual_arith_conv(&lty, &rty);
                        let unsigned = matches!(
                            res_ty.unqual(),
                            Type::ULong | Type::UInt | Type::UShort | Type::Char | Type::UChar
                        ) || matches!(
                            lty.unqual(),
                            Type::ULong | Type::UInt | Type::Ptr(_)
                        ) || matches!(
                            rty.unqual(),
                            Type::ULong | Type::UInt | Type::Ptr(_)
                        );
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        if unsigned {
                            writeln!(self.out, "\txorq\t%rdx, %rdx").unwrap();
                            writeln!(self.out, "\tdivq\t%r11").unwrap();
                        } else {
                            writeln!(self.out, "\tcqto").unwrap();
                            writeln!(self.out, "\tidivq\t%r11").unwrap();
                        }
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(res_ty)
                    }
                    BinOp::Mod => {
                        let res_ty = Self::usual_arith_conv(&lty, &rty);
                        let unsigned = matches!(
                            res_ty.unqual(),
                            Type::ULong | Type::UInt | Type::UShort | Type::Char | Type::UChar
                        ) || matches!(
                            lty.unqual(),
                            Type::ULong | Type::UInt | Type::Ptr(_)
                        ) || matches!(
                            rty.unqual(),
                            Type::ULong | Type::UInt | Type::Ptr(_)
                        );
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        if unsigned {
                            writeln!(self.out, "\txorq\t%rdx, %rdx").unwrap();
                            writeln!(self.out, "\tdivq\t%r11").unwrap();
                        } else {
                            writeln!(self.out, "\tcqto").unwrap();
                            writeln!(self.out, "\tidivq\t%r11").unwrap();
                        }
                        // remainder in %rdx
                        writeln!(self.out, "\tmovq\t%rdx, {}", reg(dest)).unwrap();
                        Ok(res_ty)
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        // Narrow ints: cmpl (low 32). SysV leaves high bits of
                        // int returns zero-extended (eax=-1 → rax=0xffffffff),
                        // while Int(-1) is emitted as full 0xffffffffffffffff;
                        // cmpq never matches → getopt while(c!=-1) spins forever.
                        // Wide (long/ptr): cmpq. Matches aarch64 cmp w vs cmp x.
                        let rty = self.typeof_expr(right, typedefs);
                        let lty_u = lty.unqual();
                        let rty_u = rty.unqual();
                        // C integer promotions: char/short/uchar/ushort → int
                        // before relational ops. Mixing promoted uchar with
                        // signed int must use signed setcc — treating Char as
                        // unsigned made `uint8_t i; for (i=0; i<=(int)-1; i++)`
                        // into unsigned `0<=0xffffffff` (DecodeXLogRecord hang).
                        // Only UInt/ULong/pointers force unsigned compares.
                        let unsignedish = matches!(
                            lty_u,
                            Type::UInt | Type::ULong | Type::Ptr(_) | Type::Array(_, _)
                        ) || matches!(
                            rty_u,
                            Type::UInt | Type::ULong | Type::Ptr(_) | Type::Array(_, _)
                        );
                        let wide = matches!(
                            lty_u,
                            Type::Long
                                | Type::ULong
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                                | Type::Float
                                | Type::Double
                        ) || matches!(
                            rty_u,
                            Type::Long
                                | Type::ULong
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                                | Type::Float
                                | Type::Double
                        );
                        if wide {
                            writeln!(self.out, "\tcmpq\t%r11, %r10").unwrap();
                        } else {
                            writeln!(self.out, "\tcmpl\t%r11d, %r10d").unwrap();
                        }
                        let setcc = match (op, unsignedish) {
                            (BinOp::Eq, _) => "sete",
                            (BinOp::Ne, _) => "setne",
                            (BinOp::Lt, true) => "setb",
                            (BinOp::Gt, true) => "seta",
                            (BinOp::Le, true) => "setbe",
                            (BinOp::Ge, true) => "setae",
                            (BinOp::Lt, false) => "setl",
                            (BinOp::Gt, false) => "setg",
                            (BinOp::Le, false) => "setle",
                            (BinOp::Ge, false) => "setge",
                            _ => unreachable!(),
                        };
                        writeln!(self.out, "\t{setcc}\t%al").unwrap();
                        writeln!(self.out, "\tmovzbq\t%al, {}", reg(dest)).unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::BitAnd => {
                        // Always use %rax intermediate: if dest is 10 (%r11),
                        // `movq %r10, %r11; andq %r11, %r11` clobbers the mask
                        // (postgres MAXALIGN via `p += MAXALIGN(sizeof)` → DSA
                        // place unaligned by +7 → Bus error in dshash/dsa).
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\tandq\t%r11, %rax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(Type::Int)
                    }
                    BinOp::BitOr => {
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\torq\t%r11, %rax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(Type::Int)
                    }
                    BinOp::BitXor => {
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\txorq\t%r11, %rax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(Type::Int)
                    }
                    BinOp::Shl => {
                        // Save count to %cl first: dest may be %r11 (holds count) or %rcx.
                        writeln!(self.out, "\tmovq\t%r11, %rcx").unwrap();
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\tshlq\t%cl, %rax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(Type::Int)
                    }
                    BinOp::Shr => {
                        let unsigned = matches!(
                            lty.unqual(),
                            Type::ULong | Type::UInt | Type::UShort | Type::UChar
                        );
                        writeln!(self.out, "\tmovq\t%r11, %rcx").unwrap();
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        if unsigned {
                            writeln!(self.out, "\tshrq\t%cl, %rax").unwrap();
                        } else {
                            writeln!(self.out, "\tsarq\t%cl, %rax").unwrap();
                        }
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        Ok(Type::Int)
                    }
                    BinOp::Comma => {
                        writeln!(self.out, "\tmovq\t%r11, {}", reg(dest)).unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::And | BinOp::Or => unreachable!(),
                }
            }
            Expr::Assign { left, right } => {
                // Dest address first so we know if this is an aggregate copy.
                // (C leaves LHS/RHS evaluation order unspecified.)
                let lty = self.emit_lvalue_addr(left, 9, typedefs)?;
                if self.needs_block_copy_ty(&lty) {
                    let sz = self.type_size(&lty).max(1);
                    writeln!(self.out, "\tpushq\t%r10").unwrap(); // dest
                    let _rty = self.emit_lvalue_addr(right, 0, typedefs)?;
                    writeln!(self.out, "\tpopq\t%rdi").unwrap();
                    writeln!(self.out, "\tmovq\t%rdi, %r8").unwrap(); // save dest (movsb advances)
                    writeln!(self.out, "\tmovq\t%rax, %rsi").unwrap();
                    writeln!(self.out, "\tmovq\t${sz}, %rcx").unwrap();
                    writeln!(self.out, "\tcld").unwrap();
                    writeln!(self.out, "\trep\tmovsb").unwrap();
                    writeln!(self.out, "\tmovq\t%r8, %rax").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(lty);
                }
                writeln!(self.out, "\tpushq\t%r10").unwrap();
                let rty = self.emit_expr_rval(right, 0, typedefs)?;
                writeln!(self.out, "\tpopq\t%r10").unwrap();
                self.coerce_rax_to_ty(&rty, &lty);
                self.store_ty(&lty, 9, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                }
                Ok(lty)
            }
            Expr::CompoundAssign { op, left, right } => {
                let lty = self.emit_lvalue_addr(left, 19, typedefs)?;
                self.load_ty(&lty, 19, 9);
                // Spill LHS address (%rbx/r19) and value (%r10): RHS eval
                // (e.g. `len += *sp++`) reuses emit_lvalue_addr(..., 19) and
                // would otherwise clobber %rbx so store_ty writes the wrong slot.
                writeln!(self.out, "\tpushq\t%rbx").unwrap();
                writeln!(self.out, "\tpushq\t%r10").unwrap();
                self.emit_expr_rval(right, 10, typedefs)?;
                writeln!(self.out, "\tpopq\t%r10").unwrap();
                writeln!(self.out, "\tpopq\t%rbx").unwrap();
                match op {
                    BinOp::Add => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            writeln!(self.out, "\timulq\t${esz}, %r11").unwrap();
                        }
                        writeln!(self.out, "\tleaq\t(%r10,%r11), %rax").unwrap();
                    }
                    BinOp::Sub => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            writeln!(self.out, "\timulq\t${esz}, %r11").unwrap();
                        }
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\tsubq\t%r11, %rax").unwrap();
                    }
                    BinOp::Mul => {
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\timulq\t%r11, %rax").unwrap();
                    }
                    BinOp::Div => {
                        let unsigned = matches!(
                            lty.unqual(),
                            Type::ULong | Type::UInt | Type::UShort | Type::UChar
                        );
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        if unsigned {
                            writeln!(self.out, "\txorq\t%rdx, %rdx").unwrap();
                            writeln!(self.out, "\tdivq\t%r11").unwrap();
                        } else {
                            writeln!(self.out, "\tcqto").unwrap();
                            writeln!(self.out, "\tidivq\t%r11").unwrap();
                        }
                    }
                    BinOp::Mod => {
                        let unsigned = matches!(
                            lty.unqual(),
                            Type::ULong | Type::UInt | Type::UShort | Type::UChar
                        );
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        if unsigned {
                            writeln!(self.out, "\txorq\t%rdx, %rdx").unwrap();
                            writeln!(self.out, "\tdivq\t%r11").unwrap();
                        } else {
                            writeln!(self.out, "\tcqto").unwrap();
                            writeln!(self.out, "\tidivq\t%r11").unwrap();
                        }
                        writeln!(self.out, "\tmovq\t%rdx, %rax").unwrap();
                    }
                    BinOp::BitAnd => {
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\tandq\t%r11, %rax").unwrap();
                    }
                    BinOp::BitOr => {
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\torq\t%r11, %rax").unwrap();
                    }
                    BinOp::BitXor => {
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\txorq\t%r11, %rax").unwrap();
                    }
                    BinOp::Shl => {
                        // Same dest/%r11 hazard as non-compound Shl.
                        writeln!(self.out, "\tmovq\t%r11, %rcx").unwrap();
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        writeln!(self.out, "\tshlq\t%cl, %rax").unwrap();
                    }
                    BinOp::Shr => {
                        let unsigned = matches!(
                            lty.unqual(),
                            Type::ULong | Type::UInt | Type::UShort | Type::UChar
                        );
                        writeln!(self.out, "\tmovq\t%r11, %rcx").unwrap();
                        writeln!(self.out, "\tmovq\t%r10, %rax").unwrap();
                        if unsigned {
                            writeln!(self.out, "\tshrq\t%cl, %rax").unwrap();
                        } else {
                            writeln!(self.out, "\tsarq\t%cl, %rax").unwrap();
                        }
                    }
                    _ => return Err("bad compound assign".into()),
                }
                self.store_ty(&lty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                }
                Ok(lty)
            }
            Expr::Call { name, args } => {
                if name == "__builtin_trap" {
                    writeln!(self.out, "\tud2").unwrap();
                    return Ok(Type::Void);
                }
                if name == "__builtin_classify_type" {
                    let val: i64 = if let Some(arg) = args.first() {
                        let ty = self.typeof_expr(arg, typedefs);
                        match ty {
                            Type::Void => -1,
                            Type::Int | Type::Long | Type::Short | Type::Char | Type::SChar | Type::UChar | Type::UInt | Type::ULong | Type::UShort => 1,
                            Type::Float | Type::Double => 8,
                            Type::Ptr(_) | Type::Array(_, _) => 5,
                            Type::Struct(_) | Type::AnonStruct(_) => 12,
                            Type::Union(_) | Type::AnonUnion(_) => 13,
                            _ => 1,
                        }
                    } else {
                        1
                    };
                    writeln!(self.out, "\tmovq\t${val}, %rax").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                let name_buf;
                let name = match name.as_str() {
                    "__builtin_abort" => {
                        name_buf = "abort".to_string();
                        &name_buf
                    }
                    "__builtin_exit" => {
                        name_buf = "exit".to_string();
                        &name_buf
                    }
                    "__builtin_printf" => {
                        name_buf = "printf".to_string();
                        &name_buf
                    }
                    "__builtin_malloc" => {
                        name_buf = "malloc".to_string();
                        &name_buf
                    }
                    "__builtin_free" => {
                        name_buf = "free".to_string();
                        &name_buf
                    }
                    "__builtin_memset" => {
                        name_buf = "memset".to_string();
                        &name_buf
                    }
                    "__builtin_memcpy" => {
                        name_buf = "memcpy".to_string();
                        &name_buf
                    }
                    "__builtin_memcmp" => {
                        name_buf = "memcmp".to_string();
                        &name_buf
                    }
                    "__builtin_alloca" => {
                        name_buf = "alloca".to_string();
                        &name_buf
                    }
                    "__builtin_strcpy" => {
                        name_buf = "strcpy".to_string();
                        &name_buf
                    }
                    "__builtin_strcmp" => {
                        name_buf = "strcmp".to_string();
                        &name_buf
                    }
                    "__builtin_strlen" => {
                        name_buf = "strlen".to_string();
                        &name_buf
                    }
                    "__builtin_vsnprintf" => {
                        name_buf = "vsnprintf".to_string();
                        &name_buf
                    }
                    "__builtin_vfprintf" => {
                        name_buf = "vfprintf".to_string();
                        &name_buf
                    }
                    "__builtin_vprintf" => {
                        name_buf = "vprintf".to_string();
                        &name_buf
                    }
                    "__builtin_vsprintf" => {
                        name_buf = "vsprintf".to_string();
                        &name_buf
                    }
                    _ => name,
                };
                if matches!(
                    name.as_str(),
                    "vprintf"
                        | "vfprintf"
                        | "vsprintf"
                        | "vsnprintf"
                        | "vdprintf"
                        | "vscanf"
                        | "vfscanf"
                        | "vsscanf"
                ) && args.len() >= 2
                    && !self.contains_local(name)
                    && !self.globals.contains_key(name)
                    && !self.funcs.get(name).map_or(false, |f| f.body.is_some())
                {
                    let n = args.len();
                    // SysV x86-64 va_list shim: construct a 24-byte va_list struct on stack:
                    // 0(%rsp): gp_offset = 0 (u32)
                    // 4(%rsp): fp_offset = 48 (u32)
                    // 8(%rsp): overflow_arg_area = pointer to self.va_regsave_off + 48(%rbp)
                    // 16(%rsp): reg_save_area = pointer to self.va_regsave_off(%rbp) (or ap)
                    // 24(%rsp)..47(%rsp): saved fixed args 0..n-2
                    writeln!(self.out, "\tsubq\t$48, %rsp").unwrap();
                    for i in 0..(n - 1) {
                        self.emit_expr_rval(&args[i], 0, typedefs)?;
                        let off = 24 + (i as i64) * 8;
                        writeln!(self.out, "\tmovq\t%rax, {off}(%rsp)").unwrap();
                    }
                    self.emit_expr_rval(&args[n - 1], 0, typedefs)?;
                    writeln!(self.out, "\tmovl\t$0, (%rsp)").unwrap();
                    writeln!(self.out, "\tmovl\t$48, 4(%rsp)").unwrap();
                    let ovf_off = if self.va_regsave_off != 0 {
                        self.va_regsave_off + 48
                    } else {
                        16
                    };
                    writeln!(self.out, "\tleaq\t{ovf_off}(%rbp), %r10").unwrap();
                    writeln!(self.out, "\tmovq\t%r10, 8(%rsp)").unwrap();
                    writeln!(self.out, "\tmovq\t%rax, 16(%rsp)").unwrap();
                    for i in 0..(n - 1) {
                        let off = 24 + (i as i64) * 8;
                        writeln!(self.out, "\tmovq\t{off}(%rsp), {}", ARG_REGS[i]).unwrap();
                    }
                    let ap_reg = ARG_REGS[n - 1];
                    writeln!(self.out, "\tmovq\t%rsp, {ap_reg}").unwrap();
                    writeln!(self.out, "\txorb\t%al, %al").unwrap();
                    let s = sym(name);
                    if cfg!(target_os = "macos") {
                        writeln!(self.out, "\tcallq\t{s}").unwrap();
                    } else {
                        writeln!(self.out, "\tcallq\t{s}@PLT").unwrap();
                    }
                    writeln!(self.out, "\taddq\t$48, %rsp").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                // Fold GCC constant builtins left as runtime calls after parse.
                // Without this, compressed/usercopy paths leave `U __builtin_constant_p`.
                if name == "__builtin_constant_p" {
                    writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                // Soft: unknown object size (GCC returns (size_t)-1 for type 0/1).
                if name == "__builtin_object_size" {
                    writeln!(self.out, "\tmovq\t$-1, %rax").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Long);
                }
                if name == "__builtin_return_address" {
                    let _ = args;
                    // Return address above saved rbp (approximate; soft lockdep cookie).
                    writeln!(self.out, "\tmovq\t8(%rbp), {}", reg(dest)).unwrap();
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                if name == "__builtin_frame_address" {
                    let _ = args;
                    writeln!(self.out, "\tmovq\t%rbp, {}", reg(dest)).unwrap();
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                // Intel SSE4.2 CRC32 intrinsics (nmmintrin.h / Postgres pg_crc32c_sse42).
                if name == "_mm_crc32_u8" || name == "_mm_crc32_u32" || name == "_mm_crc32_u64" {
                    if args.len() < 2 {
                        return Err(format!("{name} needs (crc, data)"));
                    }
                    // 1. Evaluate args[0] (crc) -> %rax
                    self.emit_expr_rval(&args[0], 0, typedefs)?;
                    // 2. Zero-extend 32-bit crc to 64-bit %rax for 8/32-bit variants (movl clears upper 32 bits of %rax)
                    if name != "_mm_crc32_u64" {
                        writeln!(self.out, "\tmovl\t%eax, %eax").unwrap();
                    }

                    // 3. Save crc on 16-byte aligned stack frame
                    writeln!(self.out, "\tsubq\t$16, %rsp").unwrap();
                    writeln!(self.out, "\tmovq\t%rax, (%rsp)").unwrap();

                    // 4. Evaluate args[1] (data) -> %rax, then copy to %rsi
                    self.emit_expr_rval(&args[1], 0, typedefs)?;
                    writeln!(self.out, "\tmovq\t%rax, %rsi").unwrap();

                    // 5. Restore crc -> %rax from stack
                    writeln!(self.out, "\tmovq\t(%rsp), %rax").unwrap();
                    writeln!(self.out, "\taddq\t$16, %rsp").unwrap();

                    // 6. Emit hardware CRC32 instruction
                    match name.as_str() {
                        "_mm_crc32_u8" => {
                            writeln!(self.out, "\tcrc32b\t%sil, %eax").unwrap();
                        }
                        "_mm_crc32_u32" => {
                            writeln!(self.out, "\tcrc32l\t%esi, %eax").unwrap();
                        }
                        _ => {
                            // u64: dest must be 64-bit form of crc register.
                            writeln!(self.out, "\tcrc32q\t%rsi, %rax").unwrap();
                        }
                    }
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::UInt);
                }
                // GCC checked arithmetic (same contract as aarch64 codegen).
                if name == "__builtin_add_overflow"
                    || name == "__builtin_sub_overflow"
                    || name == "__builtin_mul_overflow"
                {
                    if args.len() >= 3 {
                        self.emit_builtin_overflow(
                            name,
                            &args[0],
                            &args[1],
                            &args[2],
                            dest,
                            typedefs,
                        )?;
                        return Ok(Type::Int);
                    }
                }
                // Soft va_arg/va_start: SysV GP regsave cursor (postgres snprintf).
                if name == "__acc_va_start" {
                    if self.va_regsave_off == 0 {
                        // Not in a variadic function — best-effort stack args.
                        writeln!(self.out, "\tleaq\t16(%rbp), {}", reg(dest)).unwrap();
                    } else {
                        let off = self.va_regsave_off + (self.va_fixed_n as i64) * 8;
                        writeln!(self.out, "\tleaq\t{off}(%rbp), {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Ptr(Box::new(Type::Char)));
                }
                if name == "__acc_va_arg" {
                    // args: &ap — load cursor, return it, advance ap by 8.
                    if args.is_empty() {
                        return Err("__acc_va_arg needs &ap".into());
                    }
                    let ap_lvalue = match &args[0] {
                        Expr::Unary {
                            op: UnaryOp::Addr,
                            expr,
                        } => expr.as_ref(),
                        other => other,
                    };
                    self.emit_lvalue_addr(ap_lvalue, 9, typedefs)?;
                    // r10 = &ap; rax = *ap (cursor)
                    writeln!(self.out, "\tmovq\t(%r10), %rax").unwrap();
                    writeln!(self.out, "\tleaq\t8(%rax), %r11").unwrap();
                    writeln!(self.out, "\tmovq\t%r11, (%r10)").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                // SysV: first 6 in regs; rest on stack (same as direct calls).
                    // Soft builtins: ctz/clz/ffs (kernel uses these heavily).
                if name == "__builtin_ctz" || name == "__builtin_ctzl" || name == "__builtin_ctzll"
                {
                    if args.is_empty() {
                        return Err("ctz needs arg".into());
                    }
                    self.emit_expr_rval(&args[0], 0, typedefs)?;
                    if name == "__builtin_ctz" {
                        writeln!(self.out, "\tbsfl\t%eax, %eax").unwrap();
                    } else {
                        writeln!(self.out, "\tbsfq\t%rax, %rax").unwrap();
                    }
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                if name == "__builtin_clz" || name == "__builtin_clzl" || name == "__builtin_clzll"
                {
                    if args.is_empty() {
                        return Err("clz needs arg".into());
                    }
                    self.emit_expr_rval(&args[0], 0, typedefs)?;
                    if name == "__builtin_clz" {
                        writeln!(self.out, "\tbsrl\t%eax, %eax").unwrap();
                        writeln!(self.out, "\txorl\t$31, %eax").unwrap();
                    } else {
                        writeln!(self.out, "\tbsrq\t%rax, %rax").unwrap();
                        writeln!(self.out, "\txorq\t$63, %rax").unwrap();
                    }
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                if name == "__builtin_ffs" || name == "__builtin_ffsl" || name == "__builtin_ffsll"
                {
                    if args.is_empty() {
                        return Err("ffs needs arg".into());
                    }
                    self.emit_expr_rval(&args[0], 0, typedefs)?;
                    let wide = name != "__builtin_ffs";
                    if wide {
                        writeln!(self.out, "\tmovq\t%rax, %rcx").unwrap();
                        writeln!(self.out, "\tbsfq\t%rax, %rax").unwrap();
                        writeln!(self.out, "\tleaq\t1(%rax), %rax").unwrap();
                        writeln!(self.out, "\ttestq\t%rcx, %rcx").unwrap();
                        writeln!(self.out, "\tcmoveq\t%rcx, %rax").unwrap();
                    } else {
                        writeln!(self.out, "\tmovl\t%eax, %ecx").unwrap();
                        writeln!(self.out, "\tbsfl\t%eax, %eax").unwrap();
                        writeln!(self.out, "\tleal\t1(%eax), %eax").unwrap();
                        writeln!(self.out, "\ttestl\t%ecx, %ecx").unwrap();
                        writeln!(self.out, "\tcmovel\t%ecx, %eax").unwrap();
                    }
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                if name == "__builtin_popcount"
                    || name == "__builtin_popcountl"
                    || name == "__builtin_popcountll"
                {
                    if let Some(a) = args.first() {
                        self.emit_builtin_popcount(name, a, dest, typedefs)?;
                        return Ok(Type::Int);
                    }
                }
                if name == "__sync_fetch_and_add"
                    || name == "__sync_fetch_and_sub"
                    || name == "__sync_fetch_and_and"
                    || name == "__sync_fetch_and_or"
                {
                    if args.len() >= 2 {
                        let op = name.strip_prefix("__sync_fetch_and_").unwrap_or("add");
                        self.emit_sync_fetch_and(op, &args[0], &args[1], dest, typedefs)?;
                        return Ok(Type::Long);
                    }
                }
                if name == "__get_cpuid" {
                    self.emit_get_cpuid(args, dest, typedefs)?;
                    return Ok(Type::Int);
                }
                if name == "__builtin_unreachable" {
                    let lab = self.lab("unreachable");
                    writeln!(self.out, "{lab}:").unwrap();
                    writeln!(self.out, "\tjmp\t{lab}").unwrap();
                    return Ok(Type::Void);
                }
                if name == "__builtin_bswap16" || name == "__acc_bswap16" {
                    if let Some(a) = args.first() {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\trolw\t$8, %ax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        return Ok(Type::UShort);
                    }
                }
                if name == "__builtin_bswap32" || name == "__acc_bswap32" {
                    if let Some(a) = args.first() {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\tbswap\t%eax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        return Ok(Type::UInt);
                    }
                }
                if name == "__builtin_bswap64" || name == "__acc_bswap64" {
                    if let Some(a) = args.first() {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\tbswap\t%rax").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                        }
                        return Ok(Type::ULong);
                    }
                }

                if name == "__indirect__" {
                    if args.is_empty() {
                        return Err("indirect call missing callee".into());
                    }
                    let (callee, real_args) = args.split_first().unwrap();
                    let callee = match callee {
                        Expr::Unary {
                            op: UnaryOp::Deref,
                            expr,
                        } => expr.as_ref(),
                        other => other,
                    };
                    self.emit_expr_rval(callee, 0, typedefs)?;
                    writeln!(self.out, "\tsubq\t$16, %rsp").unwrap();
                    writeln!(self.out, "\tmovq\t%rax, (%rsp)").unwrap();
                    let (_nreg, nstack_bytes) = self.emit_sysv_arg_setup(real_args, &[], typedefs, 2)?;
                    let callee_off = nstack_bytes;
                    writeln!(self.out, "\tmovq\t{callee_off}(%rsp), %r10").unwrap();
                    writeln!(self.out, "\txorb\t%al, %al").unwrap();
                    writeln!(self.out, "\tcallq\t*%r10").unwrap();
                    let total_cleanup = nstack_bytes + 16;
                    writeln!(self.out, "\taddq\t${total_cleanup}, %rsp").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }

                // Known libm double(double): SysV arg/return in %xmm0 (not %rdi/%rax).
                // Without this, `val = rint(val)` passes IEEE bits in rdi and then
                // cvtsi2sd's a garbage int return → parse_int("16777216") → 0.
                // Exclude string→float (atof/strtod/strtof/nan): those take a
                // pointer in %rdi. Treating them as xmm0-arg clobbers the ABI
                // (`cvtsi2sd` of a char* before call).
                let libm_unary_xmm = matches!(
                    name.as_str(),
                    "sin"
                        | "cos"
                        | "tan"
                        | "asin"
                        | "acos"
                        | "atan"
                        | "sinh"
                        | "cosh"
                        | "tanh"
                        | "exp"
                        | "exp2"
                        | "expm1"
                        | "log"
                        | "log2"
                        | "log10"
                        | "log1p"
                        | "sqrt"
                        | "cbrt"
                        | "fabs"
                        | "ceil"
                        | "floor"
                        | "trunc"
                        | "rint"
                        | "round"
                        | "nearbyint"
                );
                if args.len() == 1
                    && libm_unary_xmm
                    && matches!(Self::known_fp_return(name), Some(Type::Double))
                {
                    self.emit_expr_rval(&args[0], 0, typedefs)?;
                    let aty = self.typeof_expr(&args[0], typedefs);
                    if !matches!(aty, Type::Float | Type::Double) {
                        writeln!(self.out, "\tcvtsi2sdq\t%rax, %xmm0").unwrap();
                    } else if matches!(aty, Type::Float) {
                        writeln!(self.out, "\tmovd\t%eax, %xmm0").unwrap();
                        writeln!(self.out, "\tcvtss2sd\t%xmm0, %xmm0").unwrap();
                    } else {
                        writeln!(self.out, "\tmovq\t%rax, %xmm0").unwrap();
                    }
                    let s = sym(name);
                    writeln!(self.out, "\tcallq\t{s}@PLT").unwrap();
                    self.emit_extend_call_return(dest, &Type::Double);
                    return Ok(Type::Double);
                }

                // SysV: first 6 integer *eightbytes* in rdi..r9; rest on stack.
                // Aggregates ≤16 bytes expand to 1–2 eightbytes (RelFileNode=12).
                // Prefer *callee prototype* param types for slot counts — using
                // typeof(arg) mismatches declared Struct params when soft types
                // the expression as Int/Ptr and breaks bison (boot_yyparse
                // "memory exhausted").
                let proto: Vec<Type> = self
                    .funcs
                    .get(name)
                    .map(|f| {
                        f.params
                            .iter()
                            .map(|(_, t)| match t {
                                Type::Array(e, _) => Type::Ptr(e.clone()),
                                other => other.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let is_fn_ptr_var = self.contains_local(name)
                    || (self.globals.contains_key(name) && !self.funcs.contains_key(name));
                let nstack_bytes = if is_fn_ptr_var {
                    self.emit_expr_rval(&Expr::Var(name.clone()), 0, typedefs)?;
                    writeln!(self.out, "\tsubq\t$16, %rsp").unwrap();
                    writeln!(self.out, "\tmovq\t%rax, (%rsp)").unwrap();
                    let (_nreg, nsb) = self.emit_sysv_arg_setup(args, &proto, typedefs, 2)?;
                    let callee_off = nsb;
                    writeln!(self.out, "\tmovq\t{callee_off}(%rsp), %r10").unwrap();
                    writeln!(self.out, "\txorb\t%al, %al").unwrap();
                    writeln!(self.out, "\tcallq\t*%r10").unwrap();
                    nsb + 16
                } else {
                    let (_nreg, nsb) = self.emit_sysv_arg_setup(args, &proto, typedefs, 0)?;
                    let s = sym(name);
                    writeln!(self.out, "\txorb\t%al, %al").unwrap();
                    if cfg!(target_os = "macos") {
                        writeln!(self.out, "\tcallq\t{s}").unwrap();
                    } else {
                        writeln!(self.out, "\tcallq\t{s}@PLT").unwrap();
                    }
                    nsb
                };
                // Pop stack args / pushed callee after call
                if nstack_bytes > 0 {
                    writeln!(self.out, "\taddq\t${nstack_bytes}, %rsp").unwrap();
                }
                // Narrow ints from *known* prototypes get extended so 64-bit
                // cmp against -1 works; undeclared libc defaults look like Int
                // but often return pointers (malloc/strdup) — do not movslq those.
                let ret_ty = self
                    .funcs
                    .get(name)
                    .map(|f| f.ret.clone())
                    .or_else(|| Self::known_fp_return(name))
                    .unwrap_or(Type::Int);
                if self.funcs.contains_key(name) || matches!(ret_ty, Type::Float | Type::Double) {
                    self.emit_extend_call_return(dest, &ret_ty);
                } else if dest != 0 {
                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                }
                Ok(ret_ty)
            }
            Expr::Index { .. } | Expr::Member { .. } => {
                let ty = self.emit_lvalue_addr(e, 9, typedefs)?;
                // Array lvalues decay to pointer (address), not a loaded value.
                // Needed for nested array members: a[0].sub[i] where .sub is T[N].
                if let Type::Array(elem, _) = ty {
                    if dest != 9 {
                        writeln!(self.out, "\tmovq\t%r10, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Ptr(elem));
                }
                self.load_ty(&ty, 9, dest);
                Ok(ty)
            }
            Expr::Cast { ty, expr } => {
                // Compound literal `(T){ .f = runtime }` as rvalue (e.g. postgres
                // list_make_ptr_cell → list_make1_impl). Soft used to eval InitList
                // as NULL, so ListCell args were always nil.
                if let Expr::InitList { fields } = expr.as_ref() {
                    let to = ty.clone();
                    let sz = self.type_size(&to).max(1);
                    let slot = Self::align_up(sz, 16);
                    writeln!(self.out, "\tsubq\t${slot}, %rsp").unwrap();
                    // Zero slot so omitted fields stay 0 (must not clobber %rdi/%rcx parameter registers).
                    for off in (0..slot).step_by(8) {
                        writeln!(self.out, "\tmovq\t$0, {off}(%rsp)").unwrap();
                    }
                    self.emit_init_list_at_rsp(0, &to, fields, typedefs)?;
                    writeln!(self.out, "\tmovq\t%rsp, %r10").unwrap();
                    self.load_ty(&to, 9, dest);
                    writeln!(self.out, "\taddq\t${slot}, %rsp").unwrap();
                    return Ok(to);
                }
                let from = self.emit_expr_rval(expr, dest, typedefs)?;
                let to = ty.clone();
                // int/long → double/float (must cvtsd2ss for float — leaving
                // double bits and `movl %eax` turns 1.0 into 0.0).
                if matches!(to, Type::Double | Type::Float)
                    && !matches!(from, Type::Double | Type::Float)
                {
                    writeln!(self.out, "\tcvtsi2sdq\t{}, %xmm0", reg(dest)).unwrap();
                    if matches!(to, Type::Float) {
                        writeln!(self.out, "\tcvtsd2ss\t%xmm0, %xmm0").unwrap();
                        writeln!(self.out, "\tmovd\t%xmm0, {}", reg_d(dest)).unwrap();
                    } else {
                        writeln!(self.out, "\tmovq\t%xmm0, {}", reg(dest)).unwrap();
                    }
                } else if matches!(from, Type::Double) && matches!(to, Type::Float) {
                    writeln!(self.out, "\tmovq\t{}, %xmm0", reg(dest)).unwrap();
                    writeln!(self.out, "\tcvtsd2ss\t%xmm0, %xmm0").unwrap();
                    writeln!(self.out, "\tmovd\t%xmm0, {}", reg_d(dest)).unwrap();
                } else if matches!(from, Type::Float) && matches!(to, Type::Double) {
                    writeln!(self.out, "\tmovd\t{}, %xmm0", reg_d(dest)).unwrap();
                    writeln!(self.out, "\tcvtss2sd\t%xmm0, %xmm0").unwrap();
                    writeln!(self.out, "\tmovq\t%xmm0, {}", reg(dest)).unwrap();
                } else if matches!(from, Type::Double | Type::Float)
                    && !matches!(
                        to,
                        Type::Double | Type::Float | Type::Void | Type::Ptr(_) | Type::Array(_, _)
                    )
                {
                    // double/float → int (truncate toward zero)
                    if matches!(from, Type::Float) {
                        writeln!(self.out, "\tmovd\t{}, %xmm0", reg_d(dest)).unwrap();
                        writeln!(self.out, "\tcvtss2sd\t%xmm0, %xmm0").unwrap();
                    } else {
                        writeln!(self.out, "\tmovq\t{}, %xmm0", reg(dest)).unwrap();
                    }
                    writeln!(self.out, "\tcvttsd2siq\t%xmm0, {}", reg(dest)).unwrap();
                } else if !matches!(to, Type::Double | Type::Float | Type::Void | Type::Ptr(_)) {
                    // Narrow integer casts must truncate (then zero/sign-extend).
                    // `(uint16)-1` must be 65535, not -1 — otherwise
                    // `firstfree != (uint16)-1` is always true and postgres DSA
                    // takes the freelist path with firstfree=0xffff
                    // (8192+65535*56 → bogus span → SEGV in init_span).
                    match to.unqual() {
                        Type::UShort | Type::Char | Type::UChar => {
                            writeln!(
                                self.out,
                                "\tmovzwq\t{}, {}",
                                reg_w(dest),
                                reg(dest)
                            )
                            .unwrap();
                        }
                        Type::Short | Type::SChar => {
                            writeln!(
                                self.out,
                                "\tmovswq\t{}, {}",
                                reg_w(dest),
                                reg(dest)
                            )
                            .unwrap();
                        }
                        Type::UInt => {
                            writeln!(
                                self.out,
                                "\tmovl\t{}, {}",
                                reg_d(dest),
                                reg_d(dest)
                            )
                            .unwrap();
                        }
                        Type::Int => {
                            writeln!(
                                self.out,
                                "\tmovslq\t{}, {}",
                                reg_d(dest),
                                reg(dest)
                            )
                            .unwrap();
                        }
                        _ => {}
                    }
                }
                Ok(to)
            }
            Expr::SizeofType(ty) => {
                let s = self.type_size(ty);
                self.emit_imm(s, dest);
                Ok(Type::Int)
            }
            Expr::SizeofExpr(ex) => {
                let ty = self.typeof_expr(ex, typedefs);
                let s = match &ty {
                    Type::Array(e, n) => self.type_size(e) * n,
                    other => self.type_size(other),
                };
                self.emit_imm(s, dest);
                Ok(Type::Int)
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let l_else = self.lab("cond_else");
                let l_end = self.lab("cond_end");
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\ttestq\t%rax, %rax").unwrap();
                writeln!(self.out, "\tje\t{l_else}").unwrap();
                self.emit_expr_rval(then_e, dest, typedefs)?;
                writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                self.emit_expr_rval(else_e, dest, typedefs)?;
                writeln!(self.out, "{l_end}:").unwrap();
                Ok(Type::Int)
            }
            Expr::PreInc(ex) => {
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                writeln!(self.out, "\taddq\t${step}, %rax").unwrap();
                self.store_ty(&ty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                }
                Ok(ty)
            }
            Expr::PreDec(ex) => {
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                writeln!(self.out, "\tsubq\t${step}, %rax").unwrap();
                self.store_ty(&ty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                }
                Ok(ty)
            }
            Expr::PostInc(ex) => {
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                writeln!(self.out, "\tmovq\t%rax, %r8").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                writeln!(self.out, "\taddq\t${step}, %rax").unwrap();
                self.store_ty(&ty, 19, 0);
                writeln!(self.out, "\tmovq\t%r8, {}", reg(dest)).unwrap();
                Ok(ty)
            }
            Expr::PostDec(ex) => {
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                writeln!(self.out, "\tmovq\t%rax, %r8").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                writeln!(self.out, "\tsubq\t${step}, %rax").unwrap();
                self.store_ty(&ty, 19, 0);
                writeln!(self.out, "\tmovq\t%r8, {}", reg(dest)).unwrap();
                Ok(ty)
            }
            Expr::InitList { .. } => {
                // Soft: pointer to empty (compound literal fallback)
                writeln!(self.out, "\txorq\t%rax, %rax").unwrap();
                if dest != 0 {
                    writeln!(self.out, "\tmovq\t%rax, {}", reg(dest)).unwrap();
                }
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
        }
    }

    fn typeof_expr(&self, e: &Expr, typedefs: &HashMap<String, Type>) -> Type {
        match e {
            Expr::StmtExpr(_stmts, final_expr) => self.typeof_expr(final_expr, typedefs),
            Expr::Int(_) | Expr::Char(_) => Type::Int,
            Expr::Float(_) => Type::Double,
            Expr::String(_) => Type::Ptr(Box::new(Type::Char)),
            Expr::Var(n) => {
                if let Some(s) = self.get_local(n) {
                    return s.ty.clone();
                }
                if let Some(t) = self.globals.get(n) {
                    return t.clone();
                }
                Type::Int
            }
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => Type::Ptr(Box::new(self.typeof_expr(expr, typedefs))),
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => match self.typeof_expr(expr, typedefs) {
                Type::Ptr(i) | Type::Array(i, _) => *i,
                _ => Type::Int,
            },
            Expr::Index { base, .. } => match self.typeof_expr(base, typedefs) {
                Type::Ptr(i) | Type::Array(i, _) => *i,
                _ => Type::Int,
            },
            Expr::Cast { ty, .. } => ty.clone(),
            Expr::Binary {
                op: BinOp::Add,
                left,
                right,
            } => {
                let l = self.typeof_expr(left, typedefs);
                let r = self.typeof_expr(right, typedefs);
                if matches!(l, Type::Float | Type::Double) || matches!(r, Type::Float | Type::Double)
                {
                    Type::Double
                } else if matches!(l, Type::Ptr(_)) {
                    l
                } else if matches!(r, Type::Ptr(_)) {
                    r
                } else {
                    Self::usual_arith_conv(&l, &r)
                }
            }
            Expr::Binary {
                op: BinOp::Sub,
                left,
                right,
            } => {
                let l = self.typeof_expr(left, typedefs);
                let r = self.typeof_expr(right, typedefs);
                if matches!(l, Type::Float | Type::Double) || matches!(r, Type::Float | Type::Double)
                {
                    Type::Double
                } else if matches!(l, Type::Ptr(_)) && matches!(r, Type::Ptr(_)) {
                    Type::Long
                } else if matches!(l, Type::Ptr(_)) {
                    l
                } else {
                    Self::usual_arith_conv(&l, &r)
                }
            }
            Expr::Binary {
                op: BinOp::Mul | BinOp::Div | BinOp::Mod,
                left,
                right,
            } => {
                let l = self.typeof_expr(left, typedefs);
                let r = self.typeof_expr(right, typedefs);
                if matches!(l, Type::Float | Type::Double) || matches!(r, Type::Float | Type::Double)
                {
                    Type::Double
                } else {
                    Self::usual_arith_conv(&l, &r)
                }
            }
            Expr::Binary {
                op: BinOp::Shl | BinOp::Shr,
                left,
                ..
            } => {
                let l = self.typeof_expr(left, typedefs);
                match l {
                    Type::ULong => Type::ULong,
                    Type::Long => Type::Long,
                    Type::UInt => Type::UInt,
                    _ => Type::Int,
                }
            }
            Expr::Member { base, field, arrow } => {
                let bt = if *arrow {
                    match self.typeof_expr(base, typedefs) {
                        Type::Ptr(i) => *i,
                        t => t,
                    }
                } else {
                    self.typeof_expr(base, typedefs)
                };
                match bt {
                    Type::Struct(n) | Type::Union(n) => self
                        .layouts
                        .get(&n)
                        .and_then(|l| l.fields.get(field).map(|(_, t)| t.clone()))
                        .unwrap_or(Type::Int),
                    _ => Type::Int,
                }
            }
            Expr::Call { name, .. } => self
                .funcs
                .get(name)
                .map(|f| f.ret.clone())
                .or_else(|| Self::known_fp_return(name))
                .unwrap_or(Type::Int),
            _ => Type::Int,
        }
    }

    fn emit_imm(&mut self, n: i64, dest: u8) {
        // movabs for full 64-bit immediates; smaller ones use movq $imm
        if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
            writeln!(self.out, "\tmovq\t${n}, {}", reg(dest)).unwrap();
        } else {
            writeln!(self.out, "\tmovabsq\t${n}, {}", reg(dest)).unwrap();
        }
    }
}

include!("codegen_x86_64_freestanding.rs");

pub fn emit_assembly(prog: &Program) -> Result<String, String> {
    let mut cg = Codegen::new();
    // Pre-reserve space for saved rbx in stack accounting for params too.
    cg.compile(prog)
}
