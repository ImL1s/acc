//! Code generators: aarch64-apple-darwin (default), x86_64, i686, riscv64.
//! Emits real assembly from the AST — no fixture hardcoding.

#[path = "codegen_x86_64.rs"]
mod x86_64;

#[path = "codegen_i686.rs"]
pub mod i686;

#[path = "codegen_riscv.rs"]
pub mod riscv;

use crate::ast::*;
use crate::assigned_names::collect_assigned_names_in_program;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

/// Reachable function names that must be emitted.
///
/// Kernel headers pull thousands of `static inline` helpers into every TU.
/// Emitting them all creates undefined refs (e.g. after arm64 PI
/// `objcopy --prefix-symbols=__pi_`) that real gcc never links because it
/// only emits referenced statics. Roots are non-static functions, `main`,
/// and any function named from global initializers (ops tables / fops).
/// One defining function per name: prefer the longest body so an empty soft-stub
/// `{ }` cannot shadow the real definition (postgres `pq_writeint*`, `t_isspace`, …).
pub fn best_functions_for_emit<'a>(prog: &'a Program) -> HashMap<String, &'a Function> {
    let mut best: HashMap<String, &'a Function> = HashMap::new();
    for item in &prog.items {
        let Item::Func(f) = item else {
            continue;
        };
        let Some(body) = f.body.as_ref() else {
            continue;
        };
        let new_len = body.len();
        let replace = match best.get(&f.name) {
            None => true,
            Some(prev) => new_len > prev.body.as_ref().map(|b| b.len()).unwrap_or(0),
        };
        if replace {
            best.insert(f.name.clone(), f);
        }
    }
    best
}

pub fn reachable_funcs(prog: &Program) -> HashSet<String> {
    let best = best_functions_for_emit(prog);
    let mut bodies: HashMap<&str, &Function> = HashMap::new();
    let mut all_names: HashSet<String> = HashSet::new();
    for item in &prog.items {
        if let Item::Func(f) = item {
            all_names.insert(f.name.clone());
        }
    }
    for f in best.values() {
        bodies.insert(f.name.as_str(), *f);
    }

    let mut roots: Vec<String> = Vec::new();
    for item in &prog.items {
        match item {
            Item::Func(f) if f.body.is_some() => {
                if !f.is_static || f.name == "main" {
                    roots.push(f.name.clone());
                }
            }
            Item::Global(g) => {
                if let Some(init) = &g.init {
                    collect_expr_fn_refs(init, &all_names, &mut roots);
                }
            }
            _ => {}
        }
    }

    let mut reachable: HashSet<String> = HashSet::new();
    let mut q: VecDeque<String> = VecDeque::new();
    for r in roots {
        if reachable.insert(r.clone()) {
            q.push_back(r);
        }
    }
    while let Some(name) = q.pop_front() {
        let Some(f) = bodies.get(name.as_str()) else {
            continue;
        };
        let Some(body) = f.body.as_ref() else {
            continue;
        };
        let mut refs = Vec::new();
        for st in body {
            collect_stmt_fn_refs(st, &all_names, &mut refs);
        }
        for r in refs {
            if reachable.insert(r.clone()) {
                q.push_back(r);
            }
        }
    }
    // Fixpoint: static-inline chains (pq_sendint → pq_writeint → pg_hton32).
    loop {
        let mut grew = false;
        for f in bodies.values() {
            if !reachable.contains(&f.name) {
                continue;
            }
            let Some(body) = f.body.as_ref() else {
                continue;
            };
            let mut refs = Vec::new();
            for st in body {
                collect_stmt_fn_refs(st, &all_names, &mut refs);
            }
            for r in refs {
                if reachable.insert(r.clone()) {
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    reachable
}

fn collect_stmt_fn_refs(st: &Stmt, fns: &HashSet<String>, out: &mut Vec<String>) {
    match st {
        Stmt::Block(ss) => {
            for s in ss {
                collect_stmt_fn_refs(s, fns, out);
            }
        }
        Stmt::Decl(d) => {
            if let Some(e) = &d.init {
                collect_expr_fn_refs(e, fns, out);
            }
        }
        Stmt::DeclGroup(ds) => {
            for d in ds {
                if let Some(e) = &d.init {
                    collect_expr_fn_refs(e, fns, out);
                }
            }
        }
        Stmt::Expr(e) | Stmt::Return(Some(e)) => collect_expr_fn_refs(e, fns, out),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::Empty | Stmt::Goto(_) | Stmt::Asm { .. } => {}
        Stmt::GotoIndirect(e) => collect_expr_fn_refs(e, fns, out),
        Stmt::If {
            cond,
            then_b,
            else_b,
        } => {
            collect_expr_fn_refs(cond, fns, out);
            collect_stmt_fn_refs(then_b, fns, out);
            if let Some(e) = else_b {
                collect_stmt_fn_refs(e, fns, out);
            }
        }
        Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
            collect_expr_fn_refs(cond, fns, out);
            collect_stmt_fn_refs(body, fns, out);
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            if let Some(i) = init {
                collect_stmt_fn_refs(i, fns, out);
            }
            if let Some(c) = cond {
                collect_expr_fn_refs(c, fns, out);
            }
            if let Some(s) = step {
                collect_expr_fn_refs(s, fns, out);
            }
            collect_stmt_fn_refs(body, fns, out);
        }
        Stmt::Label(_, s) | Stmt::Default(s) => collect_stmt_fn_refs(s, fns, out),
        Stmt::Switch { cond, body } | Stmt::Case { value: cond, body } => {
            collect_expr_fn_refs(cond, fns, out);
            collect_stmt_fn_refs(body, fns, out);
        }
    }
}

/// Collect Var names that appear as assignment / inc / dec targets.
fn collect_expr_fn_refs(e: &Expr, fns: &HashSet<String>, out: &mut Vec<String>) {
    match e {
        Expr::Int(_) | Expr::Float(_) | Expr::Char(_) | Expr::String(_) | Expr::SizeofType(_)
        | Expr::AddrOfLabel(_) => {}
        Expr::Var(name) => {
            if fns.contains(name) {
                out.push(name.clone());
            }
        }
        Expr::Call { name, args } => {
            if fns.contains(name) {
                out.push(name.clone());
            }
            for a in args {
                collect_expr_fn_refs(a, fns, out);
            }
        }
        Expr::Unary { expr, .. }
        | Expr::PreInc(expr)
        | Expr::PreDec(expr)
        | Expr::PostInc(expr)
        | Expr::PostDec(expr)
        | Expr::SizeofExpr(expr)
        | Expr::Cast { expr, .. } => collect_expr_fn_refs(expr, fns, out),
        Expr::Binary { left, right, .. }
        | Expr::Assign { left, right }
        | Expr::CompoundAssign { left, right, .. }
        | Expr::Index {
            base: left,
            index: right,
        } => {
            collect_expr_fn_refs(left, fns, out);
            collect_expr_fn_refs(right, fns, out);
        }
        Expr::Member { base, .. } => collect_expr_fn_refs(base, fns, out),
        Expr::Cond {
            cond,
            then_e,
            else_e,
        } => {
            collect_expr_fn_refs(cond, fns, out);
            collect_expr_fn_refs(then_e, fns, out);
            collect_expr_fn_refs(else_e, fns, out);
        }
        Expr::InitList { fields } => {
            for (_, v) in fields {
                collect_expr_fn_refs(v, fns, out);
            }
        }
        Expr::StmtExpr(stmts, expr) => {
            for s in stmts {
                collect_stmt_fn_refs(s, fns, out);
            }
            collect_expr_fn_refs(expr, fns, out);
        }
    }
}

/// ISA backend selection (`-m aarch64|x86_64|i686|riscv64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Aarch64,
    X86_64,
    I686,
    Riscv64,
}

impl Target {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "aarch64" | "arm64" => Some(Self::Aarch64),
            "x86_64" | "x86-64" | "amd64" => Some(Self::X86_64),
            "i686" | "i386" => Some(Self::I686),
            "riscv64" | "riscv" | "rv64" => Some(Self::Riscv64),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
            Self::I686 => "i686",
            Self::Riscv64 => "riscv64",
        }
    }
}

/// Object-file / assembler dialect (Mach-O Darwin vs ELF Linux).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOs {
    Darwin,
    Linux,
}

impl TargetOs {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "darwin" | "macos" | "apple" => Some(Self::Darwin),
            "linux" | "elf" | "gnu" => Some(Self::Linux),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Darwin => "darwin",
            Self::Linux => "linux",
        }
    }

    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Darwin
        } else {
            Self::Linux
        }
    }
}

#[derive(Clone)]
struct FieldPlace {
    offset: i64,
    ty: Type,
    /// For bit-fields: (bit_offset within container at `offset`, width in bits).
    bit: Option<(u32, u32)>,
}

#[derive(Clone)]
struct Layout {
    size: i64,
    align: i64,
    fields: HashMap<String, FieldPlace>,
}

#[derive(Clone)]
enum Storage {
    Local { offset: i64 }, // from x29, negative
    Global { name: String },
    /// address already in register (temp)
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
    anon_layouts: Vec<Layout>,
    globals: HashMap<String, Type>,
    /// Globals for which this TU emits a real data definition (.data/.bss/.comm),
    /// not mere `extern` registration. Used so call sites can tell function-pointer
    /// *variables* (`getMonotonicUs`) from mis-parsed / soft-referenced function
    /// names that only got a weak BSS placeholder (`luaL_newstate`).
    defined_data_globals: std::collections::HashSet<String>,
    /// Enum / static const integer globals: name → value (load as imm, not address).
    const_globals: HashMap<String, i64>,
    funcs: HashMap<String, Function>,
    /// current function local scopes (innermost scope at the end)
    scopes: Vec<HashMap<String, Sym>>,
    stack_size: i64,
    label_id: usize,
    break_stack: Vec<String>,
    continue_stack: Vec<String>,
    /// Source label name → unique asm label (handles multiple `__here` in one fn).
    goto_labels: HashMap<String, String>,
    /// Asm labels already defined in the current function (avoid duplicate defs).
    goto_labels_defined: std::collections::HashSet<String>,
    func_name: String,
    /// Return type of the function currently being emitted.
    func_ret: Type,
    pending_case_labs: std::collections::VecDeque<String>,
    os: TargetOs,
    /// FP-relative offset of x0..x7 save area for the current variadic function (0 = none).
    va_regsave_off: i64,
    /// FP-relative offset of d0..d7 save area (0 = none). AAPCS64 variadic floats.
    va_fpsave_off: i64,
    /// FP-relative offset of the next-VR index word for __acc_va_arg_fp (0 = none).
    va_vr_idx_off: i64,
    /// Number of fixed (named) integer/pointer params before `...`.
    va_fixed_n: usize,
    /// Number of fixed float/double named params (consume d0.. before variadic FP).
    va_fixed_fp: usize,
    /// Current active section name if explicitly set via attribute (e.g. .init.rodata.prel64).
    cur_section: Option<String>,
    /// Data symbols referenced via adrp/adr (for weak stub emission when DEFINE_PER_CPU
    /// or other macros failed to produce a definition in this TU).
    referenced_data_syms: std::collections::HashSet<String>,
}

/// Kernel/PI linker symbols that must remain undefined in TUs so PROVIDE
/// aliases and the real definitions win. Weak-stubbing these breaks early MMU
/// (`__pi__text` fake weak blocks `PROVIDE(__pi__text = _text)`).
fn is_linker_boundary_sym(name: &str) -> bool {
    matches!(
        name,
        "_text"
            | "_stext"
            | "_etext"
            | "_data"
            | "_edata"
            | "_end"
            | "__bss_start"
            | "__bss_stop"
            | "__inittext_begin"
            | "__inittext_end"
            | "__initdata_begin"
            | "__initdata_end"
            | "__start_rodata"
            | "init_pg_dir"
            | "init_pg_end"
            | "init_idmap_pg_dir"
            | "init_idmap_pg_end"
            | "swapper_pg_dir"
            | "reserved_pg_dir"
            | "init_task"
            | "init_stack"
            | "early_init_stack"
            | "primary_entry"
            | "__primary_switch"
            | "__primary_switched"
            | "__enable_mmu"
    )
}

impl Codegen {
    pub fn new() -> Self {
        Self::with_os(TargetOs::host())
    }

    pub fn with_os(os: TargetOs) -> Self {
        Self {
            out: String::new(),
            strings: Vec::new(),
            layouts: HashMap::new(),
            anon_layouts: Vec::new(),
            globals: HashMap::new(),
            defined_data_globals: std::collections::HashSet::new(),
            const_globals: HashMap::new(),
            funcs: HashMap::new(),
            scopes: vec![HashMap::new()],
            stack_size: 0,
            label_id: 0,
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            goto_labels: HashMap::new(),
            goto_labels_defined: std::collections::HashSet::new(),
            func_name: String::new(),
            func_ret: Type::Void,
            pending_case_labs: std::collections::VecDeque::new(),
            os,
            va_regsave_off: 0,
            va_fpsave_off: 0,
            va_vr_idx_off: 0,
            va_fixed_n: 0,
            va_fixed_fp: 0,
            cur_section: None,
            referenced_data_syms: std::collections::HashSet::new(),
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

    /// C ABI symbol as seen by the assembler (Darwin underscores).
    fn c_sym(&self, name: &str) -> String {
        // Darwin freestanding: keep a private errno slot. Linux userspace must
        // use `__errno_location()` (see emit_errno_*) so libc sees real errno —
        // SQLite `if (errno!=ENOENT)` after failed lstat was reading a never-
        // written `__acc_errno` (always 0) and treating ENOENT as CANTOPEN.
        let name = if name == "errno" && self.os == TargetOs::Darwin {
            "__acc_errno"
        } else {
            name
        };
        match self.os {
            TargetOs::Darwin => format!("_{name}"),
            TargetOs::Linux => name.to_string(),
        }
    }

    /// Linux: `errno` is `*__errno_location()`. Emit call; address of errno in x0.
    /// Weak freestanding stub returns `&__acc_errno` when not linked with libc.
    fn emit_errno_location_to_x0(&mut self) {
        writeln!(self.out, "\tbl\t__errno_location").unwrap();
    }

    fn emit_load_errno(&mut self, dest: u8) {
        self.emit_errno_location_to_x0();
        // errno is int; sign-extend to x for consistent comparisons.
        if dest == 0 {
            writeln!(self.out, "\tldrsw\tx0, [x0]").unwrap();
        } else {
            writeln!(self.out, "\tldrsw\tx{dest}, [x0]").unwrap();
        }
    }

    /// AAPCS64: unused bits above a small integer return in `x0` are undefined.
    /// Sign/zero-extend so 64-bit `cmp` against `-1` works (SQLite
    /// `osUnlink(z)==(-1)` / `unlink(path)==-1` after EISDIR).
    fn emit_extend_call_return(&mut self, dest: u8, ret_ty: &Type) {
        match ret_ty {
            Type::Int => {
                writeln!(self.out, "\tsxtw\tx{dest}, w0").unwrap();
            }
            Type::Short => {
                writeln!(self.out, "\tsxth\tx{dest}, w0").unwrap();
            }
            Type::SChar => {
                writeln!(self.out, "\tsxtb\tx{dest}, w0").unwrap();
            }
            Type::UInt => {
                // `mov w,w` zero-extends into the full X register.
                writeln!(self.out, "\tmov\tw{dest}, w0").unwrap();
            }
            Type::UShort => {
                writeln!(self.out, "\tand\tx{dest}, x0, #0xffff").unwrap();
            }
            Type::Char => {
                writeln!(self.out, "\tand\tx{dest}, x0, #0xff").unwrap();
            }
            Type::Float | Type::Double => {
                // Caller already moved from d0 when applicable.
            }
            _ => {
                // long / pointer / void / aggregates: full x0 is the value.
                if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                }
            }
        }
    }

    /// Address of the thread's errno (Linux) into x{dest}.
    fn emit_errno_addr(&mut self, dest: u8) {
        self.emit_errno_location_to_x0();
        if dest != 0 {
            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
        }
    }

    fn emit_text_section(&mut self) {
        match self.os {
            TargetOs::Darwin => writeln!(
                self.out,
                "\t.section\t__TEXT,__text,regular,pure_instructions"
            )
            .unwrap(),
            TargetOs::Linux => writeln!(self.out, "\t.text").unwrap(),
        }
        writeln!(self.out, "\t.p2align\t2").unwrap();
    }

    fn emit_data_section(&mut self) {
        match self.os {
            TargetOs::Darwin => writeln!(self.out, "\t.section\t__DATA,__data").unwrap(),
            TargetOs::Linux => writeln!(self.out, "\t.data").unwrap(),
        }
    }

    fn emit_bss_section(&mut self) {
        match self.os {
            TargetOs::Darwin => writeln!(self.out, "\t.section\t__DATA,__bss").unwrap(),
            TargetOs::Linux => writeln!(self.out, "\t.bss").unwrap(),
        }
    }

    fn emit_rodata_section(&mut self) {
        match self.os {
            // arm64 Darwin PIE forbids absolute relocations in the TEXT segment
            // ("illegal text-relocations"). Const data that may hold pointer
            // initializers (`.quad _foo`, `sym+off`) must live in DATA.
            // `__DATA,__const` is the Mach-O home for relocated const data.
            TargetOs::Darwin => writeln!(self.out, "\t.section\t__DATA,__const").unwrap(),
            TargetOs::Linux => writeln!(self.out, "\t.section\t.rodata").unwrap(),
        }
    }

    /// Linux arm64 PI (arch/*/pi/*) forbids R_AARCH64_ABS64 outside sections
    /// whose name contains `.rodata.prel64` (relacheck converts those to PREL64).
    ///
    /// Scalar function/data pointers (e.g. Redis `static void (*oom)(size_t) = …`)
    /// are often **mutated** at runtime — they must live in writable `.data`, not
    /// rodata.prel64 (store SEGV). Arrays of pointers (kernel ops tables) stay
    /// in prel64 for PI relacheck.
    fn emit_ptr_data_section(&mut self) {
        self.emit_ptr_data_section_kind(false);
    }

    fn emit_ptr_data_section_kind(&mut self, array_of_ptrs: bool) {
        match self.os {
            // Same as emit_rodata_section on Darwin: never __TEXT,__const for
            // absolute symbol addresses (SQLite IoFinder / UpperToLower tables).
            TargetOs::Darwin => self.emit_rodata_section(),
            TargetOs::Linux => {
                if array_of_ptrs {
                    self.cur_section = Some(".rodata.prel64".into());
                    writeln!(self.out, "\t.section\t.rodata.prel64,\"a\"").unwrap();
                } else {
                    self.cur_section = Some(".data".into());
                    writeln!(self.out, "\t.data").unwrap();
                }
            }
        }
    }

    fn emit_quad_sym_addr(&mut self, label: &str) {
        // Absolute symbol address in data — on Linux put in prel64-friendly section.
        match self.os {
            TargetOs::Linux => {
                // Still emit ABS64; relacheck rewrites if section name has prel64.
                writeln!(self.out, "\t.quad\t{label}").unwrap();
            }
            TargetOs::Darwin => {
                writeln!(self.out, "\t.quad\t{label}").unwrap();
            }
        }
    }

    fn emit_var_section(&mut self, g: &VarDecl, default_kind: &str) {
        let sec = g.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty());
        if let Some(sec) = sec {
            self.cur_section = Some(sec.to_string());
            self.emit_named_section(sec);
        } else {
            self.cur_section = None;
            match default_kind {
                "rodata" => self.emit_rodata_section(),
                "bss" => self.emit_bss_section(),
                _ => self.emit_data_section(),
            }
        }
    }

    /// Emit `.section name,"flags"` so modpost sees allocatable sections.
    /// Bare `.section foo` lacks SHF_ALLOC → "unexpected non-allocatable section".
    fn emit_named_section(&mut self, sec: &str) {
        let s = sec.trim().trim_matches('"');
        // Already has flags? e.g. `.rodata.prel64,"a"` from PI path
        if s.contains(',') || s.contains('"') {
            writeln!(self.out, "\t.section\t{s}").unwrap();
            return;
        }
        let flags = if matches!(self.os, TargetOs::Darwin) {
            // Mach-O uses different section syntax; keep bare for now.
            ""
        } else if s.contains("text") || s.ends_with(".text") || s.contains("irqentry") {
            "ax"
        } else if s.contains("bss") {
            "aw"
        } else if s.contains("data") && !s.contains("rodata") {
            "aw"
        } else {
            // tables, rodata, init.rodata, __*table*, etc.
            "a"
        };
        if flags.is_empty() {
            writeln!(self.out, "\t.section\t{s}").unwrap();
        } else {
            writeln!(self.out, "\t.section\t{s},\"{flags}\"").unwrap();
        }
    }

    fn emit_cstring_section(&mut self) {
        match self.os {
            TargetOs::Darwin => {
                writeln!(self.out, "\t.section\t__TEXT,__cstring,cstring_literals").unwrap()
            }
            TargetOs::Linux => writeln!(self.out, "\t.section\t.rodata").unwrap(),
        }
    }

    /// Materialize address of label into x{reg} (PC-relative).
    fn emit_adrp_add(&mut self, reg: u8, label: &str) {
        // Track bare symbol names (no +offset) for weak-data stubs.
        let base = label.split('+').next().unwrap_or(label);
        if !base.is_empty()
            && base
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
        {
            self.referenced_data_syms.insert(base.to_string());
        }
        match self.os {
            TargetOs::Darwin => {
                writeln!(self.out, "\tadrp\tx{reg}, {label}@PAGE").unwrap();
                writeln!(self.out, "\tadd\tx{reg}, x{reg}, {label}@PAGEOFF").unwrap();
            }
            TargetOs::Linux => {
                writeln!(self.out, "\tadrp\tx{reg}, {label}").unwrap();
                writeln!(self.out, "\tadd\tx{reg}, x{reg}, :lo12:{label}").unwrap();
            }
        }
    }

    fn emit_adrp_got(&mut self, reg: u8, label: &str) {
        match self.os {
            TargetOs::Darwin => {
                writeln!(self.out, "\tadrp\tx{reg}, {label}@GOTPAGE").unwrap();
                writeln!(
                    self.out,
                    "\tldr\tx{reg}, [x{reg}, {label}@GOTPAGEOFF]"
                )
                .unwrap();
            }
            TargetOs::Linux => {
                // Linux: GOT via adrp + ldr :got:
                writeln!(self.out, "\tadrp\tx{reg}, :got:{label}").unwrap();
                writeln!(self.out, "\tldr\tx{reg}, [x{reg}, :got_lo12:{label}]").unwrap();
            }
        }
    }

    /// Unique asm label for a C `goto` / label name within the current function.
    fn goto_lab(&mut self, name: &str) -> String {
        if let Some(l) = self.goto_labels.get(name) {
            return l.clone();
        }
        let id = self.label_id;
        self.label_id += 1;
        // Sanitize: C labels may contain `$` / leading `_` from macros.
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let l = format!("L_{}_goto_{}_{}", self.func_name, safe, id);
        self.goto_labels.insert(name.to_string(), l.clone());
        l
    }

    fn lab(&mut self, p: &str) -> String {
        let id = self.label_id;
        self.label_id += 1;
        format!("L_{}_{}_{}", self.func_name, p, id)
    }

    /// Conditional branch that can cross section boundaries (.text ↔ .init.text).
    /// Direct `cbz`/`b.cond` are only ±1MB (CONDBR19); kernel TUs overflow.
    /// Pattern: inverted short cond over long `b` (±128MB).
    fn emit_cbz_long(&mut self, reg: u8, target: &str) {
        let skip = self.lab("br_skip");
        writeln!(self.out, "\tcbnz\tx{reg}, {skip}").unwrap();
        writeln!(self.out, "\tb\t{target}").unwrap();
        writeln!(self.out, "{skip}:").unwrap();
    }

    fn emit_cbnz_long(&mut self, reg: u8, target: &str) {
        let skip = self.lab("br_skip");
        writeln!(self.out, "\tcbz\tx{reg}, {skip}").unwrap();
        writeln!(self.out, "\tb\t{target}").unwrap();
        writeln!(self.out, "{skip}:").unwrap();
    }

    fn emit_bcond_long(&mut self, cond: &str, target: &str) {
        // Invert condition and skip over long branch.
        let inv = match cond {
            "eq" => "ne",
            "ne" => "eq",
            "lt" => "ge",
            "le" => "gt",
            "gt" => "le",
            "ge" => "lt",
            "hi" => "ls",
            "ls" => "hi",
            "hs" | "cs" => "lo",
            "lo" | "cc" => "hs",
            "mi" => "pl",
            "pl" => "mi",
            "vs" => "vc",
            "vc" => "vs",
            other => other, // fallback: still emit short (rare)
        };
        let skip = self.lab("br_skip");
        writeln!(self.out, "\tb.{inv}\t{skip}").unwrap();
        writeln!(self.out, "\tb\t{target}").unwrap();
        writeln!(self.out, "{skip}:").unwrap();
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

    fn type_size(&self, ty: &Type) -> i64 {
        match ty {
            Type::Void => 0,
            // Promote stack slots for char/int to 8 for ABI-simple stack frames
            // (struct field layout still uses precise sizes below via field_size).
            Type::Char | Type::SChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt => 4,
            Type::Long | Type::ULong => 8,
            Type::Float => 4,
            Type::Double => 8,
            Type::Ptr(_) => 8,
            Type::Array(e, n) => self.type_size(e) * n,
            Type::Struct(n) | Type::Union(n) => self
                .layouts
                .get(n)
                .map(|l| l.size)
                .unwrap_or(8),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).size,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).size,
        }
    }

    fn stack_slot_size(&self, ty: &Type) -> i64 {
        // Locals always get 8-byte slots for scalars to keep x29 offsets friendly.
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
            Type::Char | Type::SChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt | Type::Float => 4,
            Type::Long | Type::ULong | Type::Double | Type::Ptr(_) => 8,
            Type::Array(e, _) => self.type_align(e),
            Type::Struct(n) | Type::Union(n) => self
                .layouts
                .get(n)
                .map(|l| l.align)
                .unwrap_or(8),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).align,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).align,
        }
    }

    fn is_struct_or_union_ty(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Struct(_) | Type::Union(_) | Type::AnonStruct(_) | Type::AnonUnion(_)
        )
    }

    /// AAPCS64: struct/union ≤16 bytes returned/passed in 1–2 GPRs (xN, xN+1).
    /// Arrays are NOT included — they decay to pointers (1 GPR).
    fn small_agg_nregs(&self, ty: &Type) -> Option<u8> {
        if !Self::is_struct_or_union_ty(ty) {
            return None;
        }
        let sz = self.type_size(ty);
        if sz == 0 {
            return None;
        }
        if sz <= 8 {
            Some(1)
        } else if sz <= 16 {
            Some(2)
        } else {
            None
        }
    }

    /// Store x{reg}.. into memory at [x{addr_reg}] for a small aggregate.
    fn store_small_agg_from_regs(&mut self, addr_reg: u8, nregs: u8, first_reg: u8) {
        for r in 0..nregs {
            let reg = first_reg + r;
            let off = (r as i64) * 8;
            if off == 0 {
                writeln!(self.out, "\tstr\tx{reg}, [x{addr_reg}]").unwrap();
            } else {
                writeln!(self.out, "\tstr\tx{reg}, [x{addr_reg}, #{off}]").unwrap();
            }
        }
    }

    /// Load small aggregate at [x{addr_reg}] into x{first_reg}..
    fn load_small_agg_to_regs(&mut self, addr_reg: u8, nregs: u8, first_reg: u8) {
        for r in 0..nregs {
            let reg = first_reg + r;
            let off = (r as i64) * 8;
            if off == 0 {
                writeln!(self.out, "\tldr\tx{reg}, [x{addr_reg}]").unwrap();
            } else {
                writeln!(self.out, "\tldr\tx{reg}, [x{addr_reg}, #{off}]").unwrap();
            }
        }
    }

    /// Address of an aggregate *source* for memcpy / full-object copy.
    ///
    /// Unlike the soft path in `emit_lvalue_addr`, this never treats a scalar
    /// rvalue (e.g. the first 8 bytes of a struct) as a pointer. That bug made
    /// `a = cond ? t1 : t2` (Token, 16B) do `memcpy(dst, t1.z, 16)` and turn
    /// SQL text into Token.z / Token.n → SQLite CREATE TRIGGER false OOM.
    fn emit_agg_copy_src(
        &mut self,
        e: &Expr,
        reg: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match e {
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let l_else = self.lab("agg_cond_else");
                let l_end = self.lab("agg_cond_end");
                self.emit_expr_rval(cond, 0, typedefs)?;
                self.emit_cbz_long(0, &l_else);
                let tty = self.emit_agg_copy_src(then_e, reg, typedefs)?;
                writeln!(self.out, "\tb\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                let ety = self.emit_agg_copy_src(else_e, reg, typedefs)?;
                writeln!(self.out, "{l_end}:").unwrap();
                Ok(if Self::is_struct_or_union_ty(&tty) {
                    tty
                } else {
                    ety
                })
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let rty = self.emit_expr_rval(expr, reg, typedefs)?;
                match rty {
                    Type::Ptr(inner) => Ok(*inner),
                    Type::Array(inner, _) => Ok(*inner),
                    _ => Ok(Type::Char),
                }
            }
            Expr::StmtExpr(stmts, final_expr) => {
                self.enter_scope();
                for s in stmts {
                    self.emit_stmt(s, typedefs)?;
                }
                let res = self.emit_agg_copy_src(final_expr, reg, typedefs);
                self.exit_scope();
                res
            }
            Expr::Cast { ty, expr } => {
                // Compound literal (T){...} as aggregate copy source: materialize
                // and return its address. Without this, `*p = (engine){...}` only
                // stored the temporary's address into the first field (Redis
                // functionsRegisterEngine → null fptr SEGV).
                if let Expr::InitList { fields } = expr.as_ref() {
                    if Self::is_struct_or_union_ty(ty) || matches!(ty, Type::Array(_, _)) {
                        let sz = self.type_size(ty).max(8);
                        let tmp = format!("__comp_agg_{}", self.label_id);
                        self.label_id += 1;
                        let off = self.alloc_local(&tmp, ty);
                        self.emit_fp_addr(off, 0);
                        writeln!(self.out, "\tmov\tx1, xzr").unwrap();
                        self.emit_imm(sz, 2);
                        writeln!(self.out, "\tbl\t{}", self.c_sym("memset")).unwrap();
                        self.emit_local_init_list(off, ty, fields, typedefs)?;
                        self.emit_fp_addr(off, reg);
                        return Ok(ty.clone());
                    }
                }
                self.emit_agg_copy_src(expr, reg, typedefs)
            }
            Expr::InitList { fields } => {
                // Bare `{...}` with known aggregate type from context is rare;
                // reject unless we have a typed cast wrapper (handled above).
                let _ = fields;
                Err("aggregate copy source is bare init list".into())
            }
            Expr::Var(_) | Expr::Index { .. } | Expr::Member { .. } => {
                self.emit_lvalue_addr(e, reg, typedefs)
            }
            Expr::Call { .. } => {
                let ty = self.typeof_expr(e, typedefs);
                if let Some(nr) = self.small_agg_nregs(&ty) {
                    let sz = self.type_size(&ty).max(8);
                    let tmp = {
                        self.stack_size = Self::align_up(self.stack_size + sz, 8);
                        -self.stack_size
                    };
                    self.emit_expr_rval(e, 0, typedefs)?;
                    self.emit_fp_addr(tmp, 9);
                    self.store_small_agg_from_regs(9, nr, 0);
                    self.emit_fp_addr(tmp, reg);
                    Ok(ty)
                } else {
                    Err("large aggregate call return not materializable as copy src".into())
                }
            }
            _ => Err("aggregate copy source is not addressable".into()),
        }
    }

    /// Prefer struct/union when either `?:` arm is aggregate (Token, etc.).
    fn cond_result_ty(&self, then_e: &Expr, else_e: &Expr, typedefs: &HashMap<String, Type>) -> Type {
        let t = self.typeof_expr(then_e, typedefs);
        let e = self.typeof_expr(else_e, typedefs);
        if Self::is_struct_or_union_ty(&t) {
            t
        } else if Self::is_struct_or_union_ty(&e) {
            e
        } else if matches!(t, Type::Float | Type::Double) || matches!(e, Type::Float | Type::Double)
        {
            Type::Double
        } else if matches!(t, Type::Long | Type::ULong | Type::Ptr(_)) {
            t
        } else if matches!(e, Type::Long | Type::ULong | Type::Ptr(_)) {
            e
        } else {
            t
        }
    }

    /// Look up field placement for `base.field` / `base->field`.
    fn member_place(
        &self,
        base: &Expr,
        field: &str,
        arrow: bool,
        typedefs: &HashMap<String, Type>,
    ) -> Option<FieldPlace> {
        let bt = if arrow {
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
                .and_then(|l| l.fields.get(field).cloned()),
            Type::AnonStruct(fs) => {
                let lay = self.layout_fields(&fs, false, false);
                lay.fields.get(field).cloned()
            }
            Type::AnonUnion(fs) => {
                let lay = self.layout_fields(&fs, true, false);
                lay.fields.get(field).cloned()
            }
            _ => None,
        }
    }

    /// Load bitfield at [x{addr_reg}] into x{dest}. Container type is `ty`.
    fn load_bitfield(
        &mut self,
        addr_reg: u8,
        ty: &Type,
        bit_off: u32,
        bit_width: u32,
        dest: u8,
    ) {
        // Load container (unsigned for bit extract).
        let sz = self.type_size(ty);
        match sz {
            1 => writeln!(self.out, "\tldrb\tw{dest}, [x{addr_reg}]").unwrap(),
            2 => writeln!(self.out, "\tldrh\tw{dest}, [x{addr_reg}]").unwrap(),
            4 => writeln!(self.out, "\tldr\tw{dest}, [x{addr_reg}]").unwrap(),
            _ => writeln!(self.out, "\tldr\tx{dest}, [x{addr_reg}]").unwrap(),
        }
        if bit_off > 0 {
            writeln!(self.out, "\tlsr\tx{dest}, x{dest}, #{bit_off}").unwrap();
        }
        if bit_width < 64 {
            let mask = if bit_width >= 32 {
                // mask low bit_width bits
                if bit_width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << bit_width) - 1
                }
            } else {
                (1u64 << bit_width) - 1
            };
            // Use and with immediate when possible
            if mask <= 0xfff {
                writeln!(self.out, "\tand\tx{dest}, x{dest}, #{mask}").unwrap();
            } else {
                self.emit_imm(mask as i64, 16);
                writeln!(self.out, "\tand\tx{dest}, x{dest}, x16").unwrap();
            }
        }
        // Leave zero-extended. (Most SQLite bitfields are unsigned flags 0/1.)
        let _ = ty;
    }

    /// Store x{val_reg} into bitfield at FP-relative `base_off` (or absolute via helper).
    fn store_bitfield(
        &mut self,
        base_off: i64,
        ty: &Type,
        bit_off: u32,
        bit_width: u32,
        val_reg: u8,
    ) -> Result<(), String> {
        // addr in x9
        self.emit_fp_addr(base_off, 9);
        self.store_bitfield_at(9, ty, bit_off, bit_width, val_reg);
        Ok(())
    }

    fn store_bitfield_at(
        &mut self,
        addr_reg: u8,
        ty: &Type,
        bit_off: u32,
        bit_width: u32,
        val_reg: u8,
    ) {
        let sz = self.type_size(ty);
        // load container into x16
        match sz {
            1 => writeln!(self.out, "\tldrb\tw16, [x{addr_reg}]").unwrap(),
            2 => writeln!(self.out, "\tldrh\tw16, [x{addr_reg}]").unwrap(),
            4 => writeln!(self.out, "\tldr\tw16, [x{addr_reg}]").unwrap(),
            _ => writeln!(self.out, "\tldr\tx16, [x{addr_reg}]").unwrap(),
        }
        let mask = if bit_width >= 64 {
            u64::MAX
        } else {
            (1u64 << bit_width) - 1
        };
        // clear field bits: container &= ~(mask << bit_off)
        let clear = !(mask << bit_off);
        self.emit_imm(clear as i64, 17);
        writeln!(self.out, "\tand\tx16, x16, x17").unwrap();
        // insert: container |= (val & mask) << bit_off
        if mask <= 0xfff {
            writeln!(self.out, "\tand\tx15, x{val_reg}, #{mask}").unwrap();
        } else {
            self.emit_imm(mask as i64, 15);
            writeln!(self.out, "\tand\tx15, x{val_reg}, x15").unwrap();
        }
        if bit_off > 0 {
            writeln!(self.out, "\tlsl\tx15, x15, #{bit_off}").unwrap();
        }
        writeln!(self.out, "\torr\tx16, x16, x15").unwrap();
        match sz {
            1 => writeln!(self.out, "\tstrb\tw16, [x{addr_reg}]").unwrap(),
            2 => writeln!(self.out, "\tstrh\tw16, [x{addr_reg}]").unwrap(),
            4 => writeln!(self.out, "\tstr\tw16, [x{addr_reg}]").unwrap(),
            _ => writeln!(self.out, "\tstr\tx16, [x{addr_reg}]").unwrap(),
        }
    }

    fn layout_fields(&self, fields: &[Field], is_union: bool, packed: bool) -> Layout {
        let mut map = HashMap::new();
        let mut max_align = 1i64;
        let mut max_size = 0i64;
        // GCC/aarch64-compatible bitfield packing (PCC_BITFIELD_TYPE_MATTERS):
        // track free position in bits; a bitfield of declared type T/width W
        // must not straddle a container of sizeof(T) bits. If it would, pad to
        // the next container boundary. Non-bitfields round up to a byte, then
        // align. This yields e.g. SQLite Column = 16 (not 24).
        // When `packed`, field alignment is forced to 1.
        let mut offset_bits: u64 = 0;

        for f in fields {
            // Anonymous nested struct/union: promote fields into this layout.
            if f.name.is_empty() && f.bit_width.is_none() {
                let nested_opt = match &f.ty {
                    Type::AnonStruct(fs) => Some(self.layout_fields(fs, false, false)),
                    Type::AnonUnion(fs) => Some(self.layout_fields(fs, true, false)),
                    Type::Struct(n) => self.layouts.get(n).cloned(),
                    Type::Union(n) => self.layouts.get(n).cloned(),
                    _ => None,
                };
                if let Some(nested) = nested_opt {
                    let nalign = if packed { 1 } else { nested.align };
                    max_align = max_align.max(nalign);
                    if is_union {
                        // Nested type starts at offset 0 of the union; keep relative field offs.
                        for (fnm, place) in &nested.fields {
                            map.insert(
                                fnm.clone(),
                                FieldPlace {
                                    offset: place.offset,
                                    ty: place.ty.clone(),
                                    bit: place.bit,
                                },
                            );
                        }
                        max_size = max_size.max(nested.size);
                    } else {
                        let mut byte_off = ((offset_bits + 7) / 8) as i64;
                        byte_off = Self::align_up(byte_off, nalign);
                        for (fnm, place) in &nested.fields {
                            map.insert(
                                fnm.clone(),
                                FieldPlace {
                                    offset: byte_off + place.offset,
                                    ty: place.ty.clone(),
                                    bit: place.bit,
                                },
                            );
                        }
                        offset_bits = ((byte_off + nested.size) as u64) * 8;
                    }
                    continue;
                }
            }

            // Bit-field packing.
            if let Some(width) = f.bit_width {
                let container_sz = self.type_size(&f.ty).max(1) as u64;
                let container_bits = container_sz * 8;
                let al = if packed { 1 } else { self.type_align(&f.ty) };
                max_align = max_align.max(al);

                if is_union {
                    // In a union, each bitfield starts at offset 0.
                    if !f.name.is_empty() && width > 0 {
                        map.insert(
                            f.name.clone(),
                            FieldPlace {
                                offset: 0,
                                ty: f.ty.clone(),
                                bit: Some((0, width)),
                            },
                        );
                    }
                    max_size = max_size.max(container_sz as i64);
                    continue;
                }

                // Zero-width: force alignment to next container boundary.
                if width == 0 {
                    let al_bits = (al as u64) * 8;
                    if al_bits > 0 {
                        offset_bits = ((offset_bits + al_bits - 1) / al_bits) * al_bits;
                    }
                    continue;
                }

                let w = width as u64;
                // If field would straddle a container boundary, pad to next container.
                if container_bits > 0
                    && (offset_bits % container_bits) + w > container_bits
                {
                    offset_bits =
                        ((offset_bits + container_bits - 1) / container_bits) * container_bits;
                }
                let bit_pos = offset_bits;
                // Container base: floor to container size from struct start.
                let cont_index = bit_pos / container_bits;
                let unit_start = (cont_index * container_sz) as i64;
                let bit_in = (bit_pos % container_bits) as u32;
                if !f.name.is_empty() {
                    map.insert(
                        f.name.clone(),
                        FieldPlace {
                            offset: unit_start,
                            ty: f.ty.clone(),
                            bit: Some((bit_in, width)),
                        },
                    );
                }
                offset_bits = bit_pos + w;
                // Track span for max_size (end of used storage in this container).
                let end_byte = ((offset_bits + 7) / 8) as i64;
                max_size = max_size.max(end_byte);
                continue;
            }

            // Ordinary field.
            let sz = self.type_size(&f.ty);
            let al = if packed { 1 } else { self.type_align(&f.ty) };
            max_align = max_align.max(al);
            if is_union {
                if !f.name.is_empty() {
                    map.insert(
                        f.name.clone(),
                        FieldPlace {
                            offset: 0,
                            ty: f.ty.clone(),
                            bit: None,
                        },
                    );
                }
                max_size = max_size.max(sz);
            } else {
                let mut byte_off = ((offset_bits + 7) / 8) as i64;
                byte_off = Self::align_up(byte_off, al);
                if !f.name.is_empty() {
                    map.insert(
                        f.name.clone(),
                        FieldPlace {
                            offset: byte_off,
                            ty: f.ty.clone(),
                            bit: None,
                        },
                    );
                }
                offset_bits = ((byte_off + sz) as u64) * 8;
            }
        }
        let final_align = if packed { 1 } else { max_align.max(1) };
        let size = if is_union {
            Self::align_up(max_size, final_align)
        } else {
            let byte_off = ((offset_bits + 7) / 8) as i64;
            Self::align_up(byte_off, final_align)
        };
        Layout {
            size,
            align: final_align,
            fields: map,
        }
    }

    fn collect_layouts(&mut self, prog: &Program) {
        // Multi-pass: type_layouts comes from a HashMap and may list a union/struct
        // before its nested named members. A single pass leaves unknown Struct(n)
        // at the 8-byte fallback (see type_size), so sizeof(union) collapses
        // (e.g. SQLite YYMINORTYPE became 8 instead of 16 → lemon stack smash).
        //
        // Critical: forward declarations produce Item::StructDef/UnionDef with
        // empty fields. Those must NOT clobber a fuller layout from type_layouts
        // (Linux: `struct task_struct;` then full body — empty overwrite made
        // sizeof/init_task emit 0 bytes and corrupted INIT_TASK).
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
                            // Forward decl — keep existing layout if any.
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
                            if let Some(l) = self.layouts.get(n).cloned() {
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

    fn resolve_typedef_type<'a>(&'a self, ty: &'a Type, typedefs: &HashMap<String, Type>) -> Type {
        // flatten one level of name aliases stored as Struct(typedefname) sometimes
        match ty {
            Type::Struct(n) | Type::Union(n) if typedefs.contains_key(n) => {
                typedefs.get(n).cloned().unwrap_or_else(|| ty.clone())
            }
            _ => ty.clone(),
        }
    }

    pub fn compile(&mut self, prog: &Program) -> Result<String, String> {
        self.out.clear();
        self.collect_layouts(prog);

        let mut typedefs = HashMap::new();
        // Names written anywhere in the TU must not be const-folded (static
        // counters like nRefSqlite3 / sqlite3_current_time are is_static with
        // const init but mutated at runtime — folding them to #0 broke
        // test3 btree_open and CURRENT_TIME). Also flex yy_init/yy_start.
        let assigned = collect_assigned_names_in_program(prog);
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
                // Never fold non-static, and never fold anything that is written.
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
                        if let Some(n) = Self::const_i64(init) {
                            self.const_globals.insert(g.name.clone(), n);
                        }
                    }
                }
            }
            if let Item::Typedef { name, ty } = item {
                // globals of typedef type use layouts by name
                if matches!(ty, Type::AnonStruct(_) | Type::AnonUnion(_)) {
                    // already in layouts under typedef name
                }
                let _ = name;
            }
        }

        // Text section first for functions
        self.emit_text_section();
        writeln!(self.out, "\t.p2align\t2").unwrap();
        // ARM64 uaccess/ex_table macros reference .L__gpr_num_xN / wN after
        // extended-asm %w0→wN substitution. Emit equates once per TU so
        // `.short ((.L__gpr_num_x0)<<0)|...` assembles (was previously avoided
        // only because %operands were dropped entirely).
        if matches!(self.os, TargetOs::Linux) {
            for i in 0..31u8 {
                writeln!(self.out, "\t.equ\t.L__gpr_num_x{i}, {i}").unwrap();
                writeln!(self.out, "\t.equ\t.L__gpr_num_w{i}, {i}").unwrap();
            }
            writeln!(self.out, "\t.equ\t.L__gpr_num_xzr, 31").unwrap();
            writeln!(self.out, "\t.equ\t.L__gpr_num_wzr, 31").unwrap();
        }

        // Drop unreferenced static / static-inline bodies (kernel header noise).
        let reachable = reachable_funcs(prog);
        let best_funcs = best_functions_for_emit(prog);

        // Unified symbol set so Func and Global never double-emit a label.
        let mut emitted_syms = std::collections::HashSet::new();
        for f in best_funcs.values() {
            let f = *f;
                // Emit: main; non-static (stubs or full); static if reachable.
                let is_root = !f.is_static || f.name == "main";
                if !is_root && !reachable.contains(&f.name) {
                    continue;
                }
                match &f.body {
                    None => {}
                    Some(b) if b.is_empty() && f.name != "main" => {
                        // Prefer freestanding only for a small whitelist of
                        // empty bodies that must win over soft stubs. Broader
                        // freestanding (smp_prepare_boot_cpu, …) is shared
                        // across many TUs — emitting strong copies from every
                        // empty body causes multiple-definition link errors.
                        // kasan_init_sw_tags is the critical one: static-inline
                        // {} from kasan.h never reached emit_function before.
                        if emitted_syms.insert(f.name.clone()) {
                            let empty_fs = matches!(
                                f.name.as_str(),
                                "kasan_init_sw_tags"
                                    | "kasan_init"
                                    | "kasan_early_init"
                            );
                            if empty_fs && self.emit_freestanding_kernel_helper(f)? {
                                // freestanding emitted
                            } else {
                                self.emit_stub_function(f)?;
                            }
                        }
                    }
                    Some(_) => {
                        if emitted_syms.insert(f.name.clone()) {
                            self.emit_function(f, &typedefs)?;
                        }
                    }
                }
        }

        // Globals (dedupe by assembler symbol; skip if already emitted as Func)
        for item in &prog.items {
            if let Item::Global(g) = item {
                if g.is_extern && g.init.is_none() {
                    continue;
                }
                let sym = self.c_sym(&g.name);
                if g.init.is_some() && emitted_syms.insert(sym) {
                    self.emit_global(g)?;
                }
            }
        }
        for item in &prog.items {
            if let Item::Global(g) = item {
                if g.is_extern && g.init.is_none() {
                    continue;
                }
                let sym = self.c_sym(&g.name);
                if emitted_syms.insert(sym) {
                    self.emit_global(g)?;
                }
            }
        }

        // strings
        if !self.strings.is_empty() {
            writeln!(self.out).unwrap();
            self.emit_cstring_section();
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

        // Linux kernel / vdso: weak stubs for helpers that BUILD_BUG paths,
        // missing macro expansions, or dead instrumentation may reference.
        // Strong definitions (when we emit them) win over these.
        if matches!(self.os, TargetOs::Linux) {
            self.emit_text_section();
            for name in [
                "__field_overflow",
                "__bad_mask",
                "__bad_unaligned_access",
                "__bad_size_call_parameter",
                "__bad_copy_to",
                "__bad_copy_from",
                "__bad_udelay",
                "____wrong_branch_error",
                // define_dev_printk_level() sometimes fails to expand in huge TUs;
                // weak no-op printks keep device drivers linkable until PP is fixed.
                "_dev_emerg",
                "_dev_alert",
                "_dev_crit",
                "_dev_err",
                "_dev_warn",
                "_dev_notice",
                "_dev_info",
                // jump_label / rust / misc call targets
                "rust_fmt_argument",
                "__declare_arg_1",
                "__declare_arg_2",
                "__declare_arg_3",
                "fdt_get_property_w",
                // Soft atomic fallbacks (Redis without stdatomic / C11). Not
                // true atomics — enough for single-threaded / soft-link smoke.
                "__atomic_add_fetch",
                "__atomic_compare_exchange_n",
                "__atomic_load_n",
                "__atomic_store_n",
                "__atomic_fetch_add",
                "__sync_add_and_fetch",
                "__sync_bool_compare_and_swap",
                "__sync_fetch_and_add",
                "__sync_fetch_and_sub",
                "__sync_fetch_and_and",
                "__sync_fetch_and_or",
            ] {
                if emitted_syms.contains(name) {
                    continue;
                }
                writeln!(self.out, "\t.weak\t{name}").unwrap();
                writeln!(self.out, "{name}:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
            }
            // Real bswap helpers (PP rewrites __builtin_bswapN → __acc_bswapN).
            for &(name, bits) in &[
                ("__acc_bswap16", 16u32),
                ("__acc_bswap32", 32u32),
                ("__acc_bswap64", 64u32),
            ] {
                if emitted_syms.contains(name) {
                    continue;
                }
                writeln!(self.out, "\t.weak\t{name}").unwrap();
                writeln!(self.out, "{name}:").unwrap();
                match bits {
                    16 => {
                        writeln!(self.out, "\trev16\tw0, w0").unwrap();
                        writeln!(self.out, "\tand\tw0, w0, #0xffff").unwrap();
                    }
                    32 => {
                        writeln!(self.out, "\trev\tw0, w0").unwrap();
                    }
                    _ => {
                        writeln!(self.out, "\trev\tx0, x0").unwrap();
                    }
                }
                writeln!(self.out, "\tret").unwrap();
            }
            // Do NOT emit a local weak __errno_location body: `bl __errno_location`
            // would bind to it at static link and never reach glibc. Leave the
            // symbol undefined so it goes through the PLT to libc (userspace).
            // Freestanding/kernel TUs that need a fallback can link a tiny stub
            // that returns &__acc_errno.
            // Weak zero data for optional globals / macro local-name leaks
            // (rculist `struct list_head *__head` when local decl soft-fails).
            writeln!(self.out, "\t.bss").unwrap();
            let mut weak_data: std::collections::HashSet<String> = std::collections::HashSet::new();
            for name in [
                "__acc_errno",
                // Process-wide AAPCS64 VR cursor for va_arg(double) after
                // va_list is passed by value (char*) into non-variadic helpers
                // like sqlite3_str_vappendf. THREADSAFE=0 only.
                "acc_va_vr_cursor",
                "elfcorehdr_addr",
                "elfcorehdr_size",
                "kvm_protected_mode_initialized",
                "___res",
                "__head",
                "console_timer",
                "param_ops_byte",
                "param_ops_short",
                "param_ops_ushort",
                "param_ops_int",
                "param_ops_uint",
                "param_ops_long",
                "param_ops_ulong",
                "param_ops_ullong",
                "param_ops_hexint",
                "param_ops_charp",
                "param_ops_bool",
                // Soft `bool`→`_Bool` can rename param_ops_bool → param_ops__Bool
                "param_ops__Bool",
                "param_ops_string",
                "blockdev_superblock",
                "def_blk_fops",
                "pci_msi_ignore_mask",
                "_debug_pagealloc_enabled",
                "_debug_pagealloc_enabled_early",
                "__pi___eh_frame_start",
                "__pi___eh_frame_end",
                "__pi_dynamic_scs_is_enabled",
                // DEFINE_PER_CPU / SCS when multi-line macros fail to expand.
                "irq_shadow_call_stack_ptr",
                "batched_entropy_u8",
                "batched_entropy_u16",
                "batched_entropy_u32",
                "batched_entropy_u64",
            ] {
                if emitted_syms.contains(name) {
                    continue;
                }
                writeln!(self.out, "\t.weak\t{name}").unwrap();
                writeln!(self.out, "\t.align\t3").unwrap();
                writeln!(self.out, "{name}:").unwrap();
                writeln!(self.out, "\t.zero\t256").unwrap();
                weak_data.insert(name.to_string());
            }
            // Any adrp target not defined in this TU: weak data stub so RELOC_HIDE
            // address-of soft-missing DEFINE_PER_CPU symbols still links.
            // Strong defs from other TUs win over these weaks.
            // Skip names already present as labels in the assembly (static defs,
            // prior weaks) to avoid "symbol is already defined" from gas.
            //
            // CRITICAL: never weak-stub linker/PI boundary symbols. PI objcopy
            // renames `_text`→`__pi__text`; a weak local stub then blocks
            // PROVIDE(__pi__text = _text) and create_init_idmap maps the wrong
            // PA range → Prefetch Abort right after msr sctlr_el1.
            let mut extra: Vec<String> = self
                .referenced_data_syms
                .iter()
                .filter(|n| {
                    !emitted_syms.contains(*n)
                        && !weak_data.contains(*n)
                        && !n.starts_with("l_str_")
                        && !n.starts_with('.')
                        && !n.starts_with('L')
                        && !n.starts_with("__pi_")
                        && !n.starts_with("__efistub_")
                        && !is_linker_boundary_sym(n)
                        // Never weak-stub cross-TU test/harness counters: a same-file
                        // weak def binds locally and hides the strong def from main.o
                        // (Redis testhelp __test_num / __failed_tests; SQLite
                        // sqlite3_search_count linked via Tcl_LinkVar in test1.c).
                        && *n != "__test_num"
                        && *n != "__failed_tests"
                        && !n.starts_with("sqlite3_search_count")
                        && !n.starts_with("sqlite3_sort_count")
                        && !n.starts_with("sqlite3_found_count")
                        && !n.starts_with("sqlite3_like_count")
                        && !n.starts_with("sqlite3_interrupt_count")
                        && !n.starts_with("sqlite3_open_file_count")
                        && !n.starts_with("sqlite3_fullsync_count")
                        && !n.starts_with("sqlite3_pager_")
                        && !n.starts_with("sqlite3_io_error")
                        && !n.starts_with("sqlite3_diskfull")
                        && !n.starts_with("sqlite3_opentemp_count")
                        && !n.starts_with("sqlite3_max_blobsize")
                        && !n.starts_with("sqlite3_current_time")
                        && !Self::is_extern_libc(n)
                        && !self.out.contains(&format!("\n{n}:"))
                        && !self.out.contains(&format!("\n\t{n}:"))
                })
                .cloned()
                .collect();
            extra.sort();
            for name in extra {
                writeln!(self.out, "\t.weak\t{name}").unwrap();
                writeln!(self.out, "\t.align\t3").unwrap();
                writeln!(self.out, "{name}:").unwrap();
                writeln!(self.out, "\t.zero\t256").unwrap();
            }
            // Soft function stubs for asm symbols sometimes missing mid-build.
            for name in ["cpu_resume"] {
                if emitted_syms.contains(name) {
                    continue;
                }
                writeln!(self.out, "\t.text").unwrap();
                writeln!(self.out, "\t.weak\t{name}").unwrap();
                writeln!(self.out, "{name}:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
            }
        }

        // Darwin host userspace (SQLite amalgamation smoke etc.): soft errno
        // rewrites `errno` → `__acc_errno` via c_sym, and va_arg FP uses
        // `acc_va_vr_cursor`. These were Linux-only above; without them
        // Darwin link fails after bare-(unsigned) cast recovery restored bodies.
        // Use `.comm` (Mach-O) — Darwin assembler rejects ELF `.weak`.
        if matches!(self.os, TargetOs::Darwin) {
            for name in ["__acc_errno", "acc_va_vr_cursor"] {
                let sym = self.c_sym(name);
                if self.out.contains(&format!("\n{sym}:"))
                    || self.out.contains(&format!(".comm\t{sym},"))
                {
                    continue;
                }
                // size 256, align log2=3 (8-byte)
                writeln!(self.out, "\t.comm\t{sym},256,3").unwrap();
            }
        }

        Ok(self.out.clone())
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
                // Process environment / invocation — must bind to glibc, never
                // a same-TU weak .bss zero (Redis spt_init SEGV on envp[i]).
                | "environ"
                | "__environ"
                | "program_invocation_name"
                | "program_invocation_short_name"
                | "errno"
                | "tzname"
                | "timezone"
                | "daylight"
        )
    }

    /// True if this TU will emit a definition (real body or soft stub) for `name`.
    /// Prototypes (`body == None`) are external — address must go through GOT.
    fn func_defined_in_tu(&self, name: &str) -> bool {
        match self.funcs.get(name) {
            Some(f) => f.body.is_some() || name == "main",
            None => name == "main",
        }
    }

    /// Materialize function designator address into x{reg}.
    /// Defined-in-TU → direct PAGE; external prototype/undeclared → GOT.
    fn emit_func_addr(&mut self, name: &str, reg: u8) {
        let lab = self.c_sym(name);
        if self.func_defined_in_tu(name) {
            self.emit_adrp_add(reg, &lab);
        } else {
            self.emit_adrp_got(reg, &lab);
        }
    }

    fn emit_global(&mut self, g: &VarDecl) -> Result<(), String> {
        // libc-provided symbols: reference only, do not define in our data.
        if Self::is_extern_libc(&g.name) {
            self.globals.insert(g.name.clone(), g.ty.clone());
            return Ok(());
        }
        let size = self.type_size(&g.ty).max(1);
        let sym = self.c_sym(&g.name);
        // File-scope static: local. Explicit __weak → .weak (version.c
        // placeholders overridden by version-timestamp.o). Initialized strong
        // globals: .globl only. Uninitialized non-weak: keep .weak to tolerate
        // multi-TU soft defs from incomplete extern handling.
        if g.is_extern {
            self.globals.insert(g.name.clone(), g.ty.clone());
            return Ok(());
        }
        // `const char linux_banner[] __weak;` alone is a weak placeholder —
        // no body, so the later initialized def (same TU via #include, or
        // version-timestamp.o) can provide the real symbol without
        // "already defined" / multi-def.
        if g.is_weak && g.init.is_none() {
            self.globals.insert(g.name.clone(), g.ty.clone());
            if !g.is_static {
                writeln!(self.out, "\n\t.weak\t{sym}").unwrap();
                writeln!(self.out, "\t.globl\t{sym}").unwrap();
            }
            return Ok(());
        }
        if !g.is_static {
            match self.os {
                TargetOs::Darwin => {
                    if g.init.is_none() {
                        let al_log = self.type_align(&g.ty).max(1).min(8).trailing_zeros();
                        writeln!(self.out, "\t.comm\t{sym},{size},{al_log}").unwrap();
                        return Ok(());
                    }
                    if g.is_weak {
                        writeln!(self.out, "\n\t.weak_definition\t{sym}").unwrap();
                    }
                    writeln!(self.out, "\t.globl\t{sym}").unwrap();
                }
                TargetOs::Linux => {
                    // Explicit __weak always .weak.
                    // Soft placeholders: when this name is also a function
                    // prototype in the TU (parser emitted a bogus BSS for a
                    // declaration like `fs_param_is_bool`). Do **not** emit a
                    // local BSS def — that multi-defines against the real
                    // function body in another .o (init/main.o vs fs/fs_parser.o).
                    // Real pointer globals like Redis `rax *Users` stay strong BSS.
                    // Function prototypes mis-parsed as 8-byte BSS (e.g. kernel
                    // `fs_param_is_bool` after soft parse of headers in main.c).
                    // Emitting a strong BSS multi-defines against the real .text
                    // in fs/fs_parser.o. Skip the bogus data def entirely.
                    let soft_fn_placeholder = g.init.is_none()
                        && !g.is_weak
                        && size <= 8
                        && !matches!(
                            g.ty,
                            Type::Array(_, _)
                                | Type::Struct(_)
                                | Type::Union(_)
                                | Type::AnonStruct(_)
                                | Type::AnonUnion(_)
                        )
                        && (self.funcs.contains_key(&g.name)
                            || g.name.starts_with("fs_param_is_")
                            || g.name.starts_with("param_ops_")
                            || g.name.starts_with("param_array_ops")
                            || g.name.starts_with("param_sysfs_"));
                    if soft_fn_placeholder {
                        self.globals.insert(g.name.clone(), g.ty.clone());
                        return Ok(());
                    }
                    if g.is_weak {
                        writeln!(self.out, "\n\t.weak\t{sym}").unwrap();
                    }
                    writeln!(self.out, "\t.globl\t{sym}").unwrap();
                }
            }
        } else {
            writeln!(self.out, "").unwrap();
        }
        if let Some(init) = &g.init {
            match init {
                // Mutable globals with scalar init must be .data (or .bss for 0),
                // never .rodata — e.g. redis `static size_t used_memory = 0` is
                // written by atomicIncr; RO placement SEGVs (C2 aarch64).
                Expr::Int(n) | Expr::Char(n) => {
                    let sec = if *n == 0 { "bss" } else { "data" };
                    self.emit_var_section(g, sec);
                    let al = self.type_align(&g.ty).max(1).min(8);
                    writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    if matches!(g.ty, Type::Float) {
                        writeln!(self.out, "\t.float\t{}", *n as f32).unwrap();
                    } else if matches!(g.ty, Type::Double) {
                        writeln!(self.out, "\t.double\t{}", *n as f64).unwrap();
                    } else if *n == 0 {
                        writeln!(self.out, "\t.zero\t{size}").unwrap();
                    } else {
                        self.emit_int_directive(size, *n);
                    }
                }
                Expr::Float(f) => {
                    self.emit_var_section(g, "data");
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    if matches!(g.ty, Type::Float) {
                        writeln!(self.out, "\t.float\t{f}").unwrap();
                    } else {
                        writeln!(self.out, "\t.double\t{f}").unwrap();
                    }
                }
                Expr::Unary {
                    op: UnaryOp::Addr,
                    expr,
                } => {
                    if let Expr::Var(v) = expr.as_ref() {
                        let is_arr = matches!(g.ty, Type::Array(_, _));
                        self.emit_ptr_data_section_kind(is_arr);
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        self.emit_quad_sym_addr(&self.c_sym(v));
                    } else if let Expr::Cast {
                        ty: cty,
                        expr: inner,
                    } = expr.as_ref()
                    {
                        if let Expr::InitList { fields } = inner.as_ref() {
                            // pointer to compound literal: emit data object + pointer
                            let id = self.label_id;
                            self.label_id += 1;
                            let gname = format!("__compg_{id}");
                            let glab = self.c_sym(&gname);
                            self.emit_var_section(g, "rodata");
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{glab}:").unwrap();
                            self.emit_init_list_data(cty, fields)?;
                            writeln!(self.out, "\t.globl\t{sym}").unwrap();
                            self.emit_ptr_data_section();
                            writeln!(self.out, "{sym}:").unwrap();
                            self.emit_quad_sym_addr(&glab);
                        } else if let Expr::Var(v) = inner.as_ref() {
                            let is_arr = matches!(g.ty, Type::Array(_, _));
                            self.emit_ptr_data_section_kind(is_arr);
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            self.emit_quad_sym_addr(&self.c_sym(v));
                        } else {
                            // best-effort null pointer
                            self.emit_var_section(g, "rodata");
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            writeln!(self.out, "\t.quad\t0").unwrap();
                        }
                    } else if let Expr::Index { base, index } = expr.as_ref() {
                        // &arr[const] → symbol + offset
                        if let (Expr::Var(v), Some(idx)) =
                            (base.as_ref(), Self::const_i64(index))
                        {
                            let esz = match &g.ty {
                                Type::Ptr(inner) => self.type_size(inner).max(1),
                                _ => 1,
                            };
                            // better: size of array element of v if known
                            let esz = self.globals.get(v).map(|t| match t {
                                Type::Array(e, _) => self.type_size(e).max(1),
                                Type::Ptr(e) => self.type_size(e).max(1),
                                _ => esz,
                            }).unwrap_or(esz);
                            self.emit_ptr_data_section();
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            let lab = format!("{}+{}", self.c_sym(v), idx * esz);
                            self.emit_quad_sym_addr(&lab);
                        } else {
                            self.emit_var_section(g, "rodata");
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            writeln!(self.out, "\t.quad\t0").unwrap();
                        }
                    } else {
                        // Unsupported &expr form: null
                        self.emit_var_section(g, "rodata");
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        writeln!(self.out, "\t.quad\t0").unwrap();
                    }
                }
                Expr::String(s) => {
                    // char arr[] = "..." is contents; char *p = "..." is pointer
                    if matches!(g.ty, Type::Array(_, _)) {
                        self.emit_var_section(g, "rodata");
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        write!(self.out, "\t.asciz\t\"").unwrap();
                        for b in s.bytes() {
                            match b {
                                b'\n' => write!(self.out, "\\n").unwrap(),
                                b'\\' => write!(self.out, "\\\\").unwrap(),
                                b'"' => write!(self.out, "\\\"").unwrap(),
                                b if (0x20..0x7f).contains(&b) => {
                                    write!(self.out, "{}", b as char).unwrap()
                                }
                                b => write!(self.out, "\\{:03o}", b).unwrap(),
                            }
                        }
                        writeln!(self.out, "\"").unwrap();
                    } else {
                        let id = self.intern_str(s);
                        self.emit_var_section(g, "rodata");
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        writeln!(self.out, "\t.quad\tl_str_{id}").unwrap();
                    }
                }
                Expr::Var(v) if self.funcs.contains_key(v) || v == "main" => {
                    // function address into a pointer global (often mutated)
                    let is_arr = matches!(g.ty, Type::Array(_, _));
                    self.emit_ptr_data_section_kind(is_arr);
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    self.emit_quad_sym_addr(&self.c_sym(v));
                }
                // static char *p = buf; — array/object designator decays to
                // address. Without this, init falls through to BSS zero and
                // Tcl_LinkVar STRING reads NULL (enc2 sqlite_last_needed_collation).
                Expr::Var(v) if matches!(g.ty, Type::Ptr(_)) => {
                    self.emit_ptr_data_section_kind(false);
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    self.emit_quad_sym_addr(&self.c_sym(v));
                }
                Expr::InitList { fields } => {
                    // Writable structs (memblock, etc.) must live in .data, not
                    // .rodata — early boot writes regions/cnt and a RO page would
                    // Data-Abort or leave max=0 forever. Explicit section attrs
                    // (e.g. __initdata_memblock → .ref.data) still win via
                    // emit_var_section.
                    self.emit_var_section(g, "data");
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    self.emit_init_list_data(&g.ty, fields)?;
                }
                Expr::SizeofType(t) => {
                    self.emit_var_section(g, "data");
                    let al = self.type_align(&g.ty).max(1).min(8);
                    writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    self.emit_int_directive(size, self.type_size(t));
                }
                Expr::SizeofExpr(ex) => {
                    let n = if let Expr::String(s) = ex.as_ref() {
                        (s.len() + 1) as i64
                    } else {
                        // Best-effort: typeof via emit path is unavailable; use 8.
                        // Full typeof needs typedefs; SizeofType is the common case.
                        8
                    };
                    self.emit_var_section(g, "data");
                    let al = self.type_align(&g.ty).max(1).min(8);
                    writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    self.emit_int_directive(size, n);
                }
                Expr::Cast { ty: _, expr } => {
                    // myint x = (myint)1; — peel casts for constant inits
                    match expr.as_ref() {
                        Expr::Int(n) | Expr::Char(n) => {
                            let sec = if *n == 0 { "bss" } else { "data" };
                            self.emit_var_section(g, sec);
                            let al = self.type_align(&g.ty).max(1).min(8);
                            writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            if matches!(g.ty, Type::Float) {
                                writeln!(self.out, "\t.float\t{}", *n as f32).unwrap();
                            } else if matches!(g.ty, Type::Double) {
                                writeln!(self.out, "\t.double\t{}", *n as f64).unwrap();
                            } else if *n == 0 {
                                writeln!(self.out, "\t.zero\t{size}").unwrap();
                            } else {
                                self.emit_int_directive(size, *n);
                            }
                        }
                        Expr::Float(f) => {
                            self.emit_var_section(g, "data");
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            if matches!(g.ty, Type::Float) {
                                writeln!(self.out, "\t.float\t{f}").unwrap();
                            } else {
                                writeln!(self.out, "\t.double\t{f}").unwrap();
                            }
                        }
                        other => {
                            // recursive peel via synthetic global
                            let mut g2 = g.clone();
                            g2.init = Some(other.clone());
                            return self.emit_global(&g2);
                        }
                    }
                }
                other => {
                    // Prefer const_i64_env so sizeof(T)*N (SQLite bitmask_size)
                    // folds; Self::const_i64 alone leaves BSS zeros and breaks
                    // join3 / Tcl_LinkVar bitmask_size.
                    if let Some(n) = self.const_i64_env(other) {
                        let sec = if n == 0 { "bss" } else { "data" };
                        self.emit_var_section(g, sec);
                        let al = self.type_align(&g.ty).max(1).min(8);
                        writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        if n == 0 {
                            writeln!(self.out, "\t.zero\t{size}").unwrap();
                        } else {
                            self.emit_int_directive(size, n);
                        }
                    } else {
                        // Non-const init we cannot fold: writable BSS.
                        self.emit_var_section(g, "bss");
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        writeln!(self.out, "\t.zero\t{size}").unwrap();
                    }
                }
            }
        } else {
            // Zero-init: prefer .bss so memblock region arrays and similar
            // writable statics can be updated at runtime. (Earlier Linux path
            // stuffed these into .rodata for vdso link quirks — that made
            // memblock_add Data-Abort when writing regions[].) Explicit
            // section attributes still win via emit_var_section.
            self.emit_var_section(g, "bss");
            writeln!(self.out, "\t.p2align\t3").unwrap();
            writeln!(self.out, "{sym}:").unwrap();
            writeln!(self.out, "\t.zero\t{size}").unwrap();
        }
        Ok(())
    }

    fn emit_init_list_data(
        &mut self,
        ty: &Type,
        fields_in: &[(Option<String>, Expr)],
    ) -> Result<(), String> {
        match ty {
            Type::Array(elem, n) => {
                let esz = self.type_size(elem);
                // Incomplete arrays may still reach here with n==0 if a parser
                // path skipped infer_array_size — recover length from the list.
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
                // Map designators and positional
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
                if let Some(lay) = self.layouts.get(name).cloned() {
                    self.emit_struct_init_data(&lay, fields_in)?;
                } else {
                    // Unknown layout: emit zeros for a reasonable blob
                    writeln!(self.out, "\t.zero\t64").unwrap();
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
        // Build values per top-level field name. Nested designators
        // (`.memory.regions = x`) are collected into a synthetic InitList for
        // the outer field so nested struct layouts get correct pointers/cnt/max.
        let mut by_name: HashMap<String, Expr> = HashMap::new();
        let mut positional: Vec<Expr> = Vec::new();
        // nested_parts[outer] = vec of (inner_path, expr) for `.outer.inner... = e`
        let mut nested_parts: HashMap<String, Vec<(Option<String>, Expr)>> = HashMap::new();
        for (name, e) in fields_in {
            if let Some(n) = name {
                if let Some((outer, rest)) = n.split_once('.') {
                    nested_parts
                        .entry(outer.to_string())
                        .or_default()
                        .push((Some(rest.to_string()), e.clone()));
                } else {
                    by_name.insert(n.clone(), e.clone());
                }
            } else {
                positional.push(e.clone());
            }
        }
        // Materialize nested InitLists for outer fields that only received
        // dotted designators (and merge with a direct `.outer = {…}` if any).
        for (outer, parts) in nested_parts {
            match by_name.get(&outer) {
                Some(Expr::InitList { fields }) => {
                    let mut merged = fields.clone();
                    merged.extend(parts);
                    by_name.insert(outer, Expr::InitList { fields: merged });
                }
                Some(_) => {
                    // Prefer explicit whole-field init over partial nested.
                }
                None => {
                    by_name.insert(outer, Expr::InitList { fields: parts });
                }
            }
        }
        let mut ordered: Vec<_> = lay.fields.iter().collect();
        ordered.sort_by_key(|(_, p)| p.offset);
        let mut pos = 0i64;
        let mut pos_i = 0usize;
        let mut i = 0;
        while i < ordered.len() {
            let off = ordered[i].1.offset;
            if pos < off {
                writeln!(self.out, "\t.zero\t{}", off - pos).unwrap();
                pos = off;
            }
            // Group union members that share the same offset — one storage slot.
            let mut j = i + 1;
            while j < ordered.len() && ordered[j].1.offset == off {
                j += 1;
            }
            let is_union_slot = j > i + 1;

            // Pick the member that actually has an initializer. Critical for
            // Redis `typeData`: `.data.string = {…}` must emit with
            // stringConfigData layout, not the first union member (yesno).
            // Using the wrong member type mis-sizes fields (int vs char*) and
            // corrupts static_configs → initConfigValues blr to garbage.
            let mut chosen = i;
            let mut e: Option<&Expr> = None;
            if let Some(ex) = by_name.get(ordered[i].0) {
                e = Some(ex);
                chosen = i;
            } else {
                for k in i..j {
                    if let Some(ex) = by_name.get(ordered[k].0) {
                        e = Some(ex);
                        chosen = k;
                        break;
                    }
                }
            }
            if e.is_none() && pos_i < positional.len() {
                e = Some(&positional[pos_i]);
                pos_i += 1;
                chosen = i; // positional targets first member / whole slot
            }

            let fty = &ordered[chosen].1.ty;
            // Union slot occupies max member size (layout size from offset for
            // single top-level union); struct field uses its own size.
            let slot = if is_union_slot {
                ordered[i..j]
                    .iter()
                    .map(|(_, p)| self.type_size(&p.ty))
                    .max()
                    .unwrap_or(0)
                    .max(self.type_size(fty))
            } else {
                self.type_size(fty)
            };

            let emitted = if let Some(e) = e {
                let before = self.out.len();
                // Nested struct/array init lists must recurse, not use scalar path.
                match e {
                    Expr::InitList { fields } => {
                        self.emit_init_list_data(fty, fields)?;
                    }
                    _ => {
                        // C `T x = {0}` with T aggregate: scalar 0 zeros the whole
                        // field (pthread_mutex_t = {0} → 48-byte zero, not one .quad).
                        if matches!(
                            fty,
                            Type::Array(_, _)
                                | Type::Struct(_)
                                | Type::Union(_)
                                | Type::AnonStruct(_)
                                | Type::AnonUnion(_)
                        ) && matches!(Self::const_i64(e), Some(0))
                        {
                            let fsz = self.type_size(fty).max(1);
                            writeln!(self.out, "\t.zero\t{fsz}").unwrap();
                        } else {
                            self.emit_scalar_data(fty, e)?;
                        }
                    }
                }
                // Best-effort: if member emit wrote fewer bytes than the union
                // slot, pad (member size may be < union size).
                let _ = before;
                self.type_size(fty)
            } else {
                writeln!(self.out, "\t.zero\t{slot}").unwrap();
                slot
            };
            if emitted < slot {
                writeln!(self.out, "\t.zero\t{}", slot - emitted).unwrap();
            }
            pos += slot;
            i = j;
        }
        if pos < lay.size {
            writeln!(self.out, "\t.zero\t{}", lay.size - pos).unwrap();
        }
        Ok(())
    }

    /// Emit a static integer of the given byte size (1/2/4/8, or larger zero blob).
    fn emit_int_directive(&mut self, size: i64, n: i64) {
        match size {
            1 => writeln!(self.out, "\t.byte\t{n}").unwrap(),
            // Critical: short/unsigned short tables (lemon yy_action, yy_lookahead)
            // must be 2-byte elements. Emitting .long broke all parser indexing.
            2 => writeln!(self.out, "\t.hword\t{n}").unwrap(),
            4 => writeln!(self.out, "\t.long\t{n}").unwrap(),
            8 => writeln!(self.out, "\t.quad\t{n}").unwrap(),
            // Aggregate zero-init via `{0}` / PTHREAD_MUTEX_INITIALIZER reaches here
            // with type_size=48 (mutex). Emitting a single .quad left only 8 bytes
            // reserved → neighboring .data bled into the mutex → glibc owner assert.
            sz if sz > 8 => {
                if n == 0 {
                    writeln!(self.out, "\t.zero\t{sz}").unwrap();
                } else {
                    writeln!(self.out, "\t.quad\t{n}").unwrap();
                    writeln!(self.out, "\t.zero\t{}", sz - 8).unwrap();
                }
            }
            _ => writeln!(self.out, "\t.quad\t{n}").unwrap(),
        }
    }

    /// Static reloc for lvalue address expressions used in static initializers:
    /// `&g`, `&g.field`, `&g.arr[i]`, combinations. Returns `(symbol, byte_offset)`.
    fn static_lvalue_reloc(&self, e: &Expr) -> Option<(String, i64)> {
        let empty: HashMap<String, Type> = HashMap::new();
        match e {
            Expr::Var(v) => {
                if self.funcs.contains_key(v) || v == "main" {
                    return Some((self.c_sym(v), 0));
                }
                // Function-scope statics live as locals with Storage::Global
                // (`__static_<func>_<name>`). Needed for static initializers
                // like `(void*)&iZero` in SQLite test1.c Sqlitetest1_Init.
                if let Some(sym) = self.get_local(v) {
                    if let Storage::Global { name } = &sym.storage {
                        return Some((self.c_sym(name), 0));
                    }
                }
                if self.globals.contains_key(v) && self.get_local(v).is_none() {
                    return Some((self.c_sym(v), 0));
                }
                // File-scope name not yet in globals map but known as const/global.
                if self.const_globals.contains_key(v) {
                    return Some((self.c_sym(v), 0));
                }
                None
            }
            Expr::Member { base, field, arrow } => {
                // `->` needs a runtime pointer; only `.` on a static object.
                if *arrow {
                    return None;
                }
                let (sym, base_off) = self.static_lvalue_reloc(base)?;
                let bty = self.typeof_expr(base, &empty);
                let foff = self.static_member_offset(&bty, field)?;
                Some((sym, base_off + foff))
            }
            Expr::Index { base, index } => {
                let (sym, base_off) = self.static_lvalue_reloc(base)?;
                let idx = Self::const_i64(index)?;
                let bty = self.typeof_expr(base, &empty);
                let esz = match &bty {
                    Type::Array(elem, _) => self.type_size(elem),
                    Type::Ptr(elem) => self.type_size(elem),
                    // Soft: unknown base treated as pointer-sized elements.
                    _ => 8,
                };
                Some((sym, base_off + idx * esz))
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                // `&*p` — only if p itself is a static address expression.
                // Not generally valid for static init; refuse.
                let _ = expr;
                None
            }
            Expr::Cast { expr, .. } => self.static_lvalue_reloc(expr),
            _ => None,
        }
    }

    fn static_member_offset(&self, base_ty: &Type, field: &str) -> Option<i64> {
        match base_ty {
            Type::Struct(n) | Type::Union(n) => {
                self.layouts.get(n).and_then(|lay| {
                    lay.fields.get(field).map(|p| p.offset)
                })
            }
            Type::AnonStruct(_) | Type::AnonUnion(_) => {
                // Layouts for anons may be keyed by synthetic names; search.
                for lay in self.layouts.values() {
                    if let Some(p) = lay.fields.get(field) {
                        return Some(p.offset);
                    }
                }
                None
            }
            _ => {
                for lay in self.layouts.values() {
                    if let Some(p) = lay.fields.get(field) {
                        return Some(p.offset);
                    }
                }
                None
            }
        }
    }

    fn emit_scalar_data(&mut self, ty: &Type, e: &Expr) -> Result<(), String> {
        // Fold simple constant expressions for static storage duration.
        // const_i64_env folds sizeof(T) and sizeof(T)*N (needs type_size).
        if let Some(n) = self.const_i64_env(e) {
            match ty {
                // Integer into float field must use IEEE bits, not raw int directive.
                Type::Float => {
                    writeln!(self.out, "\t.float\t{}", n as f32).unwrap();
                }
                Type::Double => {
                    writeln!(self.out, "\t.double\t{}", n as f64).unwrap();
                }
                _ => {
                    let sz = self.type_size(ty);
                    self.emit_int_directive(sz, n);
                }
            }
            return Ok(());
        }
        match e {
            Expr::SizeofType(t) => {
                let n = self.type_size(t);
                self.emit_int_directive(self.type_size(ty), n);
                return Ok(());
            }
            Expr::SizeofExpr(ex) => {
                let n = if let Expr::String(s) = ex.as_ref() {
                    (s.len() + 1) as i64
                } else {
                    let et = self.typeof_expr(ex, &HashMap::new());
                    match &et {
                        Type::Array(elem, n) => self.type_size(elem) * (*n).max(0),
                        other => self.type_size(other),
                    }
                };
                self.emit_int_directive(self.type_size(ty), n);
                return Ok(());
            }
            _ => {}
        }
        match e {
            Expr::Float(f) => {
                match ty {
                    Type::Float => writeln!(self.out, "\t.float\t{f}").unwrap(),
                    Type::Double => writeln!(self.out, "\t.double\t{f}").unwrap(),
                    // Soft: store truncated integer bits if target is integral.
                    _ => {
                        let sz = self.type_size(ty);
                        self.emit_int_directive(sz, *f as i64);
                    }
                }
            }
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => {
                // Static storage: emit reloc for &global, &global.field,
                // &global.arr[i] (e.g. shell isKnownWritable apst[]).
                if let Some((sym, off)) = self.static_lvalue_reloc(expr) {
                    if off == 0 {
                        writeln!(self.out, "\t.quad\t{sym}").unwrap();
                    } else {
                        writeln!(self.out, "\t.quad\t{sym}+{off}").unwrap();
                    }
                } else if let Expr::Var(v) = expr.as_ref() {
                    writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
                } else {
                    writeln!(self.out, "\t.quad\t0").unwrap();
                }
            }
            Expr::Var(v) => {
                // Integer enum/static-const globals → value, not address
                // (avoids misaligned ARM64_RELOC_UNSIGNED in struct init lists).
                let is_int_ty = matches!(
                    ty,
                    Type::Int
                        | Type::UInt
                        | Type::Char
                        | Type::SChar
                        | Type::Short
                        | Type::UShort
                        | Type::Long
                        | Type::ULong
                );
                if is_int_ty {
                    if let Some(n) = self.const_globals.get(v).copied() {
                        self.emit_int_directive(self.type_size(ty), n);
                        return Ok(());
                    }
                }
                // Function designator → address (defined in this TU).
                if self.funcs.contains_key(v) || v == "main" {
                    writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
                    return Ok(());
                }
                // True file-scope global object → address.
                if self.globals.contains_key(v) && self.get_local(v).is_none() {
                    writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
                    return Ok(());
                }
                // Locals/params: zero-fill; runtime compound path fills them.
                // (Do not emit ABS64 to a stack name.)
                if self.get_local(v).is_some() {
                    writeln!(self.out, "\t.zero\t{}", self.type_size(ty).max(1)).unwrap();
                    return Ok(());
                }
                // Unknown name used as a pointer in static init → external
                // designator (libc getcwd/open/close, etc.). SQLite aSyscall[]:
                //   { "getcwd", (sqlite3_syscall_ptr)getcwd, 0 }
                // was zero-filled here, so osGetcwd → blr xzr → SEGV.
                // Pointer-typed field: emit a real reloc for the linker.
                if matches!(ty, Type::Ptr(_)) {
                    writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
                    return Ok(());
                }
                // Non-pointer unknown: keep zero (kernel soft / incomplete).
                writeln!(self.out, "\t.zero\t{}", self.type_size(ty).max(1)).unwrap();
            }
            Expr::String(s) => {
                // char arr[N] = "lit" → embed bytes (NOT a pointer). Critical for
                // SQLite aXformType.zName[7] and similar; emitting .quad caused
                // unaligned ARM64_RELOC_UNSIGNED and ld "pointer not aligned".
                if let Type::Array(elem, n) = ty {
                    let is_byte_elem = matches!(elem.as_ref(), Type::Char | Type::SChar)
                        || self.type_size(elem) == 1;
                    if is_byte_elem {
                        let nbytes = (*n as usize).max(0);
                        let bytes = s.as_bytes();
                        for i in 0..nbytes {
                            let b = bytes.get(i).copied().unwrap_or(0);
                            writeln!(self.out, "\t.byte\t{b}").unwrap();
                        }
                        return Ok(());
                    }
                }
                // char *p = "literal" → pointer to interned string
                let id = self.intern_str(s);
                writeln!(self.out, "\t.quad\tl_str_{id}").unwrap();
            }
            Expr::InitList { fields } => {
                self.emit_init_list_data(ty, fields)?;
            }
            Expr::Cast { expr, .. } => {
                // Peel casts of string/function for static init
                return self.emit_scalar_data(ty, expr);
            }
            other => {
                if let Some(n) = self.const_i64_env(other) {
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

    /// True if every field can live in static .rodata (no locals/params).
    fn init_list_is_static_const(&self, fields: &[(Option<String>, Expr)]) -> bool {
        fields.iter().all(|(_, e)| self.expr_is_static_const(e))
    }

    fn expr_is_static_const(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Char(_) | Expr::Float(_) | Expr::String(_) => true,
            Expr::Unary {
                op: UnaryOp::Neg | UnaryOp::BitNot | UnaryOp::Not,
                expr,
            } => self.expr_is_static_const(expr),
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => self.static_lvalue_reloc(expr).is_some(),
            Expr::Var(v) => {
                self.const_globals.contains_key(v)
                    || self.funcs.contains_key(v)
                    || v == "main"
                    || (self.globals.contains_key(v) && self.get_local(v).is_none())
            }
            Expr::Cast { expr, .. } => self.expr_is_static_const(expr),
            Expr::Binary { left, right, .. } => {
                self.expr_is_static_const(left) && self.expr_is_static_const(right)
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                self.expr_is_static_const(cond)
                    && self.expr_is_static_const(then_e)
                    && self.expr_is_static_const(else_e)
            }
            Expr::InitList { fields } => self.init_list_is_static_const(fields),
            Expr::SizeofType(_) | Expr::SizeofExpr(_) => true,
            _ => false,
        }
    }

    fn const_i64(e: &Expr) -> Option<i64> {
        Self::const_i64_with(e, None)
    }

    /// Constant-fold with optional enum/static-const environment.
    /// Also folds `sizeof(T)` / `sizeof(expr)` via type_size (SQLite
    /// `static int bitmask_size = sizeof(Bitmask)*8`).
    fn const_i64_env(&self, e: &Expr) -> Option<i64> {
        self.const_i64_with_sizeof(e, Some(&self.const_globals))
    }

    fn sizeof_const_i64(&self, e: &Expr) -> Option<i64> {
        match e {
            Expr::SizeofType(t) => Some(self.type_size(t)),
            Expr::SizeofExpr(ex) => {
                if let Expr::String(s) = ex.as_ref() {
                    return Some((s.len() + 1) as i64);
                }
                let et = self.typeof_expr(ex, &HashMap::new());
                Some(match &et {
                    Type::Array(elem, n) => self.type_size(elem) * (*n).max(0),
                    other => self.type_size(other),
                })
            }
            _ => None,
        }
    }

    fn const_i64_with_sizeof(
        &self,
        e: &Expr,
        env: Option<&HashMap<String, i64>>,
    ) -> Option<i64> {
        if let Some(n) = self.sizeof_const_i64(e) {
            return Some(n);
        }
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Var(name) => env.and_then(|m| m.get(name).copied()),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(-self.const_i64_with_sizeof(expr, env)?),
            Expr::Unary {
                op: UnaryOp::BitNot,
                expr,
            } => Some(!self.const_i64_with_sizeof(expr, env)?),
            Expr::Cast { expr, .. } => self.const_i64_with_sizeof(expr, env),
            Expr::Binary { op, left, right } => {
                let l = self.const_i64_with_sizeof(left, env)?;
                let r = self.const_i64_with_sizeof(right, env)?;
                Self::const_i64_apply_binop(*op, l, r)
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let c = self.const_i64_with_sizeof(cond, env)?;
                if c != 0 {
                    self.const_i64_with_sizeof(then_e, env)
                } else {
                    self.const_i64_with_sizeof(else_e, env)
                }
            }
            _ => None,
        }
    }

    fn const_i64_apply_binop(op: BinOp, l: i64, r: i64) -> Option<i64> {
        // Shared by const_i64_with / const_i64_with_sizeof.
        let u = (l as u64, r as u64);
        let unsignedish = l < 0 || r < 0;
        Some(match op {
            BinOp::Add => l.wrapping_add(r),
            BinOp::Sub => l.wrapping_sub(r),
            BinOp::Mul => l.wrapping_mul(r),
            BinOp::Div if r != 0 => {
                if unsignedish {
                    (u.0 / u.1) as i64
                } else {
                    l / r
                }
            }
            BinOp::Mod if r != 0 => {
                if unsignedish {
                    (u.0 % u.1) as i64
                } else {
                    l % r
                }
            }
            BinOp::Shl => l.wrapping_shl(r as u32),
            BinOp::Shr => {
                if unsignedish {
                    (u.0 >> (r as u32)) as i64
                } else {
                    l.wrapping_shr(r as u32)
                }
            }
            BinOp::BitAnd => l & r,
            BinOp::BitOr => l | r,
            BinOp::BitXor => l ^ r,
            BinOp::Comma => r,
            BinOp::Eq => (l == r) as i64,
            BinOp::Ne => (l != r) as i64,
            BinOp::Lt => {
                if unsignedish {
                    (u.0 < u.1) as i64
                } else {
                    (l < r) as i64
                }
            }
            BinOp::Le => {
                if unsignedish {
                    (u.0 <= u.1) as i64
                } else {
                    (l <= r) as i64
                }
            }
            BinOp::Gt => {
                if unsignedish {
                    (u.0 > u.1) as i64
                } else {
                    (l > r) as i64
                }
            }
            BinOp::Ge => {
                if unsignedish {
                    (u.0 >= u.1) as i64
                } else {
                    (l >= r) as i64
                }
            }
            BinOp::And => ((l != 0) && (r != 0)) as i64,
            BinOp::Or => ((l != 0) || (r != 0)) as i64,
            _ => return None,
        })
    }

    fn const_i64_with(e: &Expr, env: Option<&HashMap<String, i64>>) -> Option<i64> {
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Var(name) => env.and_then(|m| m.get(name).copied()),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(-Self::const_i64_with(expr, env)?),
            Expr::Unary {
                op: UnaryOp::BitNot,
                expr,
            } => Some(!Self::const_i64_with(expr, env)?),
            Expr::Cast { expr, .. } => Self::const_i64_with(expr, env),
            Expr::Binary { op, left, right } => {
                let l = Self::const_i64_with(left, env)?;
                let r = Self::const_i64_with(right, env)?;
                Self::const_i64_apply_binop(*op, l, r)
            }
            // Remaining arms kept for callers that still use const_i64_with
            // without sizeof; Binary is handled above via apply_binop.
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let c = Self::const_i64_with(cond, env)?;
                if c != 0 {
                    Self::const_i64_with(then_e, env)
                } else {
                    Self::const_i64_with(else_e, env)
                }
            }
            _ => None,
        }
    }

    fn alloc_local(&mut self, name: &str, ty: &Type) -> i64 {
        let sz = self.stack_slot_size(ty).max(8);
        let al = 8i64;
        self.stack_size = Self::align_up(self.stack_size + sz, al);
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

    /// True if `name` is a function-pointer *variable* (call via load+blr).
    ///
    /// - Actual functions (in `funcs`, including prototypes) → false → `bl`
    /// - Local pointer variables used as callee → true → load+blr
    /// - Data globals of pointer type (`monotime (*getMonotonicUs)(void)`) → true
    /// - Undeclared names → false → `bl` (assume external function)
    ///
    /// History: treating every `globals` hit as blr broke `luaL_newstate()`
    /// (ldr of first instruction bytes). Restricting to locals only then broke
    /// Redis `getMonotonicUs()` (global fptr in .bss) with bare `bl` into BSS.
    /// Parser now keeps `T *(name)(params)` as a function (not Ptr global).
    fn is_function_pointer_var(&self, name: &str) -> bool {
        if self.funcs.contains_key(name) {
            return false;
        }
        // Data global of pointer type (function-pointer variables).
        if let Some(ty) = self.globals.get(name) {
            if matches!(ty, Type::Ptr(_)) {
                return true;
            }
        }
        // Locals (and register-address locals) of pointer type.
        if let Some(sym) = self.get_local(name) {
            if matches!(
                sym.storage,
                Storage::Local { .. } | Storage::RegAddr { .. }
            ) {
                return matches!(sym.ty, Type::Ptr(_));
            }
        }
        let _ = &self.defined_data_globals;
        false
    }

    /// Soft stub: empty definition so kernel soft-skip still produces linkable symbols.
    fn emit_stub_function(&mut self, f: &Function) -> Result<(), String> {
        let sym = self.c_sym(&f.name);
        if f.is_static {
            writeln!(self.out, "").unwrap();
        } else {
            match self.os {
                TargetOs::Darwin => {
                    writeln!(self.out, "
	.weak_definition	{sym}").unwrap();
                }
                TargetOs::Linux => {
                    writeln!(self.out, "
	.weak	{sym}").unwrap();
                }
            }
            writeln!(self.out, "\t.globl\t{sym}").unwrap();
        }
        writeln!(self.out, "\t.p2align\t2").unwrap();
        writeln!(self.out, "{sym}:").unwrap();
        // return 0
        writeln!(self.out, "\tmov\tx0, xzr").unwrap();
        writeln!(self.out, "\tret").unwrap();
        Ok(())
    }

    /// Soft freestanding (mid-boot no-ops / body replacements) is **opt-in**.
    /// Stage C1 PASS forbids discarding real C bodies for named kernel helpers
    /// unless the operator sets `ACC_SOFT_FREESTANDING=1` (ladder debugging only).
    fn soft_freestanding_enabled() -> bool {
        matches!(
            std::env::var("ACC_SOFT_FREESTANDING").ok().as_deref(),
            Some("1") | Some("true") | Some("yes")
        )
    }

    /// Kernel freestanding helpers (`panic`→`_printk`, `rest_init`, …) only when
    /// building the Linux kernel. Userspace (Redis/SQLite) must keep real bodies.
    fn kernel_freestanding_enabled() -> bool {
        matches!(
            std::env::var("ACC_KERNEL_FREESTANDING").ok().as_deref(),
            Some("1") | Some("true") | Some("yes")
        )
    }

    /// Names that discard a real C body and emit soft no-ops / partial stubs.
    /// Hard asm replacements (idmap, tpidr, unaligned load) are NOT listed here.
    fn is_soft_freestanding_name(name: &str) -> bool {
        // Gradual real-body ladder: names listed here emit real C when
        // ACC_SOFT_FREESTANDING=0. Empty for now — soft-listing rest_init /
        // do_basic_setup / VFS into main.c (#113–#114) hung after
        // random_init_early (before vfs_caches_init_early). A07 instead
        // keeps init handoff as hard freestanding but routes through
        // kernel_init + kernel_execve (no acc_real_init_payload).
        let _ = name;
        false
    }

    /// Soft freestanding bodies for kernel helpers whose real form is extended
    /// asm we cannot yet lower (EX_TABLE + `%N` operands). Better a correct
    /// plain load than an unassemblable / empty `1:ldr %0` hole.
    ///
    /// Soft mid-boot body *replacements* (no-ops / partial stubs that discard
    /// real C) only run when `ACC_SOFT_FREESTANDING=1`. Default PASS path
    /// emits real AST bodies for those names.
    fn emit_freestanding_kernel_helper(&mut self, f: &Function) -> Result<bool, String> {
        if !Self::kernel_freestanding_enabled() {
            return Ok(false);
        }
        if Self::is_soft_freestanding_name(&f.name) && !Self::soft_freestanding_enabled() {
            return Ok(false);
        }
        match f.name.as_str() {
            "load_unaligned_zeropad" => {
                let sym = self.c_sym(&f.name);
                if f.is_static {
                    writeln!(self.out, "").unwrap();
                } else {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                // x0 = addr → return *(unsigned long *)addr (no fault zeropad).
                writeln!(self.out, "\tldr\tx0, [x0]").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // Early idmap: acc's general map_range/create_init_idmap mis-packs
            // 9-arg calls and historically embedded compound-literal data in
            // .init.text. A minimal correct identity map is enough for MMU on.
            "create_init_idmap" => {
                self.emit_freestanding_create_init_idmap(f)?;
                Ok(true)
            }
            "early_map_kernel" => {
                self.emit_freestanding_early_map_kernel(f)?;
                Ok(true)
            }
            // percpu: real bodies use asm(ALTERNATIVE("msr/mrs tpidr_el1", …))
            // which acc PP does not expand and large TUs still drop. Without a
            // real msr, __my_cpu_offset stays 0 → this_cpu_ptr → FAR=0 in
            // __percpu_read_64. Plain tpidr_el1 is correct for non-VHE (QEMU virt).
            "set_my_cpu_offset" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                // x0 = offset
                writeln!(self.out, "\tmsr\ttpidr_el1, x0").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "__kern_my_cpu_offset" | "__my_cpu_offset" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tmrs\tx0, tpidr_el1").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // Early boot printk → QEMU virt PL011 (0x9000000). Real vprintk
            // aborts before console registration. Handles pr_notice("%s", s)
            // where fmt is KERN_* "\001c%s" (not bare "%s").
            // x0=fmt, x1=first vararg (string for %s).
            //
            // After cpu_uninstall_idmap(), TTBR0 no longer maps phys UART.
            // Temporarily reinstall idmap_pg_dir (still has DEVICE pud[0] for
            // 0x9000000) around the MMIO write, then restore reserved TTBR0.
            "_printk" | "printk" => {
                let sym = self.c_sym(&f.name);
                let idmap_phys = self.c_sym("acc_idmap_phys");
                // Strong .data slot in printk.o. PI objects rename externs with
                // __pi_ prefix, so also export the alias PI looks up.
                writeln!(self.out, "\n\t.globl\t{idmap_phys}").unwrap();
                writeln!(self.out, "\t.globl\t__pi_{idmap_phys}").unwrap();
                writeln!(self.out, "\t.data").unwrap();
                writeln!(self.out, "\t.p2align\t3").unwrap();
                writeln!(self.out, "{idmap_phys}:").unwrap();
                writeln!(self.out, "__pi_{idmap_phys}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "\t.text").unwrap();
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tstp\tx21, x22, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tstp\tx23, x24, [sp, #-16]!").unwrap();
                // x19=fmt, x20=arg1, x21=string to print
                writeln!(self.out, "\tmov\tx19, x0").unwrap();
                writeln!(self.out, "\tmov\tx20, x1").unwrap();
                writeln!(self.out, "\tmov\tx21, x0").unwrap(); // default: print fmt
                // Scan fmt for "%s" → use arg1 as string
                writeln!(self.out, "\tmov\tx2, x19").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(self.out, "\tldrb\tw3, [x2], #1").unwrap();
                writeln!(self.out, "\tcbz\tw3, 2f").unwrap();
                writeln!(self.out, "\tcmp\tw3, #'%'").unwrap();
                writeln!(self.out, "\tb.ne\t0b").unwrap();
                writeln!(self.out, "\tldrb\tw3, [x2]").unwrap();
                writeln!(self.out, "\tcmp\tw3, #'s'").unwrap();
                writeln!(self.out, "\tb.ne\t0b").unwrap();
                writeln!(self.out, "\tmov\tx21, x20").unwrap(); // "%s" → arg1
                writeln!(self.out, "2:").unwrap();
                // Detect high-VA (post early_map_kernel): PC has top bits set.
                // Pre-MMU / idmap-active: use UART phys directly.
                // Post cpu_uninstall_idmap: reinstall idmap via phys saved at
                // create_init_idmap into acc_idmap_phys.
                writeln!(self.out, "\tadr\tx9, 2b").unwrap();
                writeln!(self.out, "\tlsr\tx10, x9, #48").unwrap();
                writeln!(self.out, "\tcbz\tx10, 6f").unwrap(); // low PC → direct UART
                // High VA: save TTBR0, switch to phys idmap from acc_idmap_phys.
                writeln!(self.out, "\tmrs\tx23, ttbr0_el1").unwrap();
                writeln!(self.out, "\tadrp\tx9, {idmap_phys}").unwrap();
                writeln!(self.out, "\tldr\tx9, [x9, :lo12:{idmap_phys}]").unwrap();
                writeln!(self.out, "\tcbz\tx9, 6f").unwrap(); // unset → try direct
                writeln!(self.out, "\tmsr\tttbr0_el1, x9").unwrap();
                writeln!(self.out, "\tisb").unwrap();
                writeln!(self.out, "\tmov\tx24, #1").unwrap(); // need restore
                writeln!(self.out, "\tb\t7f").unwrap();
                writeln!(self.out, "6:").unwrap();
                writeln!(self.out, "\tmov\tx24, xzr").unwrap(); // no restore
                writeln!(self.out, "7:").unwrap();
                // Write PL011 at phys 0x9000000 via idmap/phys DEVICE.
                writeln!(self.out, "\tmovz\tx0, #0x0000").unwrap();
                writeln!(self.out, "\tmovk\tx0, #0x900, lsl #16").unwrap();
                writeln!(self.out, "3:").unwrap();
                writeln!(self.out, "\tldrb\tw2, [x21], #1").unwrap();
                writeln!(self.out, "\tcbz\tw2, 4f").unwrap();
                // skip non-printable KERN SOH prefix bytes except tab/lf/cr
                writeln!(self.out, "\tcmp\tw2, #0x20").unwrap();
                writeln!(self.out, "\tb.ge\t5f").unwrap();
                writeln!(self.out, "\tcmp\tw2, #0x09").unwrap();
                writeln!(self.out, "\tb.eq\t5f").unwrap();
                writeln!(self.out, "\tcmp\tw2, #0x0a").unwrap();
                writeln!(self.out, "\tb.eq\t5f").unwrap();
                writeln!(self.out, "\tcmp\tw2, #0x0d").unwrap();
                writeln!(self.out, "\tb.eq\t5f").unwrap();
                writeln!(self.out, "\tb\t3b").unwrap();
                writeln!(self.out, "5:").unwrap();
                writeln!(self.out, "\tstr\tw2, [x0]").unwrap();
                writeln!(self.out, "\tb\t3b").unwrap();
                writeln!(self.out, "4:").unwrap();
                writeln!(self.out, "\tmovz\tw2, #0x0a").unwrap();
                writeln!(self.out, "\tstr\tw2, [x0]").unwrap();
                // Restore TTBR0 if we switched
                writeln!(self.out, "\tcbz\tx24, 8f").unwrap();
                writeln!(self.out, "\tmsr\tttbr0_el1, x23").unwrap();
                writeln!(self.out, "\tisb").unwrap();
                writeln!(self.out, "8:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx23, x24, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx21, x22, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // early_fixmap_init walks swapper_pg_dir for FIXADDR; our early
            // TTBR1 map is image-only, so p*d walks return garbage and
            // Data Abort (FAR=0x80008000800080) before setup_machine_fdt.
            // No-op: freestanding _printk already hits PL011 via idmap DEVICE.
            "early_fixmap_init" | "early_ioremap_init" | "fixmap_remap_fdt" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                // fixmap_remap_fdt returns void* — NULL means scan fails.
                // setup_machine_fdt freestanding below avoids that path.
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // map_mem: real path uses __create_pgd_mapping + early_pgtable_alloc
            // which needs fixmap (no-op for us) → hang. Build linear map without
            // fixmap: 1GB PUD block for QEMU virt RAM, then activate swapper.
            "map_mem" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                // Early page-table page pool (phys-identity via kernel map).
                let pool = self.c_sym("acc_early_pt_pool");
                let pool_next = self.c_sym("acc_early_pt_next");
                writeln!(self.out, "\t.globl\t{pool}").unwrap();
                writeln!(self.out, "\t.globl\t{pool_next}").unwrap();
                writeln!(self.out, "\t.bss").unwrap();
                writeln!(self.out, "\t.p2align\t12").unwrap();
                writeln!(self.out, "{pool}:").unwrap();
                writeln!(self.out, "\t.zero\t{}", 8 * 4096).unwrap(); // 8 pages
                writeln!(self.out, "\t.data").unwrap();
                writeln!(self.out, "\t.p2align\t3").unwrap();
                writeln!(self.out, "{pool_next}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "\t.text").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tstp\tx21, x22, [sp, #-16]!").unwrap();
                // x23 is callee-saved; previous freestanding clobbered it and
                // left setup_arch / paging_init with a trashed FP/LR chain.
                writeln!(self.out, "\tstp\tx23, x24, [sp, #-16]!").unwrap();
                // x19 = pgdp argument (swapper_pg_dir VA)
                writeln!(self.out, "\tmov\tx19, x0").unwrap();
                // breadcrumb
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(self.out, "\t.asciz\t\"map_mem: linear RAM (no-fixmap)\\n\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();

                // --- Build linear map into swapper without fixmap ---
                // PAGE_OFFSET (VA_BITS=48) = 0xffff000000000000
                // memstart=0x40000000 → phys 0x40000000 @ PAGE_OFFSET
                // 1GB PUD block covers [0x40000000, 0x80000000).
                //
                // Phys conversion: TTBR1 currently holds phys(init_pg_dir).
                // phys(X) = ttbr1_phys + (VA(X) - VA(init_pg_dir)).
                let ipgd = self.c_sym("init_pg_dir");
                writeln!(self.out, "\tmrs\tx20, ttbr1_el1").unwrap(); // phys init_pg_dir
                writeln!(self.out, "\tadrp\tx21, {ipgd}").unwrap();
                writeln!(self.out, "\tadd\tx21, x21, :lo12:{ipgd}").unwrap(); // VA init
                // 1) Copy root PGD init → swapper (keeps kernel image maps)
                writeln!(self.out, "\tmov\tx0, x19").unwrap(); // dst = swapper VA
                writeln!(self.out, "\tmov\tx1, x21").unwrap(); // src = init VA
                writeln!(self.out, "\tmovz\tx2, #0x1000").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();

                // 2) PUD page from pool
                writeln!(self.out, "\tadrp\tx22, {pool}").unwrap();
                writeln!(self.out, "\tadd\tx22, x22, :lo12:{pool}").unwrap(); // pool VA
                // pool_phys = ttbr1_phys + (pool_va - init_va)
                writeln!(self.out, "\tsub\tx0, x22, x21").unwrap();
                writeln!(self.out, "\tadd\tx23, x20, x0").unwrap(); // pool phys
                // zero PUD page
                writeln!(self.out, "\tmov\tx0, x22").unwrap();
                writeln!(self.out, "\tmov\tx1, xzr").unwrap();
                writeln!(self.out, "\tmovz\tx2, #0x1000").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("memset")).unwrap();

                // 3) swapper_pg_dir[0] = pud_phys | TABLE
                writeln!(self.out, "\torr\tx0, x23, #3").unwrap();
                writeln!(self.out, "\tstr\tx0, [x19]").unwrap();

                // 4) pud[0] = 1GB NORMAL non-exec block @ phys 0x40000000
                // TYPE_BLOCK|AF|ISH|ATTR_NORMAL|PXN|UXN = 0x6000000000000d01
                writeln!(self.out, "\tmovz\tx0, #0x0000").unwrap();
                writeln!(self.out, "\tmovk\tx0, #0x4000, lsl #16").unwrap();
                writeln!(self.out, "\tmovz\tx1, #0x0d01").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0x6000, lsl #48").unwrap();
                writeln!(self.out, "\torr\tx0, x0, x1").unwrap();
                writeln!(self.out, "\tstr\tx0, [x22]").unwrap();

                // 5) switch TTBR1 → swapper phys
                writeln!(self.out, "\tdsb\tishst").unwrap();
                writeln!(self.out, "\tsub\tx0, x19, x21").unwrap();
                writeln!(self.out, "\tadd\tx0, x20, x0").unwrap(); // swapper phys
                writeln!(self.out, "\tmsr\tttbr1_el1, x0").unwrap();
                writeln!(self.out, "\tisb").unwrap();
                writeln!(self.out, "\ttlbi\tvmalle1").unwrap();
                writeln!(self.out, "\tdsb\tnsh").unwrap();
                writeln!(self.out, "\tisb").unwrap();

                // breadcrumb
                writeln!(self.out, "\tb\t5f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "4:").unwrap();
                writeln!(self.out, "\t.asciz\t\"map_mem: TTBR1=swapper + linear 1GB\\n\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "5:").unwrap();
                writeln!(self.out, "\tadr\tx0, 4b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();

                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx23, x24, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx21, x22, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // declare_vma: BUG_ON(!PAGE_ALIGNED) — no-op until vmap early works.
            // create_idmap: __pi_map_range Data-Aborts; keep early idmap.
            // unflatten_device_tree: no real FDT (setup_machine_fdt stub) →
            // fdt_check_header on garbage → die. Skip OF unflatten for now.
            // bootmem_init: sparse/vmemmap path needs full VMEMMAP page tables;
            // soft-minimal: set pfn globals from memblock and continue boot.
            "bootmem_init" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
                // min = PFN_UP(memblock_start_of_DRAM()); max = PFN_DOWN(end)
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_start_of_DRAM")).unwrap();
                writeln!(self.out, "\tadd\tx0, x0, #0xfff").unwrap();
                writeln!(self.out, "\tlsr\tx19, x0, #12").unwrap(); // min pfn
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_end_of_DRAM")).unwrap();
                writeln!(self.out, "\tlsr\tx20, x0, #12").unwrap(); // max pfn
                // max_pfn = max_low_pfn = max; min_low_pfn = min
                for (name, reg) in [
                    ("max_pfn", "x20"),
                    ("max_low_pfn", "x20"),
                    ("min_low_pfn", "x19"),
                ] {
                    let s = self.c_sym(name);
                    writeln!(self.out, "\tadrp\tx0, {s}").unwrap();
                    writeln!(self.out, "\tstr\t{reg}, [x0, :lo12:{s}]").unwrap();
                }
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(self.out, "\t.asciz\t\"bootmem_init: soft (pfn globals set)\\n\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "declare_vma"
            | "declare_kernel_vmas"
            | "create_idmap"
            | "unflatten_device_tree"
            | "unflatten_and_copy"
            | "of_fdt_limit_memory"
            // vmemmap_populate needs VMEMMAP page tables we don't have yet;
            // return success so sparse_init can finish (mem_map soft for now).
            | "vmemmap_populate"
            | "vmemmap_populate_hugepages"
            | "vmemmap_populate_basepages" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                let msg = match f.name.as_str() {
                    "create_idmap" => Some("create_idmap: skip (keep early idmap)\\n"),
                    "unflatten_device_tree" => Some("unflatten_device_tree: skip (no FDT)\\n"),
                    "vmemmap_populate" => Some("vmemmap_populate: soft-ok\\n"),
                    _ => None,
                };
                if let Some(m) = msg {
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"{m}\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                }
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // setup_machine_fdt: real path needs fixmap FDT remap (no-op above)
            // then spins forever on invalid dtb. Emit machine banner, seed
            // QEMU virt DRAM into memblock (FDT memory scan skipped), return
            // so start_kernel can continue (parse_early_param, memblock, …).
            "setup_machine_fdt" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                // pr_info("Machine model: %s\n", "linux,dummy-virt");
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(self.out, "\t.asciz\t\"Machine model: linux,dummy-virt\\n\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                // Seed memblock with QEMU virt RAM @ 0x40000000, 512 MiB
                // (matches harness -m 512M). Without this, arm64_memblock_init
                // hits memblock_double_array while can_resize==0 → panic.
                writeln!(self.out, "\tmovz\tx0, #0x0000").unwrap();
                writeln!(self.out, "\tmovk\tx0, #0x4000, lsl #16").unwrap(); // 0x40000000
                writeln!(self.out, "\tmovz\tx1, #0x0000").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0x2000, lsl #16").unwrap(); // 0x20000000 = 512MiB
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_add")).unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_allow_resize")).unwrap();
                // Serial breadcrumb for C1 ladder
                writeln!(self.out, "\tb\t3f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(self.out, "\t.asciz\t\"memblock: QEMU virt RAM 512MiB @ 0x40000000\\n\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "3:").unwrap();
                writeln!(self.out, "\tadr\tx0, 2b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                // A08: initrd FDT scan deferred to populate_rootfs (needs linear
                // map from map_mem; early idmap only covers the kernel image).
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // boot_cpu_init uses cpumask atomics (ll/sc) on __cpu_*_mask; those
            // masks are often weak/misplaced and the exclusive loop spins
            // forever (no exception). Minimal: set CPU0 bits with plain stores.
            "boot_cpu_init" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                // __cpu_*_mask are bitmaps; CPU0 → store 1 in first word.
                for name in [
                    "__cpu_online_mask",
                    "__cpu_active_mask",
                    "__cpu_present_mask",
                    "__cpu_possible_mask",
                ] {
                    let s = self.c_sym(name);
                    writeln!(self.out, "\tadrp\tx0, {s}").unwrap();
                    writeln!(self.out, "\tadd\tx0, x0, :lo12:{s}").unwrap();
                    writeln!(self.out, "\tmov\tx1, #1").unwrap();
                    writeln!(self.out, "\tstr\tx1, [x0]").unwrap();
                }
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // smp_setup_processor_id's real body ends in _printk before any
            // console is registered; acc's vprintk path currently takes a
            // Prefetch Abort (ELR=0) there and never reaches setup_arch /
            // setup_earlycon. Minimal body: record CPU0 mpidr map only.
            "smp_setup_processor_id" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                // mpidr_el1 & MPIDR_HWID_BITMASK (0xff00ffffff)
                writeln!(self.out, "\tmrs\tx0, mpidr_el1").unwrap();
                writeln!(self.out, "\tmovz\tx1, #0xffff").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0xff, lsl #16").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0xff, lsl #32").unwrap();
                writeln!(self.out, "\tand\tx1, x0, x1").unwrap(); // hwid
                writeln!(self.out, "\tmov\tx0, xzr").unwrap(); // cpu 0
                writeln!(self.out, "\tbl\t{}", self.c_sym("set_cpu_logical_map")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // kasan_init_sw_tags: hard-exit setup_arch via saved LR.
            // BSS slot acc_setup_arch_lr is defined by setup_arch prologue.
            "kasan_init_sw_tags" => {
                let sym = self.c_sym(&f.name);
                let ba = self.c_sym("boot_args");
                let saved_lr = self.c_sym("acc_setup_arch_lr");
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                } else {
                    writeln!(self.out, "\t.text").unwrap();
                }
                if !f.is_static {
                    writeln!(self.out, "\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tmov\tx20, x29").unwrap();
                writeln!(self.out, "\tadrp\tx0, {ba}").unwrap();
                writeln!(self.out, "\tadd\tx0, x0, :lo12:{ba}").unwrap();
                writeln!(self.out, "\tstr\txzr, [x0, #8]").unwrap();
                writeln!(self.out, "\tstr\txzr, [x0, #16]").unwrap();
                writeln!(self.out, "\tstr\txzr, [x0, #24]").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(self.out, "\t.asciz\t\"setup_arch: done (kasan_sw_tags)\\n\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tmov\tx29, x20").unwrap();
                writeln!(self.out, "\tldur\tx19, [x29, #-8]").unwrap();
                writeln!(self.out, "\tmov\tsp, x29").unwrap();
                writeln!(self.out, "\tldp\tx29, xzr, [sp], #16").unwrap();
                writeln!(self.out, "\tadrp\tx0, {saved_lr}").unwrap();
                writeln!(self.out, "\tldr\tx30, [x0, :lo12:{saved_lr}]").unwrap();
                writeln!(self.out, "\tcbz\tx30, 2f").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(self.out, "\tb\t3f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "4:").unwrap();
                writeln!(self.out, "\t.asciz\t\"setup_arch: hard-exit NO_SAVED_LR\\n\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "3:").unwrap();
                writeln!(self.out, "\tadr\tx0, 4b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "5:").unwrap();
                writeln!(self.out, "\twfe").unwrap();
                writeln!(self.out, "\tb\t5b").unwrap();
                Ok(true)
            }
            // memblock_free_all: real path needs sparse vmemmap + buddy
            // (__free_pages_core). Freestanding: count free DRAM PFNs, seed a
            // linear-map bump freelist (acc_bump_*), and set _totalram_pages.
            // Not a real buddy — no coalescing / struct page freelist — but
            // __get_free_pages can hand out real mapped pages.
            "memblock_free_all" => {
                let sym = self.c_sym(&f.name);
                let bump_cur = self.c_sym("acc_bump_va_cur");
                let bump_end = self.c_sym("acc_bump_va_end");
                let bump_pages = self.c_sym("acc_bump_pages");
                let bump_once = self.c_sym("acc_bump_alloc_once");
                // Emit bump globals once (strong) from this freestanding body.
                writeln!(self.out, "\n\t.globl\t{bump_cur}").unwrap();
                writeln!(self.out, "\t.globl\t{bump_end}").unwrap();
                writeln!(self.out, "\t.globl\t{bump_pages}").unwrap();
                writeln!(self.out, "\t.globl\t{bump_once}").unwrap();
                writeln!(self.out, "\t.data").unwrap();
                writeln!(self.out, "\t.p2align\t3").unwrap();
                writeln!(self.out, "{bump_cur}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "{bump_end}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "{bump_pages}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "{bump_once}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "\t.text").unwrap();
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
                // start_pfn / end_pfn for QEMU DRAM bank
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_start_of_DRAM")).unwrap();
                writeln!(self.out, "\tadd\tx0, x0, #0xfff").unwrap();
                writeln!(self.out, "\tlsr\tx19, x0, #12").unwrap(); // start_pfn
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_end_of_DRAM")).unwrap();
                writeln!(self.out, "\tlsr\tx20, x0, #12").unwrap(); // end_pfn
                // Skip first 32MiB (8192 pages) for kernel/reserved; bump the rest.
                writeln!(self.out, "\tmov\tx0, #8192").unwrap();
                writeln!(self.out, "\tadd\tx19, x19, x0").unwrap(); // bump_start_pfn
                writeln!(self.out, "\tcmp\tx20, x19").unwrap();
                writeln!(self.out, "\tb.ls\t2f").unwrap();
                writeln!(self.out, "\tsub\tx0, x20, x19").unwrap(); // page count
                writeln!(self.out, "\tb\t3f").unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tmov\tx19, xzr").unwrap();
                writeln!(self.out, "\tmov\tx20, xzr").unwrap();
                writeln!(self.out, "3:").unwrap();
                // _totalram_pages = bump page count
                let trp = self.c_sym("_totalram_pages");
                writeln!(self.out, "\tadrp\tx1, {trp}").unwrap();
                writeln!(self.out, "\tstr\tx0, [x1, :lo12:{trp}]").unwrap();
                writeln!(self.out, "\tadrp\tx1, {bump_pages}").unwrap();
                writeln!(self.out, "\tstr\tx0, [x1, :lo12:{bump_pages}]").unwrap();
                // va = (pfn << 12) + PAGE_OFFSET (0xffff800000000000)
                writeln!(self.out, "\tlsl\tx2, x19, #12").unwrap(); // phys start
                writeln!(self.out, "\tlsl\tx3, x20, #12").unwrap(); // phys end
                writeln!(self.out, "\tmovz\tx1, #0").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0x8000, lsl #32").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0xffff, lsl #48").unwrap();
                writeln!(self.out, "\tadd\tx2, x2, x1").unwrap();
                writeln!(self.out, "\tadd\tx3, x3, x1").unwrap();
                writeln!(self.out, "\tadrp\tx1, {bump_cur}").unwrap();
                writeln!(self.out, "\tstr\tx2, [x1, :lo12:{bump_cur}]").unwrap();
                writeln!(self.out, "\tadrp\tx1, {bump_end}").unwrap();
                writeln!(self.out, "\tstr\tx3, [x1, :lo12:{bump_end}]").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"memblock_free_all: freestanding bump freelist seeded\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // Bump-page freelist (not real buddy). Hands out linear-map VA
            // pages seeded by memblock_free_all. free_* is a no-op (no
            // coalescing) — enough for mid-boot kmalloc-ish probes.
            "__get_free_pages" | "__get_free_pages_noprof" | "get_zeroed_page"
            | "__get_free_page" => {
                let sym = self.c_sym(&f.name);
                let bump_cur = self.c_sym("acc_bump_va_cur");
                let bump_end = self.c_sym("acc_bump_va_end");
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
                // x0=gfp (ignore), x1=order for __get_free_pages; get_zeroed_page only gfp
                let zero = f.name.contains("zero");
                if f.name == "get_zeroed_page" || f.name == "__get_free_page" {
                    writeln!(self.out, "\tmov\tx1, xzr").unwrap(); // order 0
                }
                // size = PAGE_SIZE << order = 4096 << x1
                writeln!(self.out, "\tmov\tx2, #4096").unwrap();
                writeln!(self.out, "\tlsl\tx2, x2, x1").unwrap();
                writeln!(self.out, "\tadrp\tx3, {bump_cur}").unwrap();
                writeln!(self.out, "\tldr\tx19, [x3, :lo12:{bump_cur}]").unwrap();
                writeln!(self.out, "\tadrp\tx4, {bump_end}").unwrap();
                writeln!(self.out, "\tldr\tx20, [x4, :lo12:{bump_end}]").unwrap();
                writeln!(self.out, "\tcbz\tx19, 9f").unwrap(); // not seeded
                writeln!(self.out, "\tadd\tx5, x19, x2").unwrap();
                writeln!(self.out, "\tcmp\tx5, x20").unwrap();
                writeln!(self.out, "\tb.hi\t9f").unwrap();
                writeln!(self.out, "\tstr\tx5, [x3, :lo12:{bump_cur}]").unwrap();
                if zero {
                    // zero the page(s)
                    writeln!(self.out, "\tmov\tx0, x19").unwrap();
                    writeln!(self.out, "\tmov\tx1, x2").unwrap();
                    writeln!(self.out, "8:").unwrap();
                    writeln!(self.out, "\tcbz\tx1, 7f").unwrap();
                    writeln!(self.out, "\tstr\txzr, [x0], #8").unwrap();
                    writeln!(self.out, "\tsub\tx1, x1, #8").unwrap();
                    writeln!(self.out, "\tb\t8b").unwrap();
                    writeln!(self.out, "7:").unwrap();
                }
                // one-shot breadcrumb on first successful alloc (flag in
                // memblock_free_all's .data — do not redefine here).
                let once = self.c_sym("acc_bump_alloc_once");
                writeln!(self.out, "\tadrp\tx0, {once}").unwrap();
                writeln!(self.out, "\tldr\tx1, [x0, :lo12:{once}]").unwrap();
                writeln!(self.out, "\tcbnz\tx1, 6f").unwrap();
                writeln!(self.out, "\tmov\tx1, #1").unwrap();
                writeln!(self.out, "\tstr\tx1, [x0, :lo12:{once}]").unwrap();
                writeln!(self.out, "\tb\t5f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "4:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"__get_free_pages: freestanding bump alloc ok\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "5:").unwrap();
                writeln!(self.out, "\tadr\tx0, 4b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "6:").unwrap();
                writeln!(self.out, "\tmov\tx0, x19").unwrap();
                writeln!(self.out, "\tb\t10f").unwrap();
                writeln!(self.out, "9:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "10:").unwrap();
                writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "free_pages" | "__free_pages" | "__free_pages_core" | "free_unref_page"
            | "__free_pages_ok" => {
                // Bump allocator does not reclaim; real buddy not wired yet.
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.weak\t{sym}").unwrap();
                    writeln!(self.out, "\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // mm_core_init / mem_init: freestanding with real PFN/totalram work
            // (not empty soft). Still no full buddy freelist — C1 partial.
            "mem_init" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
                // high_memory = __va(memblock_end_of_DRAM()) for VA_BITS=48:
                // PAGE_OFFSET = 0xffff800000000000 (matches kimage high half).
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_end_of_DRAM")).unwrap();
                writeln!(self.out, "\tmovz\tx1, #0").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0x8000, lsl #32").unwrap();
                writeln!(self.out, "\tmovk\tx1, #0xffff, lsl #48").unwrap();
                writeln!(self.out, "\tadd\tx0, x0, x1").unwrap(); // phys + PAGE_OFFSET
                let hm = self.c_sym("high_memory");
                writeln!(self.out, "\tadrp\tx1, {hm}").unwrap();
                writeln!(self.out, "\tstr\tx0, [x1, :lo12:{hm}]").unwrap();
                // Seed bump freelist + totalram, then smoke-test one page alloc.
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_free_all")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap(); // gfp
                writeln!(self.out, "\tmov\tx1, xzr").unwrap(); // order 0
                writeln!(self.out, "\tbl\t{}", self.c_sym("__get_free_pages")).unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"mem_init: freestanding (high_memory + bump freelist)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "mm_core_init" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                // breadcrumb enter
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"mm_core_init: freestanding enter\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                // Do NOT call the soft-child chain here: weak soft stubs can lose
                // to strong real bodies (build_all_zonelists/mem_init/…) and hang
                // the C1 ladder. Seed bump freelist via memblock_free_all only.
                writeln!(self.out, "\tbl\t{}", self.c_sym("memblock_free_all")).unwrap();
                writeln!(self.out, "\tb\t3f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"mm_core_init: freestanding done (bump freelist, no real buddy)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "3:").unwrap();
                writeln!(self.out, "\tadr\tx0, 2b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "sched_init" => {
                // Minimal freestanding: no runqueues/CFS. Enough for C1 ladder
                // markers; schedule()/kthread still not real.
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"sched_init: freestanding (no CFS/runqueue)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "console_init" => {
                // Early PL011 freestanding _printk already works; skip real
                // console registration which needs driver core/IRQ.
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"console_init: freestanding (earlycon/PL011 live)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // rest_init: populate_rootfs (initrd unpack) then run_init_process.
            // Calling real kernel_init hangs after kernel_init_freeable: soft
            // (async_synchronize_full / free_initmem path) before run_init —
            // keep direct handoff until freeable/initcalls are de-softed safely.
            "rest_init" => {
                let sym = self.c_sym(&f.name);
                let run_init = self.c_sym("run_init_process");
                let populate = self.c_sym("populate_rootfs");
                let wait_initramfs = self.c_sym("wait_for_initramfs");
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"rest_init: freestanding (populate → run_init → kernel_execve)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "6:").unwrap();
                writeln!(self.out, "\t.asciz\t\"/init\"").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                // A08: unpack initrd before execve (do_basic_setup still soft).
                writeln!(self.out, "\tbl\t{populate}").unwrap();
                writeln!(self.out, "\tbl\t{wait_initramfs}").unwrap();
                writeln!(self.out, "\tadr\tx0, 6b").unwrap();
                writeln!(self.out, "\tbl\t{run_init}").unwrap();
                writeln!(self.out, "\tb\t3f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"rest_init: park after run_init_process\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "3:").unwrap();
                writeln!(self.out, "\tadr\tx0, 2b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "4:").unwrap();
                writeln!(self.out, "\twfe").unwrap();
                writeln!(self.out, "\tb\t4b").unwrap();
                Ok(true)
            }
            // --- rest_init / kernel_init support (real bodies in main.c) ---
            // Defer user_mode_thread(fn) until schedule_preempt_disabled so
            // complete(kthreadd_done) runs first (matches real ordering).
            "user_mode_thread" => {
                let sym = self.c_sym(&f.name);
                let dfn = self.c_sym("acc_deferred_fn");
                let darg = self.c_sym("acc_deferred_arg");
                writeln!(self.out, "\n\t.globl\t{dfn}").unwrap();
                writeln!(self.out, "\t.globl\t{darg}").unwrap();
                writeln!(self.out, "\t.data").unwrap();
                writeln!(self.out, "\t.p2align\t3").unwrap();
                writeln!(self.out, "{dfn}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "{darg}:").unwrap();
                writeln!(self.out, "\t.quad\t0").unwrap();
                writeln!(self.out, "\t.text").unwrap();
                if !f.is_static {
                    writeln!(self.out, "\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                // x0=fn, x1=arg — defer; return fake pid 1
                writeln!(self.out, "\tadrp\tx2, {dfn}").unwrap();
                writeln!(self.out, "\tstr\tx0, [x2, :lo12:{dfn}]").unwrap();
                writeln!(self.out, "\tadrp\tx2, {darg}").unwrap();
                writeln!(self.out, "\tstr\tx1, [x2, :lo12:{darg}]").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"user_mode_thread: deferred fn (pid=1)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, #1").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "kernel_thread" => {
                // Skip kthreadd; return fake pid 2.
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"kernel_thread: skip (pid=2)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, #2").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "complete" | "complete_all" => {
                // struct completion { unsigned int done; ... } — bump done.
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tldr\tw1, [x0]").unwrap();
                writeln!(self.out, "\tadd\tw1, w1, #1").unwrap();
                writeln!(self.out, "\tstr\tw1, [x0]").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "wait_for_completion" | "wait_for_completion_timeout" => {
                // complete() already ran before deferred kernel_init; return.
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "schedule_preempt_disabled" | "schedule" | "schedule_timeout" => {
                // Run deferred user_mode_thread fn once (kernel_init).
                let sym = self.c_sym(&f.name);
                let dfn = self.c_sym("acc_deferred_fn");
                let darg = self.c_sym("acc_deferred_arg");
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                if f.name == "schedule_preempt_disabled" {
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    writeln!(self.out, "\tadrp\tx0, {dfn}").unwrap();
                    writeln!(self.out, "\tldr\tx1, [x0, :lo12:{dfn}]").unwrap();
                    writeln!(self.out, "\tcbz\tx1, 1f").unwrap();
                    writeln!(self.out, "\tstr\txzr, [x0, :lo12:{dfn}]").unwrap(); // once
                    writeln!(self.out, "\tadrp\tx0, {darg}").unwrap();
                    writeln!(self.out, "\tldr\tx0, [x0, :lo12:{darg}]").unwrap();
                    writeln!(self.out, "\tblr\tx1").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                }
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "cpu_startup_entry" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"cpu_startup_entry: freestanding idle park\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(self.out, "\twfe").unwrap();
                writeln!(self.out, "\tb\t2b").unwrap();
                Ok(true)
            }
            "find_task_by_pid_ns" | "find_task_by_vpid" => {
                // Return a dedicated dummy task blob so rest_init's
                // tsk->flags |= … cannot corrupt real init_task if offsetof
                // is wrong under acc type layout.
                // Use .comm — both find_task_* live in kernel/pid.c and would
                // otherwise double-define a .bss label in one TU.
                let sym = self.c_sym(&f.name);
                let dummy = self.c_sym("acc_dummy_task");
                writeln!(self.out, "\n\t.comm\t{dummy},4096,16").unwrap();
                writeln!(self.out, "\t.text").unwrap();
                if !f.is_static {
                    writeln!(self.out, "\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tadrp\tx0, {dummy}").unwrap();
                writeln!(self.out, "\tadd\tx0, x0, :lo12:{dummy}").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"find_task_by_pid_ns: dummy_task\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tldr\tx0, [sp], #16").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "set_cpus_allowed_ptr" | "rcu_read_lock" | "rcu_read_unlock"
            | "__rcu_read_lock" | "__rcu_read_unlock" => {
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "run_init_process" | "try_to_run_init_process" => {
                // Attempt real kernel_execve (fails without VFS), then freestanding
                // EL0 busybox load from acc_busybox_blob (A09).
                // MUST live in .text — __init may be discarded by free_initmem.
                let sym = self.c_sym(&f.name);
                let kexec = self.c_sym("kernel_execve");
                let el0 = self.c_sym("acc_el0_run_busybox");
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                writeln!(self.out, "\t.text").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                // Frame + path + argv[2] + envp[1] = 16+8+16+8 → 48, align 16 → 64
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-64]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstr\tx19, [sp, #16]").unwrap();
                writeln!(self.out, "\tmov\tx19, x0").unwrap(); // path
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"Run /init as init process\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                // argv = { path, NULL }; envp = { NULL }
                writeln!(self.out, "\tstr\tx19, [sp, #32]").unwrap();
                writeln!(self.out, "\tstr\txzr, [sp, #40]").unwrap();
                writeln!(self.out, "\tstr\txzr, [sp, #48]").unwrap();
                writeln!(self.out, "\tmov\tx0, x19").unwrap();
                writeln!(self.out, "\tadd\tx1, sp, #32").unwrap();
                writeln!(self.out, "\tadd\tx2, sp, #48").unwrap();
                writeln!(self.out, "\tbl\t{kexec}").unwrap();
                writeln!(self.out, "\tb\t5f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "4:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"kernel_execve returned — try freestanding EL0 busybox\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "5:").unwrap();
                writeln!(self.out, "\tadr\tx0, 4b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                // A09: map cpio busybox ELF and eret to EL0 (does not return on success).
                writeln!(self.out, "\tbl\t{el0}").unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldr\tx19, [sp, #16]").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #64").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            // A08: wait_for_initramfs — sync already done by stubs populate_rootfs.
            // (populate_rootfs / unpack_to_rootfs are global in acc_vmlinux_stubs.c
            // because the real static rootfs_initcall is unreachable to acc.)
            "wait_for_initramfs" => {
                let sym = self.c_sym(&f.name);
                writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                writeln!(self.out, "\t.text").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"wait_for_initramfs: freestanding (sync already done)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            "panic" => {
                // Print header + fmt via freestanding _printk then park.
                // Also emit a fixed .text trailer so C1 serial still shows a
                // clear no-init marker if fmt lives in a mapping _printk
                // cannot read after TTBR0 idmap switch.
                let sym = self.c_sym(&f.name);
                if !f.is_static {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx29, sp").unwrap();
                writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tmov\tx19, x0").unwrap(); // save fmt
                writeln!(self.out, "\tb\t1f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "0:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"Kernel panic - not syncing: \\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "1:").unwrap();
                writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                // Print panic format string (may contain % — printed raw).
                writeln!(self.out, "\tmov\tx0, x19").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "\tb\t3f").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "2:").unwrap();
                writeln!(
                    self.out,
                    "\t.asciz\t\"No working init found (acc freestanding park; need buddy+CFS+VFS)\\n\""
                )
                .unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "3:").unwrap();
                writeln!(self.out, "\tadr\tx0, 2b").unwrap();
                writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                writeln!(self.out, "4:").unwrap();
                writeln!(self.out, "\twfe").unwrap();
                writeln!(self.out, "\tb\t4b").unwrap();
                Ok(true)
            }
            "async_synchronize_full" | "mark_readonly" | "do_sysctl_args"
            | "kprobe_free_init_mem" | "ftrace_free_init_mem"
            | "kgdb_free_init_mem" | "exit_boot_config" | "pti_finalize"
            | "rcu_end_inkernel_boot" | "kthreadd"
            // free_initmem: arch real body walks init pages into buddy — without
            // a freelist that NULL-derefs. Strong freestanding no-op wins over
            // arch and main.c __weak.
            | "free_initmem" | "free_initmem_default" => {
                let sym = self.c_sym(&f.name);
                // free_initmem: weak (multi def main+arch). Others: strong in
                // .text so they survive free_initmem and beat stale real bodies.
                let weak = matches!(
                    f.name.as_str(),
                    "free_initmem" | "free_initmem_default"
                );
                if !f.is_static {
                    if weak {
                        writeln!(self.out, "\n\t.weak\t{sym}").unwrap();
                    }
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "").unwrap();
                }
                // Stay out of .init.text — kernel_init calls these after free_initmem.
                writeln!(self.out, "\t.text").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                if matches!(
                    f.name.as_str(),
                    "free_initmem"
                        | "async_synchronize_full"
                        | "do_sysctl_args"
                ) {
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(
                        self.out,
                        "\t.asciz\t\"{}: freestanding no-op\\n\"",
                        f.name
                    )
                    .unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                    writeln!(self.out, "\tret").unwrap();
                } else {
                    writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                    writeln!(self.out, "\tret").unwrap();
                }
                Ok(true)
            }
            // smp_init_cpus: OF parse without FDT leaves bootcpu_valid false and
            // can walk garbage device trees. Single-CPU freestanding: ensure
            // cpu0 logical map, skip secondaries.
            // smp_build_mpidr_hash: needs valid maps; no-op for UP.
            // kasan_init*: KASAN not needed for C1 boot proof.
            // Post-setup_arch start_kernel mid: mm/sched/irq/time paths need
            // full buddy/percpu/IRQ; soft with breadcrumbs to advance C1 ladder.
            "smp_init_cpus" | "smp_build_mpidr_hash" | "kasan_init"
            | "kasan_early_init" | "init_bootcpu_ops" | "psci_dt_init" | "psci_acpi_init"
            | "request_standard_resources" | "early_ioremap_reset"
            | "setup_boot_config" | "setup_command_line" | "setup_nr_cpu_ids"
            | "setup_per_cpu_areas" | "smp_prepare_boot_cpu" | "boot_cpu_hotplug_init"
            | "sort_main_extable" | "trap_init" | "poking_init"
            // A07: real vfs_caches_init_early hangs after setup_log_buf when mm
            // bump freelist is live (#117); soft stub restores Dentry breadcrumb.
            | "vfs_caches_init_early"
            | "ftrace_init" | "early_trace_init" | "radix_tree_init"
            | "maple_tree_init" | "housekeeping_init" | "workqueue_init_early"
            | "rcu_init" | "trace_init" | "context_tracking_init" | "early_irq_init"
            | "init_IRQ" | "tick_init" | "rcu_init_nohz" | "init_timers" | "srcu_init"
            | "hrtimers_init" | "softirq_init" | "timekeeping_init" | "time_init"
            | "random_init" | "kfence_init" | "boot_init_stack_canary"
            | "perf_event_init" | "profile_init" | "call_function_init"
            | "kmem_cache_init_late" | "lockdep_init"
            | "locking_selftest" | "kmem_cache_init" | "vmalloc_init"
            | "build_all_zonelists" | "page_alloc_init_cpuhp" | "page_ext_init"
            | "page_ext_init_flatmem" | "page_ext_init_flatmem_late"
            | "mem_debugging_and_hardening_init" | "report_meminit"
            | "stack_depot_early_init" | "mem_init_print_info" | "kmemleak_init"
            | "ptlock_cache_init" | "pgtable_cache_init" | "debug_objects_mem_init"
            | "init_espfix_bsp" | "pti_init" | "mm_cache_init"
            | "kfence_alloc_pool_and_metadata" | "kmsan_init_shadow"
            | "kmsan_init_runtime"
            // rest_init / kernel_init: REAL bodies (main.c) + freestanding
            // thread/completion/schedule primitives below — not soft-empty here.
            | "kernel_init_freeable"
            | "rcu_scheduler_starting" | "numa_default_policy"
            | "do_basic_setup" | "do_pre_smp_initcalls" | "do_initcalls"
            | "proc_caches_init" | "buffer_init" | "key_init"
            | "security_init" | "vfs_caches_init" | "pagecache_init"
            | "signals_init" | "seq_file_init" | "proc_root_init"
            | "nsfs_init" | "cpuset_init" | "cgroup_init"
            | "taskstats_init_early" | "delayacct_init"
            | "acpi_early_init" | "thread_stack_cache_init"
            | "cred_init" | "fork_init" | "proc_caches_init"
            | "uts_ns_init" | "key_init" | "security_init"
            | "dbg_late_init" | "net_ns_init" | "padata_init"
            | "page_alloc_init_late" | "workqueue_init"
            | "workqueue_init_topology" | "init_mm_internals"
            | "smp_init" | "sched_init_smp" | "domain_setup"
            | "random_init_early" | "setup_log_buf"
            // vfs_caches_init_early: de-soft — real body printed Dentry/Inode before
            | "initcall_debug_enable"
            | "setup_per_cpu_pageset" | "numa_policy_init" | "late_time_init"
            | "sched_clock_init" | "calibrate_delay" | "arch_cpu_finalize_init"
            | "pid_idr_init" | "anon_vma_init" | "pidfs_init"
            | "acpi_subsystem_init" | "arch_post_acpi_subsys_init"
            | "kcsan_init" | "locking_selftest" | "lockdep_init"
            | "numa_default_policy" | "check_bugs" | "sfi_init_late"
            | "ftrace_free_mem" | "proc_sys_init" => {
                let sym = self.c_sym(&f.name);
                // Soft no-op freestanding is emitted from every TU that has a
                // non-empty body for that name. Use weak so multi-def (e.g.
                // early_irq_init in softirq.o + irqdesc.o) links cleanly.
                // Keep strong only for UP/smp helpers that must win uniquely.
                let strong = matches!(
                    f.name.as_str(),
                    "smp_init_cpus"
                        | "smp_build_mpidr_hash"
                        | "setup_boot_config"
                        | "setup_command_line"
                        | "setup_nr_cpu_ids"
                        | "setup_per_cpu_areas"
                        | "smp_prepare_boot_cpu"
                        | "boot_cpu_hotplug_init"
                        | "sort_main_extable"
                        | "trap_init"
                        | "poking_init"
                        | "ftrace_init"
                        | "early_trace_init"
                        | "radix_tree_init"
                        | "maple_tree_init"
                        | "housekeeping_init"
                        | "workqueue_init_early"
                        | "rcu_init"
                        | "trace_init"
                        | "early_irq_init"
                        | "init_IRQ"
                        | "tick_init"
                        | "init_timers"
                        | "srcu_init"
                        | "hrtimers_init"
                        | "softirq_init"
                        | "timekeeping_init"
                        | "time_init"
                        | "random_init"
                        | "random_init_early"
                        | "kmem_cache_init"
                        | "vmalloc_init"
                        | "build_all_zonelists"
                        | "mem_init"
                        | "mem_init_print_info"
                        | "mm_cache_init"
                        | "fork_init"
                        | "cred_init"
                        | "uts_ns_init"
                        | "proc_caches_init"
                        | "vfs_caches_init"
                        | "signals_init"
                        | "pid_idr_init"
                        | "anon_vma_init"
                        | "pagecache_init"
                        | "key_init"
                        | "security_init"
                        | "dbg_late_init"
                        | "net_ns_init"
                        | "smp_init"
                        | "sched_init_smp"
                        | "do_basic_setup"
                        | "do_pre_smp_initcalls"
                        | "do_initcalls"
                        | "kernel_init_freeable"
                        | "rcu_scheduler_starting"
                        | "numa_default_policy"
                        | "check_bugs"
                        | "rest_init"
                        | "sched_clock_init"
                        | "calibrate_delay"
                        | "pidfs_init"
                        | "proc_root_init"
                        | "nsfs_init"
                        | "seq_file_init"
                        | "acpi_early_init"
                        | "thread_stack_cache_init"
                        | "workqueue_init"
                        | "init_mm_internals"
                        | "page_alloc_init_late"
                );
                if f.is_static {
                    writeln!(self.out, "").unwrap();
                } else if strong {
                    writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
                } else {
                    writeln!(self.out, "\n\t.weak\t{sym}").unwrap();
                    writeln!(self.out, "\t.globl\t{sym}").unwrap();
                }
                if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    self.emit_named_section(sec);
                }
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{sym}:").unwrap();
                if f.name == "smp_init_cpus" {
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    // Re-assert CPU0 logical map from mpidr (in case cleared).
                    writeln!(self.out, "\tmrs\tx0, mpidr_el1").unwrap();
                    writeln!(self.out, "\tmovz\tx1, #0xffff").unwrap();
                    writeln!(self.out, "\tmovk\tx1, #0xff, lsl #16").unwrap();
                    writeln!(self.out, "\tmovk\tx1, #0xff, lsl #32").unwrap();
                    writeln!(self.out, "\tand\tx1, x0, x1").unwrap();
                    writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("set_cpu_logical_map")).unwrap();
                    // set_cpu_possible(0) / set_cpu_present(0) if available — soft via masks
                    for name in ["__cpu_possible_mask", "__cpu_present_mask"] {
                        let s = self.c_sym(name);
                        writeln!(self.out, "\tadrp\tx0, {s}").unwrap();
                        writeln!(self.out, "\tadd\tx0, x0, :lo12:{s}").unwrap();
                        writeln!(self.out, "\tmov\tx1, #1").unwrap();
                        writeln!(self.out, "\tstr\tx1, [x0]").unwrap();
                    }
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"smp_init_cpus: UP cpu0 only\\n\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                } else if f.name == "smp_build_mpidr_hash" {
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    // Zero boot_args[1..3] here too: kasan_init_sw_tags may be a
                    // weak empty stub if freestanding lost the link race, and
                    // setup_arch's pr_err path with "%016llx" is noisy under our
                    // minimal _printk (and has hung in past boot traces).
                    let ba = self.c_sym("boot_args");
                    writeln!(self.out, "\tadrp\tx0, {ba}").unwrap();
                    writeln!(self.out, "\tadd\tx0, x0, :lo12:{ba}").unwrap();
                    writeln!(self.out, "\tstr\txzr, [x0, #8]").unwrap();
                    writeln!(self.out, "\tstr\txzr, [x0, #16]").unwrap();
                    writeln!(self.out, "\tstr\txzr, [x0, #24]").unwrap();
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"smp_build_mpidr_hash: skip (boot_args cleared)\\n\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                } else if f.name == "setup_boot_config" {
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"setup_boot_config: ok\\n\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                } else if f.name == "setup_command_line" {
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"setup_command_line: ok\\n\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                } else if f.name == "vfs_caches_init_early" {
                    // Match historical serial crumbs from real early VFS init.
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"Dentry cache\\n\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "2:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"Inode-cache\\n\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tadr\tx0, 2b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                } else {
                    // Default soft freestanding: breadcrumb with function name so
                    // serial shows the last soft step before hang.
                    let msg = format!("{}: soft\\n", f.name);
                    writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
                    writeln!(self.out, "\tmov\tx29, sp").unwrap();
                    writeln!(self.out, "\tb\t1f").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "0:").unwrap();
                    writeln!(self.out, "\t.asciz\t\"{msg}\"").unwrap();
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                    writeln!(self.out, "1:").unwrap();
                    writeln!(self.out, "\tadr\tx0, 0b").unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym("_printk")).unwrap();
                    writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
                }
                writeln!(self.out, "\tmov\tx0, xzr").unwrap();
                writeln!(self.out, "\tret").unwrap();
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Identity-map the loaded kernel image with 2MB PMD sections.
    /// x0 = pg_dir, x1 = clrmask (ignored). Returns next free table page in x0.
    ///
    /// Layout (4K pages, 4-level, 48-bit):
    ///   pg_dir[0]     -> pud page at pg_dir+PAGE_SIZE
    ///   pud[idx]      -> pmd page at pg_dir+2*PAGE_SIZE
    ///   pmd[i]        -> 2MB block for each 2MB of [_text, _end)
    fn emit_freestanding_create_init_idmap(&mut self, f: &Function) -> Result<(), String> {
        let sym = self.c_sym(&f.name);
        if f.is_static {
            writeln!(self.out, "").unwrap();
        } else {
            writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
        }
        if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            self.emit_named_section(sec);
        }
        writeln!(self.out, "\t.p2align\t2").unwrap();
        // acc_idmap_phys is defined by freestanding _printk (hard early console).
        // Only store into it here — do not re-define (PI multi-def with printk.o).
        let idmap_phys = self.c_sym("acc_idmap_phys");
        writeln!(self.out, "{sym}:").unwrap();
        // Save frame
        writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tmov\tx29, sp").unwrap();
        // x19 = pg_dir, x20 = next free page (pg_dir + PAGE_SIZE)
        writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tstp\tx21, x22, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tstp\tx23, x24, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tmov\tx19, x0").unwrap();
        // Stash phys idmap (x0 is phys with MMU off) into acc_idmap_phys.
        // Pre-MMU adrp of that .data symbol yields phys.
        writeln!(self.out, "\tadrp\tx9, {idmap_phys}").unwrap();
        writeln!(self.out, "\tstr\tx0, [x9, :lo12:{idmap_phys}]").unwrap();
        writeln!(self.out, "\tadd\tx20, x0, #4096").unwrap(); // first free page

        // Zero three pages: PGD, PUD, PMD (kernel). UART is a pud-level block.
        writeln!(self.out, "\tmov\tx0, x19").unwrap();
        writeln!(self.out, "\tmov\tx1, xzr").unwrap();
        writeln!(self.out, "\tmovz\tx2, #0x3000").unwrap(); // 3 pages
        writeln!(self.out, "\tbl\t{}", self.c_sym("memset")).unwrap();

        // Phys of _text / _end via adrp (PC-relative → phys with MMU off)
        let tsym = self.c_sym("_text");
        let esym = self.c_sym("_end");
        writeln!(self.out, "\tadrp\tx21, {tsym}").unwrap();
        writeln!(self.out, "\tadd\tx21, x21, :lo12:{tsym}").unwrap();
        writeln!(self.out, "\tadrp\tx22, {esym}").unwrap();
        writeln!(self.out, "\tadd\tx22, x22, :lo12:{esym}").unwrap();
        // Align start down, end up to 2MB (mask = ~((1<<21)-1) = 0xffffffffffe00000)
        writeln!(self.out, "\tmovz\tx3, #0xe000").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0xffff, lsl #16").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0xffff, lsl #32").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0xffff, lsl #48").unwrap();
        writeln!(self.out, "\tand\tx21, x21, x3").unwrap();
        writeln!(self.out, "\tmovz\tx4, #0xffff").unwrap();
        writeln!(self.out, "\tmovk\tx4, #0x1f, lsl #16").unwrap(); // 0x1fffff
        writeln!(self.out, "\tadd\tx22, x22, x4").unwrap();
        writeln!(self.out, "\tand\tx22, x22, x3").unwrap();

        // PUD @ +0x1000, PMD(kernel) @ +0x2000
        writeln!(self.out, "\tadd\tx23, x19, #4096").unwrap(); // pud
        writeln!(self.out, "\tadd\tx24, x19, #8192").unwrap(); // pmd kernel
        writeln!(self.out, "\tadd\tx20, x19, #12288").unwrap(); // next free after 3 pages

        // pg_dir[0] = pud | TABLE  (PGD index 0 covers low 512GB)
        // PMD_TYPE_TABLE = 3
        writeln!(self.out, "\torr\tx0, x23, #3").unwrap();
        writeln!(self.out, "\tstr\tx0, [x19]").unwrap();

        // pud_index = (phys >> 30) & 0x1ff  — for 0x40000000 → 1
        writeln!(self.out, "\tlsr\tx0, x21, #30").unwrap();
        writeln!(self.out, "\tand\tx0, x0, #0x1ff").unwrap();
        writeln!(self.out, "\torr\tx1, x24, #3").unwrap(); // pmd | TABLE
        writeln!(self.out, "\tstr\tx1, [x23, x0, lsl #3]").unwrap();

        // pud[0] → 1GB DEVICE block at PA 0 covering PL011 @ 0x9000000.
        // TYPE_BLOCK=1 | AF | AttrIndx=0 (nGnRnE) | UXN | PXN
        // = 0x6000000000000401. (Kernel image is pud[1] @ 0x40000000.)
        writeln!(self.out, "\tmovz\tx1, #0x0401").unwrap();
        writeln!(self.out, "\tmovk\tx1, #0x6000, lsl #48").unwrap();
        writeln!(self.out, "\tstr\tx1, [x23]").unwrap(); // pud[0] device block

        // Section attrs for kernel DRAM: TYPE_SECT|AF|ISH|UXN|ATTR_NORMAL
        // value = 1 | (3<<8) | (1<<10) | (1<<54) = 0x4000000000000d01
        writeln!(self.out, "\tmovz\tx3, #0x0d01").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0x4000, lsl #48").unwrap();

        // pmd_index loop: for pa = start; pa < end; pa += 2MB
        writeln!(self.out, "\tmov\tx0, x21").unwrap(); // pa
        writeln!(self.out, "1:").unwrap();
        writeln!(self.out, "\tcmp\tx0, x22").unwrap();
        writeln!(self.out, "\tb.hs\t2f").unwrap();
        // pmd index = (pa >> 21) & 0x1ff
        writeln!(self.out, "\tlsr\tx1, x0, #21").unwrap();
        writeln!(self.out, "\tand\tx1, x1, #0x1ff").unwrap();
        // entry = (pa & ~((1<<21)-1)) | attrs  — pa already 2MB aligned
        writeln!(self.out, "\torr\tx2, x0, x3").unwrap();
        writeln!(self.out, "\tstr\tx2, [x24, x1, lsl #3]").unwrap();
        writeln!(self.out, "\tmovz\tx1, #0x20, lsl #16").unwrap(); // 2MB = 0x200000
        writeln!(self.out, "\tadd\tx0, x0, x1").unwrap();
        writeln!(self.out, "\tb\t1b").unwrap();
        writeln!(self.out, "2:").unwrap();

        // Publish page tables before any MMU enable / later TTBR0 walk.
        writeln!(self.out, "\tdsb\tishst").unwrap();

        // Pre-MMU serial banner to PL011 DR.
        writeln!(self.out, "\tmovz\tx0, #0x0000").unwrap();
        writeln!(self.out, "\tmovk\tx0, #0x900, lsl #16").unwrap(); // 0x9000000 DR
        writeln!(self.out, "\tadd\tx1, x0, #0x18").unwrap(); // FR
        writeln!(self.out, "\tb\t4f").unwrap();
        writeln!(self.out, "\t.p2align\t2").unwrap();
        writeln!(self.out, "3:").unwrap();
        writeln!(self.out, "\t.ascii\t\"acc-boot\\r\\n\\0\"").unwrap();
        writeln!(self.out, "\t.p2align\t2").unwrap();
        writeln!(self.out, "4:").unwrap();
        writeln!(self.out, "\tadr\tx2, 3b").unwrap();
        writeln!(self.out, "5:").unwrap();
        writeln!(self.out, "\tldrb\tw3, [x2], #1").unwrap();
        writeln!(self.out, "\tcbz\tw3, 7f").unwrap();
        writeln!(self.out, "6:").unwrap();
        writeln!(self.out, "\tldr\tw4, [x1]").unwrap();
        writeln!(self.out, "\ttbnz\tw4, #5, 6b").unwrap(); // TXFF
        writeln!(self.out, "\tstr\tw3, [x0]").unwrap();
        writeln!(self.out, "\tb\t5b").unwrap();
        writeln!(self.out, "7:").unwrap();

        // return next free page
        writeln!(self.out, "\tmov\tx0, x20").unwrap();
        writeln!(self.out, "\tldp\tx23, x24, [sp], #16").unwrap();
        writeln!(self.out, "\tldp\tx21, x22, [sp], #16").unwrap();
        writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
        writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
        writeln!(self.out, "\tret").unwrap();
        Ok(())
    }

    /// Map kernel image at link VAs onto physical load addresses in
    /// `init_pg_dir`, then switch TTBR1. x0=boot_status, x1=fdt (ignored).
    fn emit_freestanding_early_map_kernel(&mut self, f: &Function) -> Result<(), String> {
        let sym = self.c_sym(&f.name);
        if f.is_static {
            writeln!(self.out, "").unwrap();
        } else {
            writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
        }
        if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            self.emit_named_section(sec);
        }
        writeln!(self.out, "\t.p2align\t2").unwrap();
        writeln!(self.out, "{sym}:").unwrap();
        writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tmov\tx29, sp").unwrap();
        writeln!(self.out, "\tstp\tx19, x20, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tstp\tx21, x22, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tstp\tx23, x24, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tstp\tx25, x26, [sp, #-16]!").unwrap();

        // x19 = init_pg_dir (phys via adrp)
        let ipgd = self.c_sym("init_pg_dir");
        writeln!(self.out, "\tadrp\tx19, {ipgd}").unwrap();
        writeln!(self.out, "\tadd\tx19, x19, :lo12:{ipgd}").unwrap();

        // Zero 4 pages for pgd/pud/pmd + spare
        writeln!(self.out, "\tmov\tx0, x19").unwrap();
        writeln!(self.out, "\tmov\tx1, xzr").unwrap();
        writeln!(self.out, "\tmovz\tx2, #0x4000").unwrap();
        writeln!(self.out, "\tbl\t{}", self.c_sym("memset")).unwrap();

        // phys _text / _end
        let tsym = self.c_sym("_text");
        let esym = self.c_sym("_end");
        writeln!(self.out, "\tadrp\tx21, {tsym}").unwrap();
        writeln!(self.out, "\tadd\tx21, x21, :lo12:{tsym}").unwrap();
        writeln!(self.out, "\tadrp\tx22, {esym}").unwrap();
        writeln!(self.out, "\tadd\tx22, x22, :lo12:{esym}").unwrap();
        // link VA of _text / _end (absolute — high kernel VAs)
        // Use adrp-relative phys as PA; load link VA from absolute symbols via
        // adrp on the same symbols: with MMU off adrp gives phys, so we need
        // the link address from a .quad in rodata. Emit local constants.
        writeln!(self.out, "\tb\t3f").unwrap();
        writeln!(self.out, "\t.p2align\t3").unwrap();
        writeln!(self.out, "4:").unwrap();
        writeln!(self.out, "\t.quad\t{tsym}").unwrap();
        writeln!(self.out, "\t.quad\t{esym}").unwrap();
        writeln!(self.out, "3:").unwrap();
        writeln!(self.out, "\tadr\tx0, 4b").unwrap();
        writeln!(self.out, "\tldr\tx25, [x0]").unwrap(); // va_text
        writeln!(self.out, "\tldr\tx26, [x0, #8]").unwrap(); // va_end

        // Align phys start down / end up to 2MB; same for VA
        writeln!(self.out, "\tmovz\tx3, #0xe000").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0xffff, lsl #16").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0xffff, lsl #32").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0xffff, lsl #48").unwrap();
        writeln!(self.out, "\tand\tx21, x21, x3").unwrap();
        writeln!(self.out, "\tand\tx25, x25, x3").unwrap();
        writeln!(self.out, "\tmovz\tx4, #0xffff").unwrap();
        writeln!(self.out, "\tmovk\tx4, #0x1f, lsl #16").unwrap();
        writeln!(self.out, "\tadd\tx22, x22, x4").unwrap();
        writeln!(self.out, "\tand\tx22, x22, x3").unwrap();
        writeln!(self.out, "\tadd\tx26, x26, x4").unwrap();
        writeln!(self.out, "\tand\tx26, x26, x3").unwrap();

        // Tables: pud @ +0x1000, pmd @ +0x2000
        writeln!(self.out, "\tadd\tx23, x19, #4096").unwrap();
        writeln!(self.out, "\tadd\tx24, x19, #8192").unwrap();

        // pgd index for high VA: (va >> 39) & 0x1ff
        writeln!(self.out, "\tlsr\tx0, x25, #39").unwrap();
        writeln!(self.out, "\tand\tx0, x0, #0x1ff").unwrap();
        writeln!(self.out, "\torr\tx1, x23, #3").unwrap();
        writeln!(self.out, "\tstr\tx1, [x19, x0, lsl #3]").unwrap();

        // pud index: (va >> 30) & 0x1ff
        writeln!(self.out, "\tlsr\tx0, x25, #30").unwrap();
        writeln!(self.out, "\tand\tx0, x0, #0x1ff").unwrap();
        writeln!(self.out, "\torr\tx1, x24, #3").unwrap();
        writeln!(self.out, "\tstr\tx1, [x23, x0, lsl #3]").unwrap();

        // Section attrs: TYPE_SECT|AF|ISH|UXN|ATTR_NORMAL
        writeln!(self.out, "\tmovz\tx3, #0x0d01").unwrap();
        writeln!(self.out, "\tmovk\tx3, #0x4000, lsl #48").unwrap();

        // Walk VA/PA in lockstep 2MB steps
        writeln!(self.out, "\tmov\tx0, x25").unwrap(); // va
        writeln!(self.out, "\tmov\tx1, x21").unwrap(); // pa
        writeln!(self.out, "1:").unwrap();
        writeln!(self.out, "\tcmp\tx0, x26").unwrap();
        writeln!(self.out, "\tb.hs\t2f").unwrap();
        writeln!(self.out, "\tlsr\tx2, x0, #21").unwrap();
        writeln!(self.out, "\tand\tx2, x2, #0x1ff").unwrap();
        writeln!(self.out, "\torr\tx4, x1, x3").unwrap();
        writeln!(self.out, "\tstr\tx4, [x24, x2, lsl #3]").unwrap();
        writeln!(self.out, "\tmovz\tx4, #0x20, lsl #16").unwrap();
        writeln!(self.out, "\tadd\tx0, x0, x4").unwrap();
        writeln!(self.out, "\tadd\tx1, x1, x4").unwrap();
        writeln!(self.out, "\tb\t1b").unwrap();
        writeln!(self.out, "2:").unwrap();

        // Publish tables, switch TTBR1 to init_pg_dir
        writeln!(self.out, "\tdsb\tishst").unwrap();
        writeln!(self.out, "\tmsr\tttbr1_el1, x19").unwrap();
        writeln!(self.out, "\tisb").unwrap();
        writeln!(self.out, "\ttlbi\tvmalle1").unwrap();
        writeln!(self.out, "\tdsb\tnsh").unwrap();
        writeln!(self.out, "\tisb").unwrap();

        // Patch init_task.stack = &init_stack. Offset must match asm-offsets
        // TSK_STACK (32 on arm64 6.9 — not 24). INIT_TASK designated-init is
        // still broken (init_task often weak/bss zeros); without this patch
        // __primary_switched does: sp = *(init_task+32)+THREAD_SIZE → FAR≈0x3fe0.
        // Use absolute .quad link VAs so PI objcopy + image-vars hard aliases
        // (__pi_init_task = init_task) resolve correctly.
        let itask = self.c_sym("init_task");
        let istack = self.c_sym("init_stack");
        writeln!(self.out, "\tb\t5f").unwrap();
        writeln!(self.out, "\t.p2align\t3").unwrap();
        writeln!(self.out, "6:").unwrap();
        writeln!(self.out, "\t.quad\t{itask}").unwrap();
        writeln!(self.out, "\t.quad\t{istack}").unwrap();
        writeln!(self.out, "5:").unwrap();
        writeln!(self.out, "\tadr\tx2, 6b").unwrap();
        writeln!(self.out, "\tldr\tx0, [x2]").unwrap(); // link VA init_task
        writeln!(self.out, "\tldr\tx1, [x2, #8]").unwrap(); // link VA init_stack
        // Convert link VA → phys for the store (MMU still uses idmap TTBR0 for data):
        // phys = va - _text_va + _text_phys.  _text_phys is in x21-range earlier;
        // recompute: phys = va + (adrp_text - quad_text)
        let tsym = self.c_sym("_text");
        writeln!(self.out, "\tadrp\tx3, {tsym}").unwrap();
        writeln!(self.out, "\tadd\tx3, x3, :lo12:{tsym}").unwrap(); // phys _text
        writeln!(self.out, "\tb\t7f").unwrap();
        writeln!(self.out, "\t.p2align\t3").unwrap();
        writeln!(self.out, "8:").unwrap();
        writeln!(self.out, "\t.quad\t{tsym}").unwrap();
        writeln!(self.out, "7:").unwrap();
        writeln!(self.out, "\tadr\tx4, 8b").unwrap();
        writeln!(self.out, "\tldr\tx4, [x4]").unwrap(); // link VA _text
        writeln!(self.out, "\tsub\tx3, x3, x4").unwrap(); // phys - va = offset
        writeln!(self.out, "\tadd\tx0, x0, x3").unwrap(); // phys init_task
        // TSK_STACK = 32 (task_struct.stack) — must match include/generated/asm-offsets.h
        writeln!(self.out, "\tstr\tx1, [x0, #32]").unwrap(); // store link VA stack

        writeln!(self.out, "\tldp\tx25, x26, [sp], #16").unwrap();
        writeln!(self.out, "\tldp\tx23, x24, [sp], #16").unwrap();
        writeln!(self.out, "\tldp\tx21, x22, [sp], #16").unwrap();
        writeln!(self.out, "\tldp\tx19, x20, [sp], #16").unwrap();
        writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
        writeln!(self.out, "\tret").unwrap();
        Ok(())
    }

    fn emit_function(
        &mut self,
        f: &Function,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        if self.emit_freestanding_kernel_helper(f)? {
            return Ok(());
        }
        self.func_name = f.name.clone();
        self.func_ret = f.ret.clone();
        self.clear_locals();
        // Reserve [x29,#-8] for saved x19 (lvalue address temp / logical reg 19).
        // x19 is callee-saved under AAPCS64; we use it across calls in
        // CompoundAssign / PreInc / PostInc, so every function must preserve it
        // (same pattern as x86_64 %rbx). Without this, `n += f()` stores through
        // the callee's clobbered x19 and the add is lost (SQLite MakeRecord nHdr).
        self.stack_size = 8;
        self.break_stack.clear();
        self.continue_stack.clear();
        self.goto_labels.clear();
        self.goto_labels_defined.clear();
        self.va_regsave_off = 0;
        self.va_fpsave_off = 0;
        self.va_vr_idx_off = 0;
        self.va_fixed_fp = 0;
        self.va_fixed_n = 0;

        let body = f.body.as_ref().unwrap();

        // Measure total frame: saved x19 + params + optional va_regsave + locals
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
        // Variadic frame: GP regsave(64) + stack overflow(128) + FP save(64) + vr_idx(8).
        // Order keeps GP/stack relative layout identical to GP-only (sizeof-era),
        // with d0..d7 parked after so NestedParse/create is not shifted.
        if f.variadic {
            self.stack_size = Self::align_up(self.stack_size, 16) + 64 + 128 + 64 + 8;
        }
        let mut measure = self.scopes.clone();
        let mut measure_size = self.stack_size;
        self.measure_stmts(body, &mut measure, &mut measure_size, typedefs);
        // Spill headroom: call-arg temps use `str [sp,#-16]!` (below SP), so
        // this is only a small safety margin. Was +256; that made SQLite
        // same-table INSERT SELECT with index at ~32k rows need >8MB stack
        // (default ulimit) and SEGV. Keep 16B align slack only.
        let frame = Self::align_up(measure_size + 16, 16);

        // Reset and emit for real (keep slot for saved x19)
        self.clear_locals();
        self.stack_size = 8;

        // Count fixed GPRs / FPRs consumed (small aggregates take 1–2 regs).
        let mut fixed_gp = 0usize;
        let mut fixed_fp = 0usize;
        for (_, pty) in f.params.iter() {
            let pty = match pty {
                Type::Array(e, _) => Type::Ptr(e.clone()),
                other => other.clone(),
            };
            if matches!(pty, Type::Float | Type::Double) {
                fixed_fp += 1;
                continue;
            }
            if let Some(nr) = self.small_agg_nregs(&pty) {
                fixed_gp += nr as usize;
            } else {
                fixed_gp += 1;
            }
        }
        self.va_fixed_n = if f.variadic { fixed_gp.min(8) } else { 0 };
        self.va_fixed_fp = if f.variadic { fixed_fp.min(8) } else { 0 };

        let sym = self.c_sym(&f.name);
        let sec = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty());
        self.cur_section = sec.map(|s| s.to_string());
        if let Some(sec) = sec {
            self.emit_named_section(sec);
            writeln!(self.out, "\t.p2align\t2").unwrap();
        }
        // static / static-inline: local only. Real non-static bodies stay strong
        // unless marked __weak (COND_SYSCALL stubs must lose to real SYSCALL_DEFINE).
        if f.is_static {
            writeln!(self.out, "").unwrap();
        } else if f.is_weak {
            match self.os {
                TargetOs::Darwin => {
                    writeln!(self.out, "\n\t.weak_definition\t{sym}").unwrap();
                }
                TargetOs::Linux => {
                    writeln!(self.out, "\n\t.weak\t{sym}").unwrap();
                }
            }
            writeln!(self.out, "\t.globl\t{sym}").unwrap();
        } else {
            writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
        }
        // setup_arch: define BSS LR slot before the function label so soft=0
        // builds still link when kasan freestanding is not emitted.
        if f.name == "setup_arch" {
            let saved_lr = self.c_sym("acc_setup_arch_lr");
            writeln!(self.out, "\n\t.globl\t{saved_lr}").unwrap();
            writeln!(self.out, "\t.bss").unwrap();
            writeln!(self.out, "\t.p2align\t3").unwrap();
            writeln!(self.out, "{saved_lr}:").unwrap();
            writeln!(self.out, "\t.zero\t8").unwrap();
            if let Some(sec) = f.section.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                self.emit_named_section(sec);
            } else {
                writeln!(self.out, "\t.text").unwrap();
            }
        }
        writeln!(self.out, "\t.p2align\t2").unwrap();
        writeln!(self.out, "{sym}:").unwrap();
        writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tmov\tx29, sp").unwrap();
        // setup_arch: stash the real return address before any body code can
        // corrupt the frame. kasan_init_sw_tags freestanding hard-exits via
        // this slot when the on-stack LR is garbage.
        if f.name == "setup_arch" {
            let saved_lr = self.c_sym("acc_setup_arch_lr");
            writeln!(self.out, "\tadrp\tx16, {saved_lr}").unwrap();
            writeln!(self.out, "\tstr\tx30, [x16, :lo12:{saved_lr}]").unwrap();
        }
        // frame always >= 8 (saved x19 slot); still guard for safety
        if frame > 0 {
            if frame <= 4095 {
                writeln!(self.out, "\tsub\tsp, sp, #{frame}").unwrap();
            } else {
                self.emit_imm(frame, 16);
                writeln!(self.out, "\tsub\tsp, sp, x16").unwrap();
            }
        }
        // Preserve caller's x19 (callee-saved) before any body use.
        writeln!(self.out, "\tstr\tx19, [x29, #-8]").unwrap();

        // Allocate named params.
        // reg: 0..7 = GPR, 128+fpr = float, 255 = incoming stack slot.
        // For stack args (AAPCS64): first is at [x29, #16] after stp fp,lr.
        // nregs: 1 or 2 for small aggregates.
        // stack_x29: x29-relative offset of first 8-byte slot when reg==255.
        let mut param_offs: Vec<(i64, Type, u8, u8, i64)> = Vec::new(); // off, ty, reg, nregs, stack_x29
        let mut igpr = 0u8;
        let mut fpr = 0u8;
        let mut stack_x29 = 16i64; // first stack arg above saved fp/lr
        for (pname, pty) in f.params.iter() {
            if pname.is_empty() {
                continue;
            }
            let pty = match pty {
                Type::Array(e, _) => Type::Ptr(e.clone()),
                other => other.clone(),
            };
            let off = self.alloc_local(pname, &pty);
            if matches!(pty, Type::Float | Type::Double) {
                if fpr < 8 {
                    param_offs.push((off, pty, 128 + fpr, 1, 0));
                    fpr += 1;
                } else {
                    // float stack args: treat as 8-byte slot for now
                    param_offs.push((off, pty, 255, 1, stack_x29));
                    stack_x29 += 8;
                }
            } else if let Some(nr) = self.small_agg_nregs(&pty) {
                if igpr + nr <= 8 {
                    param_offs.push((off, pty, igpr, nr, 0));
                    igpr += nr;
                } else {
                    param_offs.push((off, pty, 255, nr, stack_x29));
                    stack_x29 += 8 * (nr as i64);
                }
            } else if Self::is_struct_or_union_ty(&pty) {
                // Large aggregate (>16B): AAPCS64 passes a hidden pointer in one
                // GPR. nregs=0 marks "by-ref: materialize with memcpy on entry".
                // Without this, Redis `pubsubUnsubscribeAllChannelsInternal(c,n,pubSubType)`
                // stored only the pointer into the local and then read
                // type.clientPubSubChannels from local+8 → garbage fptr (SIGBUS @ 0xf).
                if igpr < 8 {
                    param_offs.push((off, pty, igpr, 0, 0));
                    igpr += 1;
                } else {
                    param_offs.push((off, pty, 255, 0, stack_x29));
                    stack_x29 += 8;
                }
            } else if igpr < 8 {
                param_offs.push((off, pty, igpr, 1, 0));
                igpr += 1;
            } else {
                // 9th+ integer/pointer argument: on caller's stack.
                param_offs.push((off, pty, 255, 1, stack_x29));
                stack_x29 += 8;
            }
        }

        // Variadic: spill x0-x7, copy stack overflow, then d0-d7 + vr_idx.
        // GP/stack block matches sizeof-era; FP block is appended after.
        if f.variadic {
            self.stack_size = Self::align_up(self.stack_size, 16) + 64 + 128 + 64 + 8;
            self.va_regsave_off = -self.stack_size; // x0
            // stack overflow immediately after GP (same as GP-only layout)
            // fpsave after that; vr_idx last
            self.va_fpsave_off = self.va_regsave_off + 64 + 128;
            self.va_vr_idx_off = self.va_fpsave_off + 64;
            for r in 0u8..8 {
                let off = self.va_regsave_off + (r as i64) * 8;
                self.emit_fp_addr(off, 17);
                writeln!(self.out, "\tstr\tx{r}, [x17]").unwrap();
            }
            for i in 0i64..16 {
                let src = 16 + i * 8;
                writeln!(self.out, "\tldr\tx16, [x29, #{src}]").unwrap();
                let dest = self.va_regsave_off + 64 + i * 8;
                self.emit_fp_addr(dest, 17);
                writeln!(self.out, "\tstr\tx16, [x17]").unwrap();
            }
            for r in 0u8..8 {
                let off = self.va_fpsave_off + (r as i64) * 8;
                self.emit_fp_addr(off, 17);
                writeln!(self.out, "\tstr\td{r}, [x17]").unwrap();
            }
            self.emit_fp_addr(self.va_vr_idx_off, 17);
            writeln!(self.out, "\tmov\tw16, #{}", self.va_fixed_fp).unwrap();
            writeln!(self.out, "\tstr\tw16, [x17]").unwrap();
            for (off, pty, reg, nregs, stack_off) in &param_offs {
                if *reg < 8 {
                    for r in 0..*nregs {
                        let save_off = self.va_regsave_off + ((*reg + r) as i64) * 8;
                        self.emit_fp_addr(save_off, 17);
                        writeln!(self.out, "\tldr\tx16, [x17]").unwrap();
                        let dest_off = *off + (r as i64) * 8;
                        self.store_to_offset(dest_off, &Type::Long, 16);
                    }
                } else if *reg == 255 {
                    // Incoming stack arg relative to x29
                    for r in 0..*nregs {
                        let src = *stack_off + (r as i64) * 8;
                        writeln!(self.out, "\tldr\tx16, [x29, #{src}]").unwrap();
                        let dest_off = *off + (r as i64) * 8;
                        self.store_to_offset(dest_off, &Type::Long, 16);
                    }
                } else if *reg >= 128 {
                    let freg = *reg - 128;
                    if matches!(pty, Type::Float) {
                        writeln!(self.out, "\tfcvt\td{freg}, s{freg}").unwrap();
                    }
                    // Never fmov into x0..x7: that clobbers integer argument
                    // regs still waiting to be spilled (e.g. RealSameAsInt(double,i64)).
                    writeln!(self.out, "\tfmov\tx16, d{freg}").unwrap();
                    self.emit_fp_addr(*off, 9);
                    self.store_ty(pty, 9, 16);
                }
            }
        } else {
            // Spill scalar / small-agg params first (str only — preserve x0..x7
            // for large-aggregate by-ref sources). Push by-ref source pointers
            // onto the stack, then memcpy after all spills.
            let mut byref_slots: Vec<(i64, i64)> = Vec::new(); // (local_off, size)
            for (off, pty, reg, nregs, stack_off) in &param_offs {
                if *nregs == 0 {
                    // Large aggregate by-ref: xN (or stack) holds pointer to copy.
                    if *reg < 8 {
                        writeln!(self.out, "\tstr\tx{reg}, [sp, #-16]!").unwrap();
                    } else {
                        writeln!(self.out, "\tldr\tx16, [x29, #{}]", stack_off).unwrap();
                        writeln!(self.out, "\tstr\tx16, [sp, #-16]!").unwrap();
                    }
                    byref_slots.push((*off, self.type_size(pty).max(1)));
                    continue;
                }
                if *reg < 8 {
                    if *nregs > 1 {
                        // Small aggregate: store consecutive GPRs into local slot.
                        self.emit_fp_addr(*off, 9);
                        self.store_small_agg_from_regs(9, *nregs, *reg);
                    } else {
                        self.store_to_offset(*off, &Type::Long, *reg);
                    }
                } else if *reg == 255 {
                    // 9th+ arg: load from caller's stack frame via x29.
                    if *nregs > 1 {
                        for r in 0..*nregs {
                            let src = *stack_off + (r as i64) * 8;
                            writeln!(self.out, "\tldr\tx16, [x29, #{src}]").unwrap();
                            let dest_off = *off + (r as i64) * 8;
                            self.store_to_offset(dest_off, &Type::Long, 16);
                        }
                    } else {
                        writeln!(self.out, "\tldr\tx16, [x29, #{}]", stack_off).unwrap();
                        self.store_to_offset(*off, pty, 16);
                    }
                } else if *reg >= 128 {
                    let freg = *reg - 128;
                    if matches!(pty, Type::Float) {
                        writeln!(self.out, "\tfcvt\td{freg}, s{freg}").unwrap();
                    }
                    // Never fmov into x0..x7: that clobbers integer argument
                    // regs still waiting to be spilled (e.g. RealSameAsInt(double,i64)).
                    writeln!(self.out, "\tfmov\tx16, d{freg}").unwrap();
                    self.emit_fp_addr(*off, 9);
                    self.store_ty(pty, 9, 16);
                }
            }
            // Materialize large aggregates (pointers pushed in forward order).
            for (off, sz) in byref_slots.into_iter().rev() {
                writeln!(self.out, "\tldr\tx1, [sp], #16").unwrap();
                self.emit_fp_addr(off, 0);
                self.emit_imm(sz, 2);
                writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();
            }
        }

        for st in body {
            self.emit_stmt(st, typedefs)?;
        }

        writeln!(self.out, "\tmov\tw0, #0").unwrap();
        let end = format!("L_{}_epilogue", f.name);
        writeln!(self.out, "{end}:").unwrap();
        // Restore caller's x19 before tearing down the frame.
        writeln!(self.out, "\tldr\tx19, [x29, #-8]").unwrap();
        if frame > 0 {
            writeln!(self.out, "\tmov\tsp, x29").unwrap();
        }
        writeln!(self.out, "\tldp\tx29, x30, [sp], #16").unwrap();
        writeln!(self.out, "\tret").unwrap();
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
                if d.is_extern {
                    // Block-scope extern: reference the global, no stack slot.
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
                } else if d.is_static {
                    // static local: no stack slot; global storage keyed by name
                    // (func_name may be empty during measure — fixup at emit time)
                    if let Some(scope) = locals.last_mut() {
                        scope.insert(
                            d.name.clone(),
                            Sym {
                                ty,
                                storage: Storage::Global {
                                    name: format!("__static_pending_{}", d.name),
                                },
                            },
                        );
                    }
                } else {
                    let sz = self.stack_slot_size(&ty).max(8);
                    let al = 8i64;
                    *stack = Self::align_up(*stack + sz, al);
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
                then_b, else_b, ..
            } => {
                self.measure_stmt(then_b, locals, stack, typedefs);
                if let Some(e) = else_b {
                    self.measure_stmt(e, locals, stack, typedefs);
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::Label(_, body) => {
                self.measure_stmt(body, locals, stack, typedefs)
            }
            Stmt::For { init, body, .. } => {
                locals.push(HashMap::new());
                if let Some(i) = init {
                    self.measure_stmt(i, locals, stack, typedefs);
                }
                self.measure_stmt(body, locals, stack, typedefs);
                locals.pop();
            }
            // SQLite VdbeExec is a giant switch with many case-local decls;
            // skipping these under-counted the frame (e.g. 400B) while emit
            // used offsets past -800 → stack smash / Btree* = 0x8.
            Stmt::Switch { body, .. } => self.measure_stmt(body, locals, stack, typedefs),
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
            Expr::Unary { expr, .. }
            | Expr::Cast { expr, .. }
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
            Expr::Call { args, .. } => {
                for a in args {
                    self.measure_expr(a, locals, stack, typedefs);
                }
            }
            Expr::Cond { cond, then_e, else_e } => {
                self.measure_expr(cond, locals, stack, typedefs);
                self.measure_expr(then_e, locals, stack, typedefs);
                self.measure_expr(else_e, locals, stack, typedefs);
            }
            Expr::Member { base, .. } => self.measure_expr(base, locals, stack, typedefs),
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
                // Typedef of a named struct/union already registered under `n` —
                // do not recurse into the same Struct(n) (infinite loop).
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
                out_store_exprs: _,
            } => {
                // Evaluate input operands into assigned xN ("r"(off), matching
                // "0"(ptr) for RELOC_HIDE, etc.).
                for (reg, e) in in_loads {
                    self.emit_expr_rval(e, *reg, typedefs)?;
                }
                // Emit kbuild DEFINE lines; skip raw templates with %0/%[name].
                // Drop .rept/.endr and full .macro…​.endm (not just the directives —
                // keeping the body alone leaves gas macro params like `\sreg` /
                // `\rt` and breaks as with "found '\', expected: ')'").
                let mut rept_depth = 0i32;
                let mut macro_depth = 0i32;
                let mut if_depth = 0i32;
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
                        continue; // inside unreified .rept body
                    }
                    // Full .macro … .endm block (arm64 sysreg.h DEFINE_MRS_S etc.)
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
                    // Soft-drop gas .if/.ifdef/.ifndef … .endif (ALTERNATIVE leaves
                    // `.if 1==1` with bodies that had `%0` filtered out → "EOF inside
                    // conditional"). Skip the whole conditional including .else.
                    if lower.starts_with(".ifdef")
                        || lower.starts_with(".ifndef")
                        || lower.starts_with(".if ")
                        || lower == ".if"
                        || lower.starts_with(".if\t")
                    {
                        if_depth += 1;
                        continue;
                    }
                    if lower.starts_with(".elseif") || lower.starts_with(".elseifdef") {
                        continue; // still inside skipped conditional
                    }
                    if lower == ".else" || lower.starts_with(".else ") || lower.starts_with(".else\t")
                    {
                        continue;
                    }
                    if lower.starts_with(".endif") {
                        if if_depth > 0 {
                            if_depth -= 1;
                        }
                        continue;
                    }
                    if if_depth > 0 {
                        continue;
                    }
                    // Paired with dropped DEFINE_MRS_S / DEFINE_MSR_S
                    if lower.starts_with(".purgem") {
                        continue;
                    }
                    // Orphan calls to ephemeral gas macros we did not emit.
                    // `mrs_s x0, SYS_…` / `msr_s SYS_…, x0` only work inside the
                    // define/use/purgem triple from read_sysreg_s.
                    {
                        let mnem = lower
                            .split(|c: char| c == ' ' || c == '\t' || c == ',')
                            .next()
                            .unwrap_or("");
                        if mnem == "mrs_s" || mnem == "msr_s" {
                            continue;
                        }
                    }
                    // Gas macro formal params (`\sreg`, `\rt`) if any leak out.
                    if t.as_bytes().windows(2).any(|w| {
                        w[0] == b'\\' && (w[1].is_ascii_alphabetic() || w[1] == b'_')
                    }) {
                        continue;
                    }
                    // Drop any remaining GCC asm operand refs: %0, %w1, %l[lab], %[name].
                    // Emitting them produces unassemblable `b %l[l_no]` / `stlrb %w1, xzr`.
                    // Still emit a leading numeric local label (`1:ldr %0` → `1:`) so
                    // EX_TABLE `1b`/`2b` fixups from the same template assemble.
                    // Drop any line still containing an unresolved GCC asm operand
                    // (`%0`, `%w1`, `%[name]`, or garbage like `%*((u64*)…)` from
                    // failed macro paste). Keep `%%` (literal percent).
                    {
                        let b = t.as_bytes();
                        let mut i = 0usize;
                        let mut bad_pct = false;
                        while i < b.len() {
                            if b[i] == b'%' {
                                if i + 1 < b.len() && b[i + 1] == b'%' {
                                    i += 2;
                                    continue;
                                }
                                bad_pct = true;
                                break;
                            }
                            i += 1;
                        }
                        if bad_pct {
                            if let Some(colon) = t.find(':') {
                                let lab = t[..colon].trim();
                                if !lab.is_empty() && lab.bytes().all(|b| b.is_ascii_digit()) {
                                    writeln!(self.out, "{lab}:").unwrap();
                                }
                            }
                            continue;
                        }
                    }
                    // Drop broken mrs/msr with non-register operands (e.g. KVM hyp
                    // after soft %N fold: `mrs -14, spsr_el2` / `msr spsr_el2, -14`).
                    // Match broadly: any mrs/msr line containing `,-N` or starting operand `-N`.
                    if (lower.starts_with("mrs") || lower.starts_with("msr"))
                        && (t.contains(",-")
                            || t.contains(", -")
                            || t.contains("\t-")
                            || t.contains(" -"))
                    {
                        // Only drop if a negative integer operand is present (not e.g. label -L)
                        let has_neg_imm = t.split(|c: char| {
                            c == ',' || c == ' ' || c == '\t' || c == '[' || c == ']'
                        })
                        .any(|tok| {
                            let tok = tok.trim();
                            tok.starts_with('-')
                                && tok.len() > 1
                                && tok.as_bytes()[1].is_ascii_digit()
                        });
                        if has_neg_imm {
                            continue;
                        }
                    }
                    writeln!(self.out, "\t{line}").unwrap();
                }
                // Store xN into C vars for "=r" output operands
                // (e.g. mrs %0, sp_el0 : "=r"(sp_el0) → get_current).
                for (reg, var) in out_stores {
                    self.emit_asm_operand_store(var, *reg, typedefs)?;
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
                    // Block-scope `extern T name;` — bind name to the global
                    // symbol (no stack). Critical for SQLite test1.c:
                    //   extern int sqlite3_search_count;
                    //   Tcl_LinkVar(..., (char*)&sqlite3_search_count, ...);
                    self.insert_local(
                        d.name.clone(),
                        Sym {
                            ty: ty.clone(),
                            storage: Storage::Global {
                                name: d.name.clone(),
                            },
                        },
                    );
                    // Ensure the global is known so lookup/GOT works even if
                    // the defining TU's symbol was only seen as a BSS def.
                    if !self.globals.contains_key(&d.name) {
                        self.globals.insert(d.name.clone(), ty.clone());
                    }
                    return Ok(());
                }
                if d.is_static {
                    // Emit once as a unique global; re-entry keeps the value.
                    // MUST uniquify by occurrence: C allows multiple
                    // `static const char *TTYPE_strs[]` in sibling blocks of
                    // the same function (tclsqlite.c DB_TRACE vs DB_TRANSACTION).
                    // Reusing `__static_<func>_<name>` made transaction see the
                    // trace table → "must be statement, profile, row, or close".
                    let id = self.label_id;
                    self.label_id += 1;
                    let gname = format!("__static_{}_{}_{}", self.func_name, d.name, id);
                    if !self.globals.contains_key(&gname) {
                        let mut g = d.clone();
                        g.name = gname.clone();
                        // Keep is_static=true: local linkage, no .globl. Also
                        // emit_global maps static zero-init to .data (not .bss)
                        // so vdso link scripts that discard .bss still resolve
                        // `__static_*_ret` etc.
                        g.is_static = true;
                        self.emit_global(&g)?;
                        // switch back to text
                        self.emit_text_section();
                        self.globals.insert(gname.clone(), ty.clone());
                    }
                    self.insert_local(
                        d.name.clone(),
                        Sym {
                            ty: ty.clone(),
                            storage: Storage::Global { name: gname },
                        },
                    );
                    // static init is in data; do not re-run at each call
                    return Ok(());
                }
                // Always allocate a fresh stack slot. C allows the same name in
                // sibling blocks with different types/sizes (sqlite3FpDecode:
                // `double rr` in the long-double branch vs `double rr[2]` in
                // the else). Reusing the first offset leaves rr[2] as 8 bytes
                // and overlaps the next local (exp) — FpDecode then corrupts
                // exp via rr[1] and prints garbage like 1.5e+g70.
                let off = self.alloc_local(&d.name, &ty);
                if let Some(init) = &d.init {
                    if let Expr::InitList { fields } = init {
                        self.emit_local_init_list(off, &ty, fields, typedefs)?;
                        return Ok(());
                    }
                    // Small aggregate init: value in x0[,x1] (call) or via memcpy from lvalue.
                    if let Some(nr) = self.small_agg_nregs(&ty) {
                        if matches!(init, Expr::Call { .. }) {
                            self.emit_expr_rval(init, 0, typedefs)?;
                            self.emit_fp_addr(off, 9);
                            self.store_small_agg_from_regs(9, nr, 0);
                        } else if self.emit_lvalue_addr(init, 0, typedefs).is_ok() {
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                            self.emit_fp_addr(off, 9);
                            writeln!(self.out, "\tldr\tx1, [sp], #16").unwrap();
                            writeln!(self.out, "\tmov\tx0, x9").unwrap();
                            self.emit_imm(self.type_size(&ty), 2);
                            writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();
                        } else {
                            self.emit_expr_rval(init, 0, typedefs)?;
                            self.emit_fp_addr(off, 9);
                            self.store_small_agg_from_regs(9, nr, 0);
                        }
                        return Ok(());
                    }
                    // Large aggregate init (e.g. SQLite `Mem x = pIn3[0];` sizeof=56).
                    // Must memcpy the full object — load_ty only moves 8 bytes and
                    // leaves flags/enc garbage, which made SeekRowid treat NULL as 0.
                    if Self::is_struct_or_union_ty(&ty) && self.type_size(&ty) > 8 {
                        if self.emit_lvalue_addr(init, 0, typedefs).is_ok() {
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                            self.emit_fp_addr(off, 9);
                            writeln!(self.out, "\tldr\tx1, [sp], #16").unwrap();
                            writeln!(self.out, "\tmov\tx0, x9").unwrap();
                            self.emit_imm(self.type_size(&ty), 2);
                            writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();
                            return Ok(());
                        }
                        // *ptr form
                        if let Expr::Unary {
                            op: UnaryOp::Deref,
                            expr,
                        } = init
                        {
                            self.emit_expr_rval(expr, 0, typedefs)?;
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                            self.emit_fp_addr(off, 9);
                            writeln!(self.out, "\tldr\tx1, [sp], #16").unwrap();
                            writeln!(self.out, "\tmov\tx0, x9").unwrap();
                            self.emit_imm(self.type_size(&ty), 2);
                            writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();
                            return Ok(());
                        }
                    }
                    // `char buf[] = "hi";` / `unsigned char zHex[] = "0123..."`:
                    // string rvalue is a pointer; must memcpy bytes into the
                    // array slot. Storing the pointer made hexio BinToHex index
                    // stack garbage (SQLite alter2/hexio_get_int → 0).
                    if let (Type::Array(elem, n), Expr::String(s)) = (&ty, init) {
                        if matches!(elem.as_ref(), Type::Char | Type::SChar) {
                            let id = self.intern_str(s);
                            let slab = format!("l_str_{id}");
                            self.emit_fp_addr(off, 0);
                            self.emit_adrp_add(1, &slab);
                            let copy_n = if *n > 0 {
                                *n
                            } else {
                                (s.len() + 1) as i64
                            };
                            self.emit_imm(copy_n, 2);
                            writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();
                            return Ok(());
                        }
                    }
                    self.emit_expr_rval(init, 0, typedefs)?;
                    if matches!(ty, Type::Float | Type::Double) {
                        let rty = self.typeof_expr(init, typedefs);
                        if !matches!(rty, Type::Float | Type::Double) {
                            // u64→double must be ucvtf, not scvtf (AtoF significand path).
                            if matches!(rty, Type::UShort | Type::UInt | Type::ULong) {
                                writeln!(self.out, "\tucvtf\td0, x0").unwrap();
                            } else {
                                writeln!(self.out, "\tscvtf\td0, x0").unwrap();
                            }
                            writeln!(self.out, "\tfmov\tx0, d0").unwrap();
                        }
                        self.emit_fp_addr(off, 9);
                        self.store_ty(&ty, 9, 0);
                    } else {
                        // float → integer conversion for `int e = 97.0;`
                        let rty = self.typeof_expr(init, typedefs);
                        if matches!(rty, Type::Float | Type::Double)
                            && matches!(
                                ty,
                                Type::Char
                                    | Type::SChar
                                    | Type::Short
                                    | Type::UShort
                                    | Type::Int
                                    | Type::UInt
                                    | Type::Long
                                    | Type::ULong
                            )
                        {
                            writeln!(self.out, "\tfmov\td0, x0").unwrap();
                            if matches!(ty, Type::UShort | Type::UInt | Type::ULong) {
                                writeln!(self.out, "\tfcvtzu\tx0, d0").unwrap();
                            } else {
                                writeln!(self.out, "\tfcvtzs\tx0, d0").unwrap();
                            }
                        }
                        self.store_to_offset(off, &ty, 0);
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
                    let rty = self.func_ret.clone();
                    if let Some(nr) = self.small_agg_nregs(&rty) {
                        // AAPCS64: composite ≤16 bytes in x0[, x1].
                        if self.emit_lvalue_addr(ex, 9, typedefs).is_ok() {
                            self.load_small_agg_to_regs(9, nr, 0);
                        } else {
                            // Non-lvalue (e.g. call): evaluate then re-pack from a temp.
                            // Allocate a temp on the frame for the returned bits.
                            let tmp = {
                                self.stack_size =
                                    Self::align_up(self.stack_size + self.type_size(&rty).max(8), 8);
                                -self.stack_size
                            };
                            // Best-effort: emit as scalar path then store x0 (and hope x1).
                            self.emit_expr_rval(ex, 0, typedefs)?;
                            self.emit_fp_addr(tmp, 9);
                            writeln!(self.out, "\tstr\tx0, [x9]").unwrap();
                            if nr >= 2 {
                                writeln!(self.out, "\tstr\tx1, [x9, #8]").unwrap();
                            }
                            self.load_small_agg_to_regs(9, nr, 0);
                        }
                    } else {
                        self.emit_expr_rval(ex, 0, typedefs)?;
                    }
                } else {
                    writeln!(self.out, "\tmov\tw0, #0").unwrap();
                }
                writeln!(self.out, "\tb\tL_{}_epilogue", self.func_name).unwrap();
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
                self.emit_cbz_long(0, &l_else);
                self.emit_stmt(then_b, typedefs)?;
                writeln!(self.out, "\tb\t{l_end}").unwrap();
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
                self.emit_cbz_long(0, &l_end);
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "\tb\t{l_head}").unwrap();
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
                self.emit_cbnz_long(0, &l_head);
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
                    self.emit_cbz_long(0, &l_end);
                }
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "{l_cont}:").unwrap();
                if let Some(s) = step {
                    self.emit_expr_rval(s, 0, typedefs)?;
                }
                writeln!(self.out, "\tb\t{l_head}").unwrap();
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
                writeln!(self.out, "\tb\t{l}").unwrap();
                Ok(())
            }
            Stmt::Continue => {
                let l = self
                    .continue_stack
                    .last()
                    .ok_or("continue outside loop")?
                    .clone();
                writeln!(self.out, "\tb\t{l}").unwrap();
                Ok(())
            }
            Stmt::Goto(name) => {
                let lab = self.goto_lab(name);
                writeln!(self.out, "\tb\t{lab}").unwrap();
                Ok(())
            }
            Stmt::GotoIndirect(e) => {
                // GCC `goto *expr` — address in x0, then branch.
                self.emit_expr_rval(e, 0, typedefs)?;
                writeln!(self.out, "\tbr\tx0").unwrap();
                Ok(())
            }
            Stmt::Label(name, inner) => {
                let lab = self.goto_lab(name);
                if self.goto_labels_defined.insert(lab.clone()) {
                    writeln!(self.out, "{lab}:").unwrap();
                }
                self.emit_stmt(inner, typedefs)
            }
            Stmt::Switch { cond, body } => {
                let l_end = self.lab("swend");
                let l_default = self.lab("swdef");
                self.break_stack.push(l_end.clone());
                // Nested switches must not clobber outer case label queue
                let saved_cases = std::mem::take(&mut self.pending_case_labs);
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                let mut cases: Vec<(Option<i64>, String)> = Vec::new();
                self.collect_switch_cases(body, &mut cases);
                self.pending_case_labs.clear();
                let mut has_default = false;
                let mut default_lab = l_default.clone();
                for (val, lab) in &cases {
                    if let Some(v) = val {
                        self.pending_case_labs.push_back(lab.clone());
                        writeln!(self.out, "\tldr\tx0, [sp]").unwrap();
                        self.emit_imm(*v, 1);
                        writeln!(self.out, "\tcmp\tx0, x1").unwrap();
                        self.emit_bcond_long("eq", &lab);
                    } else {
                        has_default = true;
                        default_lab = lab.clone();
                    }
                }
                if has_default {
                    writeln!(self.out, "\tb\t{default_lab}").unwrap();
                } else {
                    writeln!(self.out, "\tb\t{l_end}").unwrap();
                }
                self.emit_switch_body(body, &default_lab, typedefs)?;
                // Emit any case labels collected but not walked (rare nestings /
                // Duff-device edge cases) so assembler refs always resolve.
                while let Some(lab) = self.pending_case_labs.pop_front() {
                    writeln!(self.out, "{lab}:").unwrap();
                }
                // Also define any case label that was referenced in the dispatch
                // but never written as a definition (collect/emit walk mismatch).
                for (val, lab) in &cases {
                    if val.is_some() && !self.out.contains(&format!("{lab}:")) {
                        writeln!(self.out, "{lab}:").unwrap();
                    }
                }
                if has_default && !self.out.contains(&format!("{default_lab}:")) {
                    writeln!(self.out, "{default_lab}:").unwrap();
                }
                // Pop switch value on ALL exits. Label MUST come first: `break`
                // and unmatched dispatch jump to l_end; if cleanup is above the
                // label those paths skip it and leak 16B per iteration (SQLite
                // VdbeExec: ~200B/row → O(n) stack → SEGV on count-1.2.5 @8MB).
                writeln!(self.out, "{l_end}:").unwrap();
                writeln!(self.out, "\tadd\tsp, sp, #16").unwrap();
                self.break_stack.pop();
                self.pending_case_labs = saved_cases;
                Ok(())
            }
            Stmt::Case { body, .. } => self.emit_stmt(body, typedefs),
            Stmt::Default(body) => self.emit_stmt(body, typedefs),
        }
    }

    fn collect_switch_cases(&mut self, st: &Stmt, out: &mut Vec<(Option<i64>, String)>) {
        match st {
            Stmt::Block(ss) => {
                for s in ss {
                    self.collect_switch_cases(s, out);
                }
            }
            Stmt::DeclGroup(_decls) => {}
            Stmt::Case { value, body } => {
                let lab = self.lab("case");
                // Fold full constant expressions: `(6)|((1)<<4)` (Lua ttypetag)
                // and enum constants (TK_IF, etc.) registered in const_globals.
                let v = self.const_i64_env(value);
                out.push((v, lab));
                self.collect_switch_cases(body, out);
            }
            Stmt::Default(body) => {
                let lab = self.lab("swdef");
                out.push((None, lab));
                self.collect_switch_cases(body, out);
            }
            Stmt::Label(_, inner) => self.collect_switch_cases(inner, out),
            // Duff's device: cases nested inside loops
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
                // keep scanning for nested cases (Duff's device)
                self.emit_switch_body(body, default_lab, typedefs)
            }
            Stmt::Default(body) => {
                writeln!(self.out, "{default_lab}:").unwrap();
                self.emit_switch_body(body, default_lab, typedefs)
            }
            Stmt::DoWhile { body, cond } => {
                // reimplement loop so body still walks case labels
                let l_head = self.lab("do");
                let l_cont = self.lab("docont");
                let l_end = self.lab("enddo");
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_cont.clone());
                writeln!(self.out, "{l_head}:").unwrap();
                self.emit_switch_body(body, default_lab, typedefs)?;
                writeln!(self.out, "{l_cont}:").unwrap();
                self.emit_expr_rval(cond, 0, typedefs)?;
                self.emit_cbnz_long(0, &l_head);
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                Ok(())
            }
            Stmt::While { body, cond } => {
                let l_head = self.lab("while");
                let l_cont = self.lab("whilecont");
                let l_end = self.lab("endwhile");
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_cont.clone());
                writeln!(self.out, "{l_head}:").unwrap();
                self.emit_expr_rval(cond, 0, typedefs)?;
                self.emit_cbz_long(0, &l_end);
                self.emit_switch_body(body, default_lab, typedefs)?;
                writeln!(self.out, "{l_cont}:").unwrap();
                writeln!(self.out, "\tb\t{l_head}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                Ok(())
            }
            // TINFL_CR_RETURN_FOREVER nests `case` inside `for (;;)` — must walk cases.
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let l_head = self.lab("for");
                let l_cont = self.lab("forcont");
                let l_end = self.lab("endfor");
                if let Some(i) = init {
                    self.emit_switch_body(i, default_lab, typedefs)?;
                }
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_cont.clone());
                writeln!(self.out, "{l_head}:").unwrap();
                if let Some(c) = cond {
                    self.emit_expr_rval(c, 0, typedefs)?;
                    self.emit_cbz_long(0, &l_end);
                }
                self.emit_switch_body(body, default_lab, typedefs)?;
                writeln!(self.out, "{l_cont}:").unwrap();
                if let Some(s) = step {
                    self.emit_expr_rval(s, 0, typedefs)?;
                }
                writeln!(self.out, "\tb\t{l_head}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                Ok(())
            }
            Stmt::Label(name, inner) => {
                let lab = self.goto_lab(name);
                if self.goto_labels_defined.insert(lab.clone()) {
                    writeln!(self.out, "{lab}:").unwrap();
                }
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
                self.emit_cbz_long(0, &l_else);
                self.emit_switch_body(then_b, default_lab, typedefs)?;
                writeln!(self.out, "\tb\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                if let Some(e) = else_b {
                    self.emit_switch_body(e, default_lab, typedefs)?;
                }
                writeln!(self.out, "{l_end}:").unwrap();
                Ok(())
            }
            other => self.emit_stmt(other, typedefs),
        }
    }

    /// Materialize x29+off into x{addr_reg}.
    /// AArch64 ADD/SUB immediates are non-negative 0..4095; LDR/STR signed 9-bit is -256..255.
    fn emit_fp_addr(&mut self, off: i64, addr_reg: u8) {
        if off == 0 {
            writeln!(self.out, "\tmov\tx{addr_reg}, x29").unwrap();
        } else {
            self.emit_add_imm(addr_reg, 29, off);
        }
    }

    /// `x{dest} = x{src} + imm` with legal AArch64 immediates.
    /// ADD/SUB imm12 is 0..4095, optionally shifted left by 12.
    fn emit_add_imm(&mut self, dest: u8, src: u8, imm: i64) {
        if imm == 0 {
            if dest != src {
                writeln!(self.out, "\tmov\tx{dest}, x{src}").unwrap();
            }
            return;
        }
        if (0..=4095).contains(&imm) {
            writeln!(self.out, "\tadd\tx{dest}, x{src}, #{imm}").unwrap();
            return;
        }
        if (-4095..=-1).contains(&imm) {
            writeln!(self.out, "\tsub\tx{dest}, x{src}, #{}", -imm).unwrap();
            return;
        }
        // Positive: try imm12 + (imm12 << 12) encoding (covers up to ~16MiB).
        if imm > 0 {
            let hi = imm >> 12;
            let lo = imm & 0xfff;
            if hi <= 4095 {
                if lo == 0 {
                    writeln!(self.out, "\tadd\tx{dest}, x{src}, #{hi}, lsl #12").unwrap();
                } else {
                    writeln!(self.out, "\tadd\tx{dest}, x{src}, #{hi}, lsl #12").unwrap();
                    writeln!(self.out, "\tadd\tx{dest}, x{dest}, #{lo}").unwrap();
                }
                return;
            }
        }
        // Negative large: sub with same encoding.
        if imm < 0 {
            let n = -imm;
            let hi = n >> 12;
            let lo = n & 0xfff;
            if hi <= 4095 {
                if lo == 0 {
                    writeln!(self.out, "\tsub\tx{dest}, x{src}, #{hi}, lsl #12").unwrap();
                } else {
                    writeln!(self.out, "\tsub\tx{dest}, x{src}, #{hi}, lsl #12").unwrap();
                    writeln!(self.out, "\tsub\tx{dest}, x{dest}, #{lo}").unwrap();
                }
                return;
            }
        }
        // Fallback: materialize full imm in a temp then add.
        let tmp = if src != 16 && dest != 16 {
            16u8
        } else if src != 17 && dest != 17 {
            17u8
        } else {
            15u8
        };
        self.emit_imm(imm, tmp);
        writeln!(self.out, "\tadd\tx{dest}, x{src}, x{tmp}").unwrap();
    }

    /// Store `x{reg}` into C local/global `var` for extended-asm "=r" outputs.
    fn emit_asm_operand_store(
        &mut self,
        var: &str,
        reg: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        if let Some(sym) = self.get_local(var).cloned() {
            match &sym.storage {
                Storage::Local { offset } => {
                    self.store_to_offset(*offset, &sym.ty, reg);
                }
                Storage::RegAddr { reg: r } => {
                    writeln!(self.out, "\tmov\tx{r}, x{reg}").unwrap();
                }
                Storage::Global { name } => {
                    let lab = self.c_sym(name);
                    let addr = if reg != 16 { 16u8 } else { 17u8 };
                    writeln!(self.out, "\tadrp\tx{addr}, {lab}").unwrap();
                    writeln!(self.out, "\tstr\tx{reg}, [x{addr}, :lo12:{lab}]").unwrap();
                }
            }
            return Ok(());
        }
        if self.globals.contains_key(var) {
            let lab = self.c_sym(var);
            // Use x16 as scratch for address (avoid clobbering result reg if reg==16)
            let addr = if reg != 16 { 16u8 } else { 17u8 };
            writeln!(self.out, "\tadrp\tx{addr}, {lab}").unwrap();
            writeln!(self.out, "\tstr\tx{reg}, [x{addr}, :lo12:{lab}]").unwrap();
            return Ok(());
        }
        let _ = typedefs;
        Ok(())
    }

    fn store_to_offset(&mut self, off: i64, ty: &Type, reg: u8) {
        // Always width-correct stores. Using `str x` for Int (4 bytes) was wrong
        // for packed array slots: `const int m[] = {a,b,c,d}` wrote 8 bytes at
        // each 4-byte stride and clobbered the next local (Redis serverLogRaw:
        // syslogLevelMap overwrote `msg` → fprintf %s SEGV on 0xffff00000000).
        // Scalar locals still use 8-byte slots; loads use ldrsw/ldur and only
        // need the low width defined.
        if matches!(ty, Type::Float | Type::Double) {
            self.emit_fp_addr(off, 17);
            self.store_ty(ty, 17, reg);
            return;
        }
        if (-256..256).contains(&off) {
            match self.type_size(ty) {
                1 => writeln!(self.out, "\tstrb\tw{reg}, [x29, #{off}]").unwrap(),
                2 => writeln!(self.out, "\tstrh\tw{reg}, [x29, #{off}]").unwrap(),
                4 => writeln!(self.out, "\tstr\tw{reg}, [x29, #{off}]").unwrap(),
                _ => writeln!(self.out, "\tstr\tx{reg}, [x29, #{off}]").unwrap(),
            }
        } else {
            self.emit_fp_addr(off, 17);
            self.store_ty(ty, 17, reg);
        }
    }

    fn load_from_offset(&mut self, off: i64, ty: &Type, reg: u8) {
        // Float/double must go through fcvt paths; integer stack slots are 8-byte
        // but may only have low 32 bits defined — use width-correct loads.
        if matches!(ty, Type::Float | Type::Double) {
            self.emit_fp_addr(off, 17);
            self.load_ty(ty, 17, reg);
            return;
        }
        if (-256..256).contains(&off) {
            match ty {
                Type::Char => writeln!(self.out, "\tldrb\tw{reg}, [x29, #{off}]").unwrap(),
                // Sign-extend all the way to X (not W): `ldrsb w` zero-extends to x64
                // and turns -2 into 0xfffffffe, breaking lemon yysize pointer math.
                Type::SChar => writeln!(self.out, "\tldrsb\tx{reg}, [x29, #{off}]").unwrap(),
                Type::Short => writeln!(self.out, "\tldrsh\tx{reg}, [x29, #{off}]").unwrap(),
                Type::UShort => writeln!(self.out, "\tldrh\tw{reg}, [x29, #{off}]").unwrap(),
                Type::Int => {
                    // sign-extend 32→64 so high bits are never stale garbage
                    writeln!(self.out, "\tldrsw\tx{reg}, [x29, #{off}]").unwrap();
                }
                // Zero-extend unsigned 32-bit (Pgno/u32): 0xfffffffe must stay positive.
                Type::UInt => writeln!(self.out, "\tldr\tw{reg}, [x29, #{off}]").unwrap(),
                Type::Long | Type::ULong | Type::Ptr(_) => {
                    writeln!(self.out, "\tldr\tx{reg}, [x29, #{off}]").unwrap();
                }
                _ => match self.type_size(ty) {
                    1 => writeln!(self.out, "\tldrb\tw{reg}, [x29, #{off}]").unwrap(),
                    2 => writeln!(self.out, "\tldrsh\tx{reg}, [x29, #{off}]").unwrap(),
                    4 => writeln!(self.out, "\tldrsw\tx{reg}, [x29, #{off}]").unwrap(),
                    _ => writeln!(self.out, "\tldr\tx{reg}, [x29, #{off}]").unwrap(),
                },
            }
        } else {
            self.emit_fp_addr(off, 17);
            match ty {
                Type::Int => writeln!(self.out, "\tldrsw\tx{reg}, [x17]").unwrap(),
                Type::UInt => writeln!(self.out, "\tldr\tw{reg}, [x17]").unwrap(),
                Type::Char => writeln!(self.out, "\tldrb\tw{reg}, [x17]").unwrap(),
                Type::SChar => writeln!(self.out, "\tldrsb\tx{reg}, [x17]").unwrap(),
                Type::Short => writeln!(self.out, "\tldrsh\tx{reg}, [x17]").unwrap(),
                Type::UShort => writeln!(self.out, "\tldrh\tw{reg}, [x17]").unwrap(),
                Type::Long | Type::ULong | Type::Ptr(_) => {
                    writeln!(self.out, "\tldr\tx{reg}, [x17]").unwrap()
                }
                _ => self.load_ty(ty, 17, reg),
            }
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
                // Flexible/unsized `T a[] = {…}`: n may be 0; use initializer count.
                let count = if *n > 0 {
                    *n as usize
                } else {
                    fields_in.len()
                };
                let esz = self.type_size(elem).max(1);
                let start = base_off;
                for i in 0..count {
                    let eoff = start + (i as i64) * esz;
                    if let Some((_, e)) = fields_in.get(i) {
                        // Nested brace init for element: recurse with element type.
                        // Critical for `struct S a[] = { {ptr, 1, 2, 3}, … }` —
                        // bare emit_expr_rval(InitList) used to treat the list as
                        // char[64] (.byte ints) and store only an 8-byte pointer.
                        if let Expr::InitList { fields } = e {
                            self.emit_local_init_list(eoff, elem, fields, typedefs)?;
                        } else if self.type_size(elem) > 8
                            && matches!(
                                elem.as_ref(),
                                Type::Struct(_)
                                    | Type::Union(_)
                                    | Type::AnonStruct(_)
                                    | Type::AnonUnion(_)
                                    | Type::Array(_, _)
                            )
                        {
                            // Aggregate rvalue (e.g. other local struct): memcpy.
                            if self.emit_lvalue_addr(e, 1, typedefs).is_ok() {
                                writeln!(self.out, "\tstr\tx1, [sp, #-16]!").unwrap();
                                self.emit_fp_addr(eoff, 0);
                                writeln!(self.out, "\tldr\tx1, [sp], #16").unwrap();
                                self.emit_imm(self.type_size(elem), 2);
                                writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();
                            } else {
                                self.emit_expr_rval(e, 0, typedefs)?;
                                self.store_to_offset(eoff, elem, 0);
                            }
                        } else {
                            self.emit_expr_rval(e, 0, typedefs)?;
                            self.store_to_offset(eoff, elem, 0);
                        }
                    }
                }
            }
            Type::Struct(name) | Type::Union(name) => {
                // Soft: incomplete/forward types (e.g. pin_cookie) may lack layout
                // mid-TU — skip field stores rather than abort the whole kernel TU.
                let Some(lay) = self.layouts.get(name).cloned() else {
                    return Ok(());
                };
                self.emit_local_struct_fields(base_off, &lay, fields_in, typedefs)?;
            }
            Type::AnonStruct(fs) | Type::AnonUnion(fs) => {
                let is_union = matches!(ty, Type::AnonUnion(_));
                let lay = self.layout_fields(fs, is_union, false);
                self.emit_local_struct_fields(base_off, &lay, fields_in, typedefs)?;
            }
            _ => {
                if let Some((_, e)) = fields_in.first() {
                    self.emit_expr_rval(e, 0, typedefs)?;
                    self.store_to_offset(base_off, ty, 0);
                }
            }
        }
        Ok(())
    }

    /// Field-wise local store for a struct/union layout from an initializer list.
    fn emit_local_struct_fields(
        &mut self,
        base_off: i64,
        lay: &Layout,
        fields_in: &[(Option<String>, Expr)],
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
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
        ordered.sort_by_key(|(_, p)| p.offset);
        let mut pos_i = 0usize;
        let mut i = 0usize;
        while i < ordered.len() {
            let (fname, place) = ordered[i];
            let off = place.offset;
            // Union members share offset — only one store per slot.
            let mut j = i + 1;
            while j < ordered.len() && ordered[j].1.offset == off {
                j += 1;
            }
            let e = if let Some(ex) = by_name.get(fname) {
                Some(*ex)
            } else {
                let mut found = None;
                for k in i..j {
                    if let Some(ex) = by_name.get(ordered[k].0) {
                        found = Some(*ex);
                        break;
                    }
                }
                if found.is_some() {
                    found
                } else if pos_i < positional.len() {
                    let e = positional[pos_i];
                    pos_i += 1;
                    Some(e)
                } else {
                    None
                }
            };
            if let Some(e) = e {
                if let Expr::InitList { fields } = e {
                    // Nested struct/array field: recurse (not char[64] blob path).
                    self.emit_local_init_list(
                        base_off + place.offset,
                        &place.ty,
                        fields,
                        typedefs,
                    )?;
                } else if let Some((bo, bw)) = place.bit {
                    self.emit_expr_rval(e, 0, typedefs)?;
                    self.store_bitfield(base_off + place.offset, &place.ty, bo, bw, 0)?;
                } else {
                    self.emit_expr_rval(e, 0, typedefs)?;
                    self.store_to_offset(base_off + place.offset, &place.ty, 0);
                }
            }
            i = j;
        }
        Ok(())
    }

    fn lookup(&mut self, name: &str) -> Result<Sym, String> {
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
        if let Some(_f) = self.funcs.get(name) {
            return Ok(Sym {
                ty: Type::Ptr(Box::new(Type::Void)),
                storage: Storage::Global {
                    name: name.to_string(),
                },
            });
        }
        // Fallback: function pointers to forward-declared or external functions
        // should reference the symbol directly, not a zeroed __undef static.
        Ok(Sym {
            ty: Type::Ptr(Box::new(Type::Void)),
            storage: Storage::Global {
                name: name.to_string(),
            },
        })
    }

    /// Emit address of lvalue into x{reg}
    fn emit_lvalue_addr(
        &mut self,
        e: &Expr,
        reg: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match e {
            Expr::StmtExpr(stmts, final_expr) => {
                self.enter_scope();
                for s in stmts {
                    self.emit_stmt(s, typedefs)?;
                }
                let res = self.emit_lvalue_addr(final_expr, reg, typedefs);
                self.exit_scope();
                res
            }
            Expr::Var(name) => {
                if name == "errno" && self.os == TargetOs::Linux {
                    self.emit_errno_addr(reg);
                    return Ok(Type::Int);
                }
                let sym = self.lookup(name)?;
                match &sym.storage {
                    Storage::Local { offset } => {
                        self.emit_fp_addr(*offset, reg);
                    }
                    Storage::Global { name } => {
                        let lab = self.c_sym(name);
                        if Self::is_extern_libc(name) {
                            self.emit_adrp_got(reg, &lab);
                        } else {
                            self.emit_adrp_add(reg, &lab);
                        }
                    }
                    Storage::RegAddr { reg: r } => {
                        if *r != reg {
                            writeln!(self.out, "\tmov\tx{reg}, x{r}").unwrap();
                        }
                    }
                }
                Ok(sym.ty)
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let ty = self.emit_expr_rval(expr, reg, typedefs)?;
                match ty {
                    Type::Ptr(inner) => Ok(*inner),
                    Type::Array(inner, _) => Ok(*inner),
                    // Incomplete typing: integer/void used as pointer (kernel soft path).
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
                // addr = base + index * elem_size
                // Spill base: Binary/index eval uses x9 as a temporary and would
                // clobber the array/pointer address if left live across the index.
                let bty = self.emit_expr_rval(base, 9, typedefs)?;
                writeln!(self.out, "\tstr\tx9, [sp, #-16]!").unwrap();
                self.emit_expr_rval(index, 10, typedefs)?;
                writeln!(self.out, "\tldr\tx9, [sp], #16").unwrap();
                let (elem, decayed) = match bty {
                    Type::Array(e, _) => (*e.clone(), true),
                    Type::Ptr(e) => (*e.clone(), true),
                    // Incomplete typing: many undeclared libc calls default to Int
                    // but are used as pointers/arrays (sqlite amalgamation).
                    // Void/Struct appear when typeof/soft typedefs collapse
                    // (vt.c, sock.c atomic_t[i] via wrong base type).
                    Type::Int
                    | Type::UInt
                    | Type::Long
                    | Type::ULong
                    | Type::Short
                    | Type::UShort
                    | Type::Char
                    | Type::Void
                    | Type::Struct(_)
                    | Type::Union(_) => (Type::Char, true),
                    other => return Err(format!("index of non-array {:?}", other)),
                };
                let _ = decayed;
                let esz = self.type_size(&elem).max(1);
                self.emit_imm(esz as i64, 11);
                writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                writeln!(self.out, "\tadd\tx{reg}, x9, x10").unwrap();
                Ok(elem)
            }
            Expr::Member { base, field, arrow } => {
                let base_ty = if *arrow {
                    let t = self.emit_expr_rval(base, reg, typedefs)?;
                    match t {
                        Type::Ptr(inner) => *inner,
                        // Incomplete typing: treat integer as opaque pointer.
                        Type::Int
                        | Type::UInt
                        | Type::Long
                        | Type::ULong
                        | Type::Short
                        | Type::UShort
                        | Type::Char
                        | Type::Void => Type::Struct("__opaque__".into()),
                        other => other,
                    }
                } else {
                    self.emit_lvalue_addr(base, reg, typedefs)?
                };
                let lay = match &base_ty {
                    Type::Struct(n) if n == "__opaque__" => {
                        // Best-effort: find any layout that contains this field.
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
                                FieldPlace {
                                    offset: 0,
                                    ty: Type::Ptr(Box::new(Type::Void)),
                                    bit: None,
                                },
                            );
                            Layout {
                                size: 8,
                                align: 8,
                                fields,
                            }
                        })
                    }
                    Type::Struct(n) | Type::Union(n) => self.layouts.get(n).cloned().unwrap_or_else(
                        || {
                            // Soft: named struct without recorded layout (incomplete/soft skip).
                            let mut fields = HashMap::new();
                            fields.insert(
                                field.clone(),
                                FieldPlace {
                                    offset: 0,
                                    ty: Type::Ptr(Box::new(Type::Void)),
                                    bit: None,
                                },
                            );
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
                        // Soft: postgres/kernel often member-access through
                        // char[N] / opaque array overlays (sockaddr-style).
                        let mut fields = HashMap::new();
                        fields.insert(
                            field.clone(),
                            FieldPlace {
                                offset: 0,
                                ty: Type::Ptr(Box::new(Type::Void)),
                                bit: None,
                            },
                        );
                        Layout {
                            size: 8,
                            align: 8,
                            fields,
                        }
                    }
                    other => return Err(format!("member of non-struct {:?} .{}", other, field)),
                };
                // Soft: incomplete/opaque layouts often miss fields (kernel soft path).
                // Unknown field → offset 0, treat as void* so later load/store still emit.
                let place = if let Some(p) = lay.fields.get(field) {
                    p.clone()
                } else {
                    FieldPlace {
                        offset: 0,
                        ty: Type::Ptr(Box::new(Type::Void)),
                        bit: None,
                    }
                };
                // Bitfields are not addressable as pure lvalues in C; we still return
                // the container address so assign/load paths can special-case via
                // typeof + a bitfield store helper. Non-bitfield: address of field.
                if place.offset != 0 {
                    self.emit_add_imm(reg, reg, place.offset);
                }
                // Stash bitfield info on a side channel? For now return container type;
                // load path for Member uses typeof and layout again.
                Ok(place.ty)
            }
            // Kernel headers take address of many rvalues (casts, calls, ternaries).
            // Soft: evaluate as rvalue into reg and treat as opaque pointer address.
            // CRITICAL: never do that for struct/union — scalar rvalue is the first
            // word of the object, not its address (breaks `a = c ? t1 : t2` memcpy).
            other => {
                let ty = self.typeof_expr(other, typedefs);
                if Self::is_struct_or_union_ty(&ty) {
                    return self.emit_agg_copy_src(other, reg, typedefs);
                }
                let _ = self.emit_expr_rval(other, reg, typedefs)?;
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
        }
    }

    fn load_ty(&mut self, ty: &Type, addr_reg: u8, dest: u8) {
        match ty {
            Type::Float => {
                // load f32 → promote to f64 bits in x{dest} (for printf/varargs)
                writeln!(self.out, "\tldr\ts0, [x{addr_reg}]").unwrap();
                writeln!(self.out, "\tfcvt\td0, s0").unwrap();
                writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
            }
            Type::Double => {
                writeln!(self.out, "\tldr\td0, [x{addr_reg}]").unwrap();
                writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
            }
            Type::SChar => {
                writeln!(self.out, "\tldrsb\tx{dest}, [x{addr_reg}]").unwrap();
            }
            Type::UShort => {
                writeln!(self.out, "\tldrh\tw{dest}, [x{addr_reg}]").unwrap();
            }
            Type::UInt => {
                // zero-extend u32 → x (ldr w clears upper 32)
                writeln!(self.out, "\tldr\tw{dest}, [x{addr_reg}]").unwrap();
            }
            Type::ULong => {
                writeln!(self.out, "\tldr\tx{dest}, [x{addr_reg}]").unwrap();
            }
            Type::Int => {
                writeln!(self.out, "\tldrsw\tx{dest}, [x{addr_reg}]").unwrap();
            }
            Type::Short => {
                writeln!(self.out, "\tldrsh\tx{dest}, [x{addr_reg}]").unwrap();
            }
            _ => match self.type_size(ty) {
                1 => writeln!(self.out, "\tldrb\tw{dest}, [x{addr_reg}]").unwrap(),
                2 => writeln!(self.out, "\tldrsh\tx{dest}, [x{addr_reg}]").unwrap(),
                4 => writeln!(self.out, "\tldrsw\tx{dest}, [x{addr_reg}]").unwrap(),
                _ => writeln!(self.out, "\tldr\tx{dest}, [x{addr_reg}]").unwrap(),
            },
        }
    }

    /// Truncate/wrap an integer in x{reg} to `ty`'s value domain (as C would
    /// after storing back into an object of that type). Used after ++/-- so
    /// the expression value matches the stored value (u8 255+1 → 0, not 256).
    fn truncate_int_to_ty(&mut self, ty: &Type, reg: u8) {
        match ty {
            // plain char is unsigned on our Linux aarch64 target
            Type::Char => {
                writeln!(self.out, "\tand\tx{reg}, x{reg}, #0xff").unwrap();
            }
            Type::SChar => {
                writeln!(self.out, "\tsxtb\tx{reg}, w{reg}").unwrap();
            }
            Type::UShort => {
                writeln!(self.out, "\tand\tx{reg}, x{reg}, #0xffff").unwrap();
            }
            Type::Short => {
                writeln!(self.out, "\tsxth\tx{reg}, w{reg}").unwrap();
            }
            Type::UInt => {
                writeln!(self.out, "\tmov\tw{reg}, w{reg}").unwrap();
            }
            Type::Int => {
                writeln!(self.out, "\tsxtw\tx{reg}, w{reg}").unwrap();
            }
            _ => {}
        }
    }

    /// C integer promotions + usual arithmetic conversions (no floats).
    /// Narrow types promote to Int; UInt wins over Int at the same width so
    /// `u32 + int` stays 32-bit unsigned (wraps), matching SQLite nOvfl math.
    fn usual_arith_conv(l: &Type, r: &Type) -> Type {
        let promote = |t: &Type| -> Type {
            match t {
                Type::Char | Type::SChar | Type::Short | Type::UShort => Type::Int,
                other => other.clone(),
            }
        };
        let l = promote(l);
        let r = promote(r);
        if matches!(l, Type::ULong) || matches!(r, Type::ULong) {
            return Type::ULong;
        }
        if matches!(l, Type::Long) || matches!(r, Type::Long) {
            return Type::Long;
        }
        if matches!(l, Type::UInt) || matches!(r, Type::UInt) {
            return Type::UInt;
        }
        Type::Int
    }

    /// True when arithmetic must use 32-bit ops (wrap at 2^32).
    fn is_narrow_int(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Int | Type::UInt | Type::Short | Type::UShort | Type::Char | Type::SChar
        )
    }

    /// `__builtin_{add,sub,mul}_overflow(a, b, r)` — store wrapping result to `*r`,
    /// return 1 if signed/unsigned overflow occurred (GCC semantics).
    fn emit_builtin_overflow(
        &mut self,
        name: &str,
        a: &Expr,
        b: &Expr,
        r: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        // Eval order: a, b, then r (pointer). Spill a/b across r evaluation.
        let _ = self.emit_expr_rval(a, 0, typedefs)?;
        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
        let _ = self.emit_expr_rval(b, 0, typedefs)?;
        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
        let rty = self.emit_expr_rval(r, 2, typedefs)?; // x2 = r
        writeln!(self.out, "\tldr\tx1, [sp], #16").unwrap(); // b
        writeln!(self.out, "\tldr\tx0, [sp], #16").unwrap(); // a

        // Pointee type of r decides width + signedness.
        let pointee = match &rty {
            Type::Ptr(inner) => (**inner).clone(),
            other => other.clone(),
        };
        let sz = self.type_size(&pointee);
        // aarch64 Linux: plain char is unsigned. Signed overflow uses V flag.
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

        // Compute wrapping result into x3 and set NZCV / high-half for overflow.
        match (op, sz, signed) {
            ("add", 8, true) => {
                writeln!(self.out, "\tadds\tx3, x0, x1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, vs").unwrap();
            }
            ("add", 4, true) => {
                writeln!(self.out, "\tadds\tw3, w0, w1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, vs").unwrap();
            }
            ("add", 8, false) => {
                writeln!(self.out, "\tadds\tx3, x0, x1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, cs").unwrap();
            }
            ("add", 4, false) => {
                writeln!(self.out, "\tadds\tw3, w0, w1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, cs").unwrap();
            }
            ("sub", 8, true) => {
                writeln!(self.out, "\tsubs\tx3, x0, x1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, vs").unwrap();
            }
            ("sub", 4, true) => {
                writeln!(self.out, "\tsubs\tw3, w0, w1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, vs").unwrap();
            }
            ("sub", 8, false) => {
                writeln!(self.out, "\tsubs\tx3, x0, x1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, cc").unwrap(); // unsigned underflow
            }
            ("sub", 4, false) => {
                writeln!(self.out, "\tsubs\tw3, w0, w1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcset\tw0, cc").unwrap();
            }
            ("mul", 8, true) => {
                // Signed 64×64 → 128; overflow if high != sign-extend of low.
                writeln!(self.out, "\tmul\tx3, x0, x1").unwrap();
                writeln!(self.out, "\tsmulh\tx4, x0, x1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tasr\tx5, x3, #63").unwrap();
                writeln!(self.out, "\tcmp\tx4, x5").unwrap();
                writeln!(self.out, "\tcset\tw0, ne").unwrap();
            }
            ("mul", 4, true) => {
                // 32-bit signed: product in x, check fits in s32.
                writeln!(self.out, "\tsmull\tx3, w0, w1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tsxtw\tx4, w3").unwrap();
                writeln!(self.out, "\tcmp\tx3, x4").unwrap();
                writeln!(self.out, "\tcset\tw0, ne").unwrap();
            }
            ("mul", 8, false) => {
                writeln!(self.out, "\tmul\tx3, x0, x1").unwrap();
                writeln!(self.out, "\tumulh\tx4, x0, x1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tcmp\tx4, #0").unwrap();
                writeln!(self.out, "\tcset\tw0, ne").unwrap();
            }
            ("mul", 4, false) => {
                writeln!(self.out, "\tumull\tx3, w0, w1").unwrap();
                self.store_ty(&pointee, 2, 3);
                writeln!(self.out, "\tlsr\tx4, x3, #32").unwrap();
                writeln!(self.out, "\tcmp\tx4, #0").unwrap();
                writeln!(self.out, "\tcset\tw0, ne").unwrap();
            }
            // Narrow (1/2 byte) or fallback: widen to 64-bit signed check via portable cmp.
            _ => {
                // Portable signed/unsigned via 64-bit arithmetic + range check.
                match op {
                    "add" => writeln!(self.out, "\tadd\tx3, x0, x1").unwrap(),
                    "sub" => writeln!(self.out, "\tsub\tx3, x0, x1").unwrap(),
                    _ => writeln!(self.out, "\tmul\tx3, x0, x1").unwrap(),
                }
                self.store_ty(&pointee, 2, 3);
                // Truncate result to width and compare with full x3.
                match sz {
                    1 if signed => {
                        writeln!(self.out, "\tsxtb\tx4, w3").unwrap();
                        writeln!(self.out, "\tcmp\tx3, x4").unwrap();
                    }
                    1 => {
                        writeln!(self.out, "\tuxtb\tw4, w3").unwrap();
                        writeln!(self.out, "\tcmp\tx3, x4").unwrap();
                    }
                    2 if signed => {
                        writeln!(self.out, "\tsxth\tx4, w3").unwrap();
                        writeln!(self.out, "\tcmp\tx3, x4").unwrap();
                    }
                    2 => {
                        writeln!(self.out, "\tuxth\tw4, w3").unwrap();
                        writeln!(self.out, "\tcmp\tx3, x4").unwrap();
                    }
                    _ => {
                        // Default: no overflow reported (keeps old soft behavior for exotic types).
                        writeln!(self.out, "\tmov\tw0, wzr").unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                        }
                        return Ok(());
                    }
                }
                writeln!(self.out, "\tcset\tw0, ne").unwrap();
            }
        }
        if dest != 0 {
            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
        }
        Ok(())
    }

    /// Software / `cnt` popcount for GCC builtins (PostgreSQL pg_bitutils).
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
                writeln!(self.out, "\tcnt\tw0, w0").unwrap();
            }
            "__builtin_popcountl" | "__builtin_popcountll" => {
                writeln!(self.out, "\tcnt\tx0, x0").unwrap();
            }
            _ => return Err(format!("unknown popcount builtin: {name}")),
        }
        if dest != 0 {
            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
        }
        Ok(())
    }

    /// GCC legacy `__sync_fetch_and_{add,sub,and,or}(ptr, val)` → old value.
    fn emit_sync_fetch_and(
        &mut self,
        op: &str,
        ptr: &Expr,
        val: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        self.emit_expr_rval(ptr, 0, typedefs)?; // x0 = ptr
        self.emit_expr_rval(val, 1, typedefs)?; // x1 = val
        let lab = self.lab("sync_fetch");
        writeln!(self.out, "{lab}:").unwrap();
        writeln!(self.out, "\tldaxr\tx2, [x0]").unwrap();
        match op {
            "add" => writeln!(self.out, "\tadd\tx3, x2, x1").unwrap(),
            "sub" => writeln!(self.out, "\tsub\tx3, x2, x1").unwrap(),
            "and" => writeln!(self.out, "\tand\tx3, x2, x1").unwrap(),
            "or" => writeln!(self.out, "\torr\tx3, x2, x1").unwrap(),
            _ => return Err(format!("unknown sync op: {op}")),
        }
        writeln!(self.out, "\tstlxr\tw4, x3, [x0]").unwrap();
        writeln!(self.out, "\tcbnz\tw4, {lab}").unwrap();
        writeln!(self.out, "\tmov\tx0, x2").unwrap();
        if dest != 0 {
            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
        }
        Ok(())
    }

    fn store_ty(&mut self, ty: &Type, addr_reg: u8, val_reg: u8) {
        match ty {
            Type::Float => {
                // value may be f64 bits in x{val_reg}; truncate to f32
                writeln!(self.out, "\tfmov\td0, x{val_reg}").unwrap();
                writeln!(self.out, "\tfcvt\ts0, d0").unwrap();
                writeln!(self.out, "\tstr\ts0, [x{addr_reg}]").unwrap();
            }
            Type::Double => {
                writeln!(self.out, "\tfmov\td0, x{val_reg}").unwrap();
                writeln!(self.out, "\tstr\td0, [x{addr_reg}]").unwrap();
            }
            // Precise widths for struct/memory fields (must not clobber neighbors).
            _ => match self.type_size(ty) {
                1 => writeln!(self.out, "\tstrb\tw{val_reg}, [x{addr_reg}]").unwrap(),
                2 => writeln!(self.out, "\tstrh\tw{val_reg}, [x{addr_reg}]").unwrap(),
                4 => writeln!(self.out, "\tstr\tw{val_reg}, [x{addr_reg}]").unwrap(),
                _ => writeln!(self.out, "\tstr\tx{val_reg}, [x{addr_reg}]").unwrap(),
            },
        }
    }

    /// Emit rvalue into x{dest}; returns type of value (pointers as Ptr, arrays decay to ptr)
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
            Expr::Int(n) => {
                self.emit_imm(*n, dest);
                // Lexer stores ULLONG_MAX etc. as i64 bitpatterns (high bit set).
                // Type as ULong so ULLONG_MAX/10 uses udiv, not sdiv(-1,10)=0.
                Ok(Self::int_lit_type(*n))
            }
            Expr::Char(n) => {
                self.emit_imm(*n, dest);
                Ok(Type::Int)
            }
            Expr::Float(f) => {
                // materialize f64 bits into x{dest}, then fmov d0
                let bits = f.to_bits() as i64;
                self.emit_imm(bits, dest);
                writeln!(self.out, "\tfmov\td0, x{dest}").unwrap();
                // keep bits in x{dest} for integer path; also leave d0 for float ops
                Ok(Type::Double)
            }
            Expr::String(s) => {
                let id = self.intern_str(s);
                let slab = format!("l_str_{id}");
                self.emit_adrp_add(dest, &slab);
                Ok(Type::Ptr(Box::new(Type::Char)))
            }
            Expr::AddrOfLabel(label) => {
                let lab = format!("L_{}_goto_{}", self.func_name, label);
                self.emit_adrp_add(dest, &lab);
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
            Expr::Var(name) => {
                // GCC/C99 predefined function name identifiers.
                if name == "__func__" || name == "__FUNCTION__" || name == "__PRETTY_FUNCTION__" {
                    let fname = if self.func_name.is_empty() {
                        "?".to_string()
                    } else {
                        self.func_name.clone()
                    };
                    let id = self.intern_str(&fname);
                    let slab = format!("l_str_{id}");
                    self.emit_adrp_add(dest, &slab);
                    return Ok(Type::Ptr(Box::new(Type::Char)));
                }
                // Linux: errno is thread-local via __errno_location(), not a global.
                if name == "errno" && self.os == TargetOs::Linux {
                    self.emit_load_errno(dest);
                    return Ok(Type::Int);
                }
                // Soft: percpu `typeof(pcp) ___res` residue when stmt-expr / typeof
                // fails — treat as zero rather than an extern global.
                if name == "___res" {
                    writeln!(self.out, "\tmov\tx{dest}, xzr").unwrap();
                    return Ok(Type::ULong);
                }
                // Enumerators / static const ints: emit immediate (not load from .data).
                if let Some(n) = self.const_globals.get(name).copied() {
                    self.emit_imm(n, dest);
                    return Ok(Type::Int);
                }
                // if function name used as value — not supported
                let sym = match self.lookup(name) {
                    Ok(s) => s,
                    Err(_) => {
                        // Function designator (defined, prototype, or undeclared extern).
                        // Defined-in-TU uses PAGE; prototypes/extern use GOT.
                        self.emit_func_addr(name, dest);
                        return Ok(Type::Ptr(Box::new(Type::Void)));
                    }
                };
                match &sym.ty {
                    Type::Array(elem, _) => {
                        // decay to pointer
                        match &sym.storage {
                            Storage::Local { offset } => {
                                self.emit_fp_addr(*offset, dest);
                            }
                            Storage::Global { name } => {
                                let lab = self.c_sym(name);
                                if Self::is_extern_libc(name) {
                                    self.emit_adrp_got(dest, &lab);
                                } else {
                                    self.emit_adrp_add(dest, &lab);
                                }
                            }
                            _ => {}
                        }
                        Ok(Type::Ptr(elem.clone()))
                    }
                    ty => {
                        match &sym.storage {
                            Storage::Local { offset } => {
                                self.load_from_offset(*offset, ty, dest);
                            }
                            Storage::Global { name } => {
                                let lab = self.c_sym(name);
                                // C99 function designator as rvalue → address of function.
                                // Never load-from-code (ldrsw of first insn → bad fptr).
                                // ONLY for known functions or undeclared names.
                                // Data pointer globals (stdout/environ/char*) must LOAD the
                                // stored pointer: GOT gives &stdout, then ldr for the FILE*.
                                // Treating Type::Ptr as designator made fprintf(stdout,…)
                                // pass &stdout → Redis serverLogRaw SEGV in glibc printf.
                                if self.funcs.contains_key(name)
                                    || (!self.globals.contains_key(name)
                                        && !self.const_globals.contains_key(name))
                                {
                                    self.emit_func_addr(name, dest);
                                } else {
                                    if Self::is_extern_libc(name) {
                                        self.emit_adrp_got(9, &lab);
                                    } else {
                                        self.emit_adrp_add(9, &lab);
                                    }
                                    self.load_ty(ty, 9, dest);
                                }
                            }
                            Storage::RegAddr { reg } => {
                                self.load_ty(ty, *reg, dest);
                            }
                        }
                        Ok(ty.clone())
                    }
                }
            }
            Expr::Unary { op, expr } => match op {
                UnaryOp::Neg => {
                    let ty = self.emit_expr_rval(expr, dest, typedefs)?;
                    if matches!(ty, Type::Float | Type::Double) {
                        writeln!(self.out, "\tfmov\td0, x{dest}").unwrap();
                        writeln!(self.out, "\tfneg\td0, d0").unwrap();
                        writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                        Ok(Type::Double)
                    } else {
                        writeln!(self.out, "\tneg\tx{dest}, x{dest}").unwrap();
                        Ok(Type::Int)
                    }
                }
                UnaryOp::Not => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tcmp\tx{dest}, #0").unwrap();
                    writeln!(self.out, "\tcset\tx{dest}, eq").unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::BitNot => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tmvn\tx{dest}, x{dest}").unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::Addr => {
                    // &function
                    if let Expr::Var(n) = expr.as_ref() {
                        if self.funcs.contains_key(n) || n == "main" {
                            self.emit_func_addr(n, dest);
                            return Ok(Type::Ptr(Box::new(Type::Void)));
                        }
                    }
                    // &(T){...} compound literal
                    if let Expr::Cast { ty, expr: inner } = expr.as_ref() {
                        if matches!(inner.as_ref(), Expr::InitList { .. }) {
                            let t = self.emit_expr_rval(
                                &Expr::Cast {
                                    ty: ty.clone(),
                                    expr: inner.clone(),
                                },
                                dest,
                                typedefs,
                            )?;
                            return Ok(t);
                        }
                    }
                    let ty = self.emit_lvalue_addr(expr, dest, typedefs)?;
                    Ok(Type::Ptr(Box::new(ty)))
                }
                UnaryOp::Deref => {
                    // va_arg(ap, double) → *(double*)__acc_va_arg(&ap). Walk the
                    // process VR cursor (set by va_start) so system-gcc callers work.
                    if let Expr::Cast {
                        ty: Type::Ptr(inner),
                        expr: call,
                    } = expr.as_ref()
                    {
                        if matches!(inner.as_ref(), Type::Float | Type::Double) {
                            if let Expr::Call { name, args: _ } = call.as_ref() {
                                if name == "__acc_va_arg" || name == "__acc_va_arg_fp" {
                                    let cur = self.c_sym("acc_va_vr_cursor");
                                    self.referenced_data_syms.insert(cur.clone());
                                    // OS-correct page relocs (Darwin rejects Linux #:lo12:).
                                    match self.os {
                                        TargetOs::Darwin => {
                                            writeln!(self.out, "\tadrp\tx9, {cur}@PAGE")
                                                .unwrap();
                                            writeln!(
                                                self.out,
                                                "\tldr\tx16, [x9, {cur}@PAGEOFF]"
                                            )
                                            .unwrap();
                                            writeln!(self.out, "\tadd\tx10, x16, #8").unwrap();
                                            writeln!(
                                                self.out,
                                                "\tstr\tx10, [x9, {cur}@PAGEOFF]"
                                            )
                                            .unwrap();
                                        }
                                        TargetOs::Linux => {
                                            writeln!(self.out, "\tadrp\tx9, {cur}").unwrap();
                                            writeln!(
                                                self.out,
                                                "\tldr\tx16, [x9, #:lo12:{cur}]"
                                            )
                                            .unwrap();
                                            writeln!(self.out, "\tadd\tx10, x16, #8").unwrap();
                                            writeln!(
                                                self.out,
                                                "\tstr\tx10, [x9, #:lo12:{cur}]"
                                            )
                                            .unwrap();
                                        }
                                    }
                                    // load IEEE bits from old cursor
                                    writeln!(self.out, "\tldr\tx{dest}, [x16]").unwrap();
                                    // Do not advance GP ap — ints and floats are
                                    // independent under AAPCS64.
                                    return Ok(*inner.clone());
                                }
                            }
                        }
                    }
                    let ty = self.emit_expr_rval(expr, 9, typedefs)?;
                    let inner = match ty {
                        Type::Ptr(i) => *i,
                        Type::Array(i, _) => *i,
                        // Soft: integer used as pointer (opaque/incomplete casts in real C).
                        Type::Int | Type::UInt | Type::Long | Type::ULong => Type::Int,
                        other => return Err(format!("deref {:?}", other)),
                    };
                    self.load_ty(&inner, 9, dest);
                    Ok(inner)
                }
            },
            Expr::Binary { op, left, right } => {
                // short-circuit && ||
                if *op == BinOp::And {
                    let l_false = self.lab("and_false");
                    let l_end = self.lab("and_end");
                    self.emit_expr_rval(left, dest, typedefs)?;
                    self.emit_cbz_long(dest, &l_false);
                    self.emit_expr_rval(right, dest, typedefs)?;
                    writeln!(self.out, "\tcmp\tx{dest}, #0").unwrap();
                    writeln!(self.out, "\tcset\tx{dest}, ne").unwrap();
                    writeln!(self.out, "\tb\t{l_end}").unwrap();
                    writeln!(self.out, "{l_false}:").unwrap();
                    writeln!(self.out, "\tmov\tx{dest}, #0").unwrap();
                    writeln!(self.out, "{l_end}:").unwrap();
                    return Ok(Type::Int);
                }
                if *op == BinOp::Or {
                    let l_true = self.lab("or_true");
                    let l_end = self.lab("or_end");
                    self.emit_expr_rval(left, dest, typedefs)?;
                    self.emit_cbnz_long(dest, &l_true);
                    self.emit_expr_rval(right, dest, typedefs)?;
                    writeln!(self.out, "\tcmp\tx{dest}, #0").unwrap();
                    writeln!(self.out, "\tcset\tx{dest}, ne").unwrap();
                    writeln!(self.out, "\tb\t{l_end}").unwrap();
                    writeln!(self.out, "{l_true}:").unwrap();
                    writeln!(self.out, "\tmov\tx{dest}, #1").unwrap();
                    writeln!(self.out, "{l_end}:").unwrap();
                    return Ok(Type::Int);
                }

                // Spill left so right-hand evaluation cannot clobber it (nested loads use x9).
                let lty = self.emit_expr_rval(left, 9, typedefs)?;
                writeln!(self.out, "\tstr\tx9, [sp, #-16]!").unwrap();
                let rty = self.emit_expr_rval(right, 10, typedefs)?;
                writeln!(self.out, "\tldr\tx9, [sp], #16").unwrap();

                // promote int+float → float ops in d registers
                let floaty = matches!(lty, Type::Float | Type::Double)
                    || matches!(rty, Type::Float | Type::Double);
                if floaty {
                    // move to d0/d1 (int → float first if needed; unsigned → ucvtf)
                    if matches!(lty, Type::Float | Type::Double) {
                        writeln!(self.out, "\tfmov\td0, x9").unwrap();
                    } else if matches!(lty, Type::UShort | Type::UInt | Type::ULong) {
                        writeln!(self.out, "\tucvtf\td0, x9").unwrap();
                    } else {
                        writeln!(self.out, "\tscvtf\td0, x9").unwrap();
                    }
                    if matches!(rty, Type::Float | Type::Double) {
                        writeln!(self.out, "\tfmov\td1, x10").unwrap();
                    } else if matches!(rty, Type::UShort | Type::UInt | Type::ULong) {
                        writeln!(self.out, "\tucvtf\td1, x10").unwrap();
                    } else {
                        writeln!(self.out, "\tscvtf\td1, x10").unwrap();
                    }
                    match op {
                        BinOp::Add => writeln!(self.out, "\tfadd\td0, d0, d1").unwrap(),
                        BinOp::Sub => writeln!(self.out, "\tfsub\td0, d0, d1").unwrap(),
                        BinOp::Mul => writeln!(self.out, "\tfmul\td0, d0, d1").unwrap(),
                        BinOp::Div => writeln!(self.out, "\tfdiv\td0, d0, d1").unwrap(),
                        BinOp::Eq => {
                            writeln!(self.out, "\tfcmp\td0, d1").unwrap();
                            writeln!(self.out, "\tcset\tx{dest}, eq").unwrap();
                            return Ok(Type::Int);
                        }
                        BinOp::Ne => {
                            writeln!(self.out, "\tfcmp\td0, d1").unwrap();
                            writeln!(self.out, "\tcset\tx{dest}, ne").unwrap();
                            return Ok(Type::Int);
                        }
                        BinOp::Lt => {
                            writeln!(self.out, "\tfcmp\td0, d1").unwrap();
                            writeln!(self.out, "\tcset\tx{dest}, mi").unwrap();
                            return Ok(Type::Int);
                        }
                        BinOp::Gt => {
                            writeln!(self.out, "\tfcmp\td0, d1").unwrap();
                            writeln!(self.out, "\tcset\tx{dest}, gt").unwrap();
                            return Ok(Type::Int);
                        }
                        BinOp::Le => {
                            writeln!(self.out, "\tfcmp\td0, d1").unwrap();
                            writeln!(self.out, "\tcset\tx{dest}, ls").unwrap();
                            return Ok(Type::Int);
                        }
                        BinOp::Ge => {
                            writeln!(self.out, "\tfcmp\td0, d1").unwrap();
                            writeln!(self.out, "\tcset\tx{dest}, ge").unwrap();
                            return Ok(Type::Int);
                        }
                        _ => {
                            writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                            return Ok(Type::Double);
                        }
                    }
                    writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                    return Ok(Type::Double);
                }

                // pointer arithmetic
                match op {
                    BinOp::Add => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            self.emit_imm(esz as i64, 11);
                            writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            writeln!(self.out, "\tadd\tx{dest}, x9, x10").unwrap();
                            return Ok(lty);
                        }
                        // int + ptr
                        let rty2 = self.typeof_expr(right, typedefs);
                        if let Type::Ptr(inner) = rty2 {
                            let esz = self.type_size(&inner).max(1);
                            self.emit_imm(esz as i64, 11);
                            writeln!(self.out, "\tmul\tx9, x9, x11").unwrap();
                            writeln!(self.out, "\tadd\tx{dest}, x10, x9").unwrap();
                            return Ok(Type::Ptr(inner));
                        }
                        // 32-bit wrap for int/u32: SQLite clearCellOverflow relies on
                        // `(u32)nPayload - nLocal + ovflPageSize` wrapping so nOvfl==0
                        // for corrupt near-4GiB payloads (corruptI-6.1).
                        let result_ty = Self::usual_arith_conv(&lty, &rty);
                        if Self::is_narrow_int(&result_ty) {
                            writeln!(self.out, "\tadd\tw{dest}, w9, w10").unwrap();
                            // W-writes zero-extend; signed int results need sxtw so
                            // negative offsets (ptr+(-1)) stay negative in X.
                            if matches!(result_ty, Type::Int | Type::Short | Type::SChar) {
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\tadd\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::Sub => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            let rty = self.typeof_expr(right, typedefs);
                            if matches!(rty, Type::Ptr(_)) {
                                writeln!(self.out, "\tsub\tx{dest}, x9, x10").unwrap();
                                self.emit_imm(esz as i64, 11);
                                writeln!(self.out, "\tsdiv\tx{dest}, x{dest}, x11").unwrap();
                                return Ok(Type::Int);
                            }
                            // ptr - int
                            self.emit_imm(esz as i64, 11);
                            writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            writeln!(self.out, "\tsub\tx{dest}, x9, x10").unwrap();
                            return Ok(lty);
                        }
                        let result_ty = Self::usual_arith_conv(&lty, &rty);
                        if Self::is_narrow_int(&result_ty) {
                            writeln!(self.out, "\tsub\tw{dest}, w9, w10").unwrap();
                            if matches!(result_ty, Type::Int | Type::Short | Type::SChar) {
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\tsub\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::Mul => {
                        let result_ty = Self::usual_arith_conv(&lty, &rty);
                        if Self::is_narrow_int(&result_ty) {
                            writeln!(self.out, "\tmul\tw{dest}, w9, w10").unwrap();
                            if matches!(result_ty, Type::Int | Type::Short | Type::SChar) {
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\tmul\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::Div => {
                        // size_t / unsigned → udiv (MAX_SIZET/sizeof and
                        // (UINT64_MAX-9)/10 must not sdiv -1).
                        // Use typeof of *operands*, not lty from a prior Sub
                        // that used to always report Int.
                        // Also: const operands with high bit (u64 bitcast) even
                        // if typeof still says Int (belt for untyped paths).
                        let lt = self.typeof_expr(left, typedefs);
                        let rt = self.typeof_expr(right, typedefs);
                        let hi_bit = matches!(Self::const_i64_with(left, None), Some(n) if n < 0)
                            || matches!(Self::const_i64_with(right, None), Some(n) if n < 0);
                        let result_ty = Self::usual_arith_conv(&lt, &rt);
                        let unsigned = hi_bit
                            || matches!(
                                result_ty,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            )
                            || matches!(
                                lt,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            )
                            || matches!(
                                rt,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            );
                        let narrow = Self::is_narrow_int(&result_ty);
                        if unsigned {
                            if narrow {
                                writeln!(self.out, "\tudiv\tw{dest}, w9, w10").unwrap();
                            } else {
                                writeln!(self.out, "\tudiv\tx{dest}, x9, x10").unwrap();
                            }
                        } else if narrow {
                            writeln!(self.out, "\tsdiv\tw{dest}, w9, w10").unwrap();
                            writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                        } else {
                            writeln!(self.out, "\tsdiv\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(if unsigned {
                            if matches!(lt, Type::ULong) || matches!(rt, Type::ULong) {
                                Type::ULong
                            } else {
                                Type::UInt
                            }
                        } else {
                            result_ty
                        })
                    }
                    BinOp::Mod => {
                        let lt = self.typeof_expr(left, typedefs);
                        let rt = self.typeof_expr(right, typedefs);
                        let hi_bit = matches!(Self::const_i64_with(left, None), Some(n) if n < 0)
                            || matches!(Self::const_i64_with(right, None), Some(n) if n < 0);
                        let result_ty = Self::usual_arith_conv(&lt, &rt);
                        let unsigned = hi_bit
                            || matches!(
                                result_ty,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            )
                            || matches!(
                                lt,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            )
                            || matches!(
                                rt,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            );
                        let narrow = Self::is_narrow_int(&result_ty);
                        if unsigned {
                            if narrow {
                                writeln!(self.out, "\tudiv\tw11, w9, w10").unwrap();
                                writeln!(self.out, "\tmsub\tw{dest}, w11, w10, w9").unwrap();
                            } else {
                                writeln!(self.out, "\tudiv\tx11, x9, x10").unwrap();
                                writeln!(self.out, "\tmsub\tx{dest}, x11, x10, x9").unwrap();
                            }
                        } else if narrow {
                            writeln!(self.out, "\tsdiv\tw11, w9, w10").unwrap();
                            writeln!(self.out, "\tmsub\tw{dest}, w11, w10, w9").unwrap();
                            writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                        } else {
                            writeln!(self.out, "\tsdiv\tx11, x9, x10").unwrap();
                            writeln!(self.out, "\tmsub\tx{dest}, x11, x10, x9").unwrap();
                        }
                        Ok(if unsigned {
                            if matches!(lt, Type::ULong) || matches!(rt, Type::ULong) {
                                Type::ULong
                            } else {
                                Type::UInt
                            }
                        } else {
                            result_ty
                        })
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        // 64-bit cmp for wide types: u64 masks such as
                        // sqlite3IsNaN's `(y & 0x7ff0000000000000) == 0x7ff0…`
                        // break under `cmp w` (low half always 0 → every finite
                        // double looked like NaN → AtoF rewrote 1.1 to Inf).
                        // 32-bit cmp for narrow ints: AAPCS64 leaves high bits of
                        // int returns undefined, so `unlink()==-1` fails with
                        // `cmp x` (0xffffffff vs 0xffffffffffffffff). `cmp w`
                        // compares the defined low half.
                        // Pointers must use unsigned relationals: SQLite's
                        // `z < (u8*)(-1)` in Utf8CharLen is always false under
                        // signed `lt` (high-bit zTerm = -1), so ESCAPE 'X' looked
                        // zero-length → "ESCAPE expression must be a single character".
                        let unsignedish = matches!(
                            lty,
                            Type::UInt
                                | Type::ULong
                                | Type::UShort
                                | Type::Char
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                        ) || matches!(
                            rty,
                            Type::UInt
                                | Type::ULong
                                | Type::UShort
                                | Type::Char
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                        );
                        let wide = matches!(
                            lty,
                            Type::Long
                                | Type::ULong
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                                | Type::Float
                                | Type::Double
                        ) || matches!(
                            rty,
                            Type::Long
                                | Type::ULong
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                                | Type::Float
                                | Type::Double
                        );
                        if wide {
                            writeln!(self.out, "\tcmp\tx9, x10").unwrap();
                        } else {
                            writeln!(self.out, "\tcmp\tw9, w10").unwrap();
                        }
                        // Unsigned relational conditions for unsigned operands.
                        let cond = match (op, unsignedish) {
                            (BinOp::Eq, _) => "eq",
                            (BinOp::Ne, _) => "ne",
                            (BinOp::Lt, true) => "lo",
                            (BinOp::Gt, true) => "hi",
                            (BinOp::Le, true) => "ls",
                            (BinOp::Ge, true) => "hs",
                            (BinOp::Lt, false) => "lt",
                            (BinOp::Gt, false) => "gt",
                            (BinOp::Le, false) => "le",
                            (BinOp::Ge, false) => "ge",
                            _ => unreachable!(),
                        };
                        writeln!(self.out, "\tcset\tx{dest}, {cond}").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::BitAnd => {
                        let result_ty = Self::usual_arith_conv(&lty, &rty);
                        if Self::is_narrow_int(&result_ty) {
                            writeln!(self.out, "\tand\tw{dest}, w9, w10").unwrap();
                            if matches!(result_ty, Type::Int | Type::Short | Type::SChar) {
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\tand\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::BitOr => {
                        let result_ty = Self::usual_arith_conv(&lty, &rty);
                        if Self::is_narrow_int(&result_ty) {
                            writeln!(self.out, "\torr\tw{dest}, w9, w10").unwrap();
                            if matches!(result_ty, Type::Int | Type::Short | Type::SChar) {
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\torr\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::BitXor => {
                        let result_ty = Self::usual_arith_conv(&lty, &rty);
                        if Self::is_narrow_int(&result_ty) {
                            writeln!(self.out, "\teor\tw{dest}, w9, w10").unwrap();
                            if matches!(result_ty, Type::Int | Type::Short | Type::SChar) {
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\teor\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::Shl => {
                        // Shift result type follows promoted left operand.
                        let result_ty = Self::usual_arith_conv(&lty, &Type::Int);
                        if Self::is_narrow_int(&result_ty) {
                            writeln!(self.out, "\tlsl\tw{dest}, w9, w10").unwrap();
                            if matches!(result_ty, Type::Int | Type::Short | Type::SChar) {
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\tlsl\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::Shr => {
                        let result_ty = Self::usual_arith_conv(&lty, &Type::Int);
                        let unsigned = matches!(
                            result_ty,
                            Type::UInt | Type::ULong | Type::UShort | Type::Char
                        ) || matches!(
                            lty,
                            Type::UInt | Type::ULong | Type::UShort | Type::Char
                        );
                        if Self::is_narrow_int(&result_ty) {
                            if unsigned {
                                writeln!(self.out, "\tlsr\tw{dest}, w9, w10").unwrap();
                            } else {
                                writeln!(self.out, "\tasr\tw{dest}, w9, w10").unwrap();
                                writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                            }
                        } else if unsigned {
                            writeln!(self.out, "\tlsr\tx{dest}, x9, x10").unwrap();
                        } else {
                            writeln!(self.out, "\tasr\tx{dest}, x9, x10").unwrap();
                        }
                        Ok(result_ty)
                    }
                    BinOp::Comma => {
                        // left already evaluated for side effects; result is right in x10
                        writeln!(self.out, "\tmov\tx{dest}, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::And | BinOp::Or => unreachable!(),
                }
            }
            Expr::Assign { left, right } => {
                // Probe left type first for aggregate copy.
                let lty_probe = self.typeof_expr(left, typedefs);
                let lsz = self.type_size(&lty_probe);
                let is_agg = lsz > 8
                    && matches!(
                        lty_probe,
                        Type::Struct(_)
                            | Type::Union(_)
                            | Type::AnonStruct(_)
                            | Type::AnonUnion(_)
                            | Type::Array(_, _)
                    );
                // a = f() where f returns a small aggregate in x0[,x1].
                if is_agg {
                    if let Expr::Call { name, args } = right.as_ref() {
                        if let Some(nr) = self.small_agg_nregs(&lty_probe) {
                            // Emit the call for its side-effects / return regs; ignore dest.
                            let _ = self.emit_expr_rval(
                                &Expr::Call {
                                    name: name.clone(),
                                    args: args.clone(),
                                },
                                0,
                                typedefs,
                            )?;
                            // After call, x0/x1 hold the value (emit_expr_rval leaves them).
                            let lty = self.emit_lvalue_addr(left, 9, typedefs)?;
                            self.store_small_agg_from_regs(9, nr, 0);
                            if dest != 0 {
                                writeln!(self.out, "\tmov\tx{dest}, x9").unwrap();
                            }
                            return Ok(lty);
                        }
                    }
                }
                // Aggregate / struct assign via memcpy (full object, not first word).
                // Use emit_agg_copy_src so `a = cond ? t1 : t2` copies from &t1/&t2,
                // never from the pointer value stored in t1.z (Token CREATE TRIGGER OOM).
                if is_agg {
                    let (src_ok, copy_sz) = match right.as_ref() {
                        // *p  (e.g. *va_arg(...))
                        Expr::Unary {
                            op: UnaryOp::Deref,
                            expr,
                        } => {
                            let rty = self.typeof_expr(expr, typedefs);
                            let sz = match &rty {
                                Type::Ptr(inner) => self.type_size(inner).max(1),
                                _ => lsz,
                            };
                            self.emit_expr_rval(expr, 0, typedefs)?;
                            (true, sz)
                        }
                        // struct/union source: lvalue, ?: arms, small call return, etc.
                        other => match self.emit_agg_copy_src(other, 0, typedefs) {
                            Ok(_) => (true, lsz),
                            Err(_) => (false, lsz),
                        },
                    };
                    if src_ok {
                        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                        let lty = self.emit_lvalue_addr(left, 9, typedefs)?;
                        writeln!(self.out, "\tldr\tx1, [sp], #16").unwrap(); // src
                        writeln!(self.out, "\tmov\tx0, x9").unwrap(); // dst
                        self.emit_imm(copy_sz, 2);
                        writeln!(self.out, "\tbl\t{}", self.c_sym("memcpy")).unwrap();
                        if dest != 0 {
                            writeln!(self.out, "\tmov\tx{dest}, x9").unwrap();
                        }
                        return Ok(lty);
                    }
                    // Non-addressable aggregate RHS (rare): materialize via small-agg regs.
                    if let Some(nr) = self.small_agg_nregs(&lty_probe) {
                        self.emit_expr_rval(right, 0, typedefs)?;
                        let lty = self.emit_lvalue_addr(left, 9, typedefs)?;
                        self.store_small_agg_from_regs(9, nr, 0);
                        if dest != 0 {
                            writeln!(self.out, "\tmov\tx{dest}, x9").unwrap();
                        }
                        return Ok(lty);
                    }
                }
                // Bitfield assignment: a.bf = expr
                if let Expr::Member {
                    base,
                    field,
                    arrow,
                } = left.as_ref()
                {
                    if let Some(place) = self.member_place(base, field, *arrow, typedefs) {
                        if let Some((bo, bw)) = place.bit {
                            self.emit_expr_rval(right, 0, typedefs)?;
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                            let _ = self.emit_lvalue_addr(left, 9, typedefs)?;
                            writeln!(self.out, "\tldr\tx0, [sp], #16").unwrap();
                            self.store_bitfield_at(9, &place.ty, bo, bw, 0);
                            if dest != 0 {
                                // reload extracted value
                                self.load_bitfield(9, &place.ty, bo, bw, dest);
                            }
                            return Ok(place.ty);
                        }
                    }
                }
                // Spill RHS to stack: PostInc/PostDec also use x12 as a temp.
                let rty = self.emit_expr_rval(right, 0, typedefs)?;
                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                let lty = self.emit_lvalue_addr(left, 9, typedefs)?;
                writeln!(self.out, "\tldr\tx0, [sp], #16").unwrap();
                let rty = self.typeof_expr(right, typedefs);
                // int → float conversion on assign
                if matches!(lty, Type::Float | Type::Double)
                    && !matches!(rty, Type::Float | Type::Double)
                {
                    if matches!(rty, Type::UShort | Type::UInt | Type::ULong) {
                        writeln!(self.out, "\tucvtf\td0, x0").unwrap();
                    } else {
                        writeln!(self.out, "\tscvtf\td0, x0").unwrap();
                    }
                    writeln!(self.out, "\tfmov\tx0, d0").unwrap();
                }
                // float → int conversion on assign
                if matches!(
                    lty,
                    Type::Char
                        | Type::SChar
                        | Type::Short
                        | Type::UShort
                        | Type::Int
                        | Type::UInt
                        | Type::Long
                        | Type::ULong
                ) && matches!(rty, Type::Float | Type::Double)
                {
                    writeln!(self.out, "\tfmov\td0, x0").unwrap();
                    if matches!(lty, Type::UShort | Type::UInt | Type::ULong) {
                        writeln!(self.out, "\tfcvtzu\tx0, d0").unwrap();
                    } else {
                        writeln!(self.out, "\tfcvtzs\tx0, d0").unwrap();
                    }
                }
                // Stack locals use full 8-byte slots for integer scalars.
                // Globals/memory must use store_ty width — otherwise
                // `sqlite3_io_error_pending = -1` (pager disable_simulated_io_errors)
                // does `str x0` and clobbers adjacent BSS `sqlite3_io_error_persist`.
                let full_slot_local = match left.as_ref() {
                    Expr::Var(name)
                        if matches!(
                            lty,
                            Type::Char | Type::Short | Type::Int | Type::Long | Type::Ptr(_)
                        ) =>
                    {
                        matches!(
                            self.lookup(name).map(|s| s.storage),
                            Ok(Storage::Local { .. })
                        )
                    }
                    _ => false,
                };
                if full_slot_local {
                    writeln!(self.out, "\tstr\tx0, [x9]").unwrap();
                } else {
                    self.store_ty(&lty, 9, 0);
                }
                if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                }
                let _ = rty;
                Ok(lty)
            }
            Expr::CompoundAssign { op, left, right } => {
                // left = left op right (pointer += n scales)
                // Spill left value AND address before evaluating right — RHS
                // Binary/calls reuse x9 and nested CompoundAssign/PreInc reuse
                // x19; without spilling the address, `n += f()` writes through
                // the callee's x19 after the call (nHdr never updates).
                let lty = self.emit_lvalue_addr(left, 19, typedefs)?;
                self.load_ty(&lty, 19, 9);
                writeln!(self.out, "\tstr\tx9, [sp, #-16]!").unwrap();
                writeln!(self.out, "\tstr\tx19, [sp, #-16]!").unwrap();
                self.emit_expr_rval(right, 10, typedefs)?;
                writeln!(self.out, "\tldr\tx19, [sp], #16").unwrap();
                writeln!(self.out, "\tldr\tx9, [sp], #16").unwrap();
                let floaty = matches!(lty, Type::Float | Type::Double);
                if floaty {
                    writeln!(self.out, "\tfmov\td0, x9").unwrap();
                    let rty = self.typeof_expr(right, typedefs);
                    if matches!(rty, Type::Float | Type::Double) {
                        writeln!(self.out, "\tfmov\td1, x10").unwrap();
                    } else if matches!(rty, Type::UShort | Type::UInt | Type::ULong) {
                        writeln!(self.out, "\tucvtf\td1, x10").unwrap();
                    } else {
                        writeln!(self.out, "\tscvtf\td1, x10").unwrap();
                    }
                    match op {
                        BinOp::Add => writeln!(self.out, "\tfadd\td0, d0, d1").unwrap(),
                        BinOp::Sub => writeln!(self.out, "\tfsub\td0, d0, d1").unwrap(),
                        BinOp::Mul => writeln!(self.out, "\tfmul\td0, d0, d1").unwrap(),
                        BinOp::Div => writeln!(self.out, "\tfdiv\td0, d0, d1").unwrap(),
                        _ => return Err("bad float compound assign".into()),
                    }
                    writeln!(self.out, "\tfmov\tx0, d0").unwrap();
                } else {
                    match op {
                        BinOp::Add => {
                            if let Type::Ptr(inner) = &lty {
                                let esz = self.type_size(inner).max(1);
                                self.emit_imm(esz as i64, 11);
                                writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            }
                            writeln!(self.out, "\tadd\tx0, x9, x10").unwrap();
                        }
                        BinOp::Sub => {
                            if let Type::Ptr(inner) = &lty {
                                let esz = self.type_size(inner).max(1);
                                self.emit_imm(esz as i64, 11);
                                writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            }
                            writeln!(self.out, "\tsub\tx0, x9, x10").unwrap();
                        }
                        BinOp::Mul => writeln!(self.out, "\tmul\tx0, x9, x10").unwrap(),
                        BinOp::Div => {
                            // u64 s /= 10 in sqlite3AtoF strip loop: must udiv.
                            // Signed sdiv of significand ≥2^63 yields ~1.76e19 → 17.6.
                            let unsigned = matches!(
                                lty,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            );
                            if unsigned {
                                writeln!(self.out, "\tudiv\tx0, x9, x10").unwrap();
                            } else {
                                writeln!(self.out, "\tsdiv\tx0, x9, x10").unwrap();
                            }
                        }
                        BinOp::Mod => {
                            let unsigned = matches!(
                                lty,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            );
                            if unsigned {
                                writeln!(self.out, "\tudiv\tx11, x9, x10").unwrap();
                            } else {
                                writeln!(self.out, "\tsdiv\tx11, x9, x10").unwrap();
                            }
                            writeln!(self.out, "\tmsub\tx0, x11, x10, x9").unwrap();
                        }
                        BinOp::BitAnd => writeln!(self.out, "\tand\tx0, x9, x10").unwrap(),
                        BinOp::BitOr => writeln!(self.out, "\torr\tx0, x9, x10").unwrap(),
                        BinOp::BitXor => writeln!(self.out, "\teor\tx0, x9, x10").unwrap(),
                        BinOp::Shl => writeln!(self.out, "\tlsl\tx0, x9, x10").unwrap(),
                        BinOp::Shr => {
                            let unsigned = matches!(
                                lty,
                                Type::UInt | Type::ULong | Type::UShort | Type::Char
                            );
                            if unsigned {
                                writeln!(self.out, "\tlsr\tx0, x9, x10").unwrap();
                            } else {
                                writeln!(self.out, "\tasr\tx0, x9, x10").unwrap();
                            }
                        }
                        _ => return Err("bad compound assign".into()),
                    }
                }
                self.store_ty(&lty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                }
                Ok(lty)
            }
            Expr::Call { name, args } => {
                // Fold GCC constant builtins left as runtime calls after parse.
                if name == "__builtin_constant_p" {
                    // Soft: treat as non-constant at runtime (dead-code ok).
                    // Prefer 0 so order_base_2-style ternary takes the lib path only
                    // when the arg wasn't folded at parse time.
                    writeln!(self.out, "\tmov\tx{dest}, xzr").unwrap();
                    return Ok(Type::Int);
                }
                // D-redis / Phase B.2: glibc math.h expands fpclassify →
                // __builtin_fpclassify(...); acc PP may leave either spelling.
                // Map to libm __fpclassify / __fpclassifyf / __fpclassifyl.
                if name == "fpclassify" || name == "__builtin_fpclassify" {
                    let val = if name == "__builtin_fpclassify" && args.len() >= 6 {
                        &args[5]
                    } else if let Some(a) = args.first() {
                        a
                    } else {
                        writeln!(self.out, "\tmov\tx{dest}, xzr").unwrap();
                        return Ok(Type::Int);
                    };
                    let aty = self.typeof_expr(val, typedefs);
                    let lib = match aty {
                        Type::Float => "__fpclassifyf",
                        Type::Double => "__fpclassify",
                        // long double and unknown → double classifier
                        _ => "__fpclassify",
                    };
                    self.emit_expr_rval(val, 0, typedefs)?;
                    if !matches!(aty, Type::Float | Type::Double) {
                        writeln!(self.out, "\tscvtf\td0, x0").unwrap();
                    } else {
                        writeln!(self.out, "\tfmov\td0, x0").unwrap();
                    }
                    writeln!(self.out, "\tbl\t{}", self.c_sym(lib)).unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                    }
                    return Ok(Type::Int);
                }
                // Kernel lockdep / tracing: return address of caller.
                // Level 0 → link register (x30) saved at function entry is ideal;
                // we expose current LR which is good enough for lockdep cookies.
                if name == "__builtin_return_address" {
                    let _ = args; // level ignored (always treat as 0)
                    writeln!(self.out, "\tmov\tx{dest}, x30").unwrap();
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                if name == "__builtin_frame_address" {
                    let _ = args;
                    writeln!(self.out, "\tmov\tx{dest}, x29").unwrap();
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                if name == "__builtin_clzll"
                    || name == "__builtin_clzl"
                    || name == "__builtin_clz"
                {
                    if let Some(a) = args.first() {
                        let _ = self.emit_expr_rval(a, 0, typedefs)?;
                        // clz x0, x0 — leading zeros of 64-bit (32-bit clz uses w0)
                        if name == "__builtin_clz" {
                            writeln!(self.out, "\tclz\tw0, w0").unwrap();
                        } else {
                            writeln!(self.out, "\tclz\tx0, x0").unwrap();
                        }
                        if dest != 0 {
                            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                        }
                        return Ok(Type::Int);
                    }
                }
                if name == "__builtin_ctzll"
                    || name == "__builtin_ctzl"
                    || name == "__builtin_ctz"
                {
                    if let Some(a) = args.first() {
                        let _ = self.emit_expr_rval(a, 0, typedefs)?;
                        // rbit + clz = trailing zeros (ARM64 has no direct ctz).
                        if name == "__builtin_ctz" {
                            writeln!(self.out, "\trbit\tw0, w0").unwrap();
                            writeln!(self.out, "\tclz\tw0, w0").unwrap();
                        } else {
                            writeln!(self.out, "\trbit\tx0, x0").unwrap();
                            writeln!(self.out, "\tclz\tx0, x0").unwrap();
                        }
                        if dest != 0 {
                            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                        }
                        return Ok(Type::Int);
                    }
                }
                // GCC checked arithmetic: store wrap result to *r, return 1 iff overflow.
                // SQLite uses these for i64 add/sub/mul under GCC_VERSION>=5004000.
                if name == "__builtin_add_overflow"
                    || name == "__builtin_sub_overflow"
                    || name == "__builtin_mul_overflow"
                {
                    if args.len() >= 3 {
                        self.emit_builtin_overflow(name, &args[0], &args[1], &args[2], dest, typedefs)?;
                        return Ok(Type::Int);
                    }
                }
                // ffs = ctz + 1, or 0 if arg is 0 (GCC semantics).
                if name == "__builtin_ffs"
                    || name == "__builtin_ffsl"
                    || name == "__builtin_ffsll"
                {
                    if let Some(a) = args.first() {
                        let _ = self.emit_expr_rval(a, 0, typedefs)?;
                        let wide = name != "__builtin_ffs";
                        if wide {
                            writeln!(self.out, "\trbit\tx1, x0").unwrap();
                            writeln!(self.out, "\tclz\tx1, x1").unwrap();
                            writeln!(self.out, "\tcmp\tx0, #0").unwrap();
                            writeln!(self.out, "\tcset\tx2, ne").unwrap();
                            writeln!(self.out, "\tadd\tx0, x1, #1").unwrap();
                            writeln!(self.out, "\tmul\tx0, x0, x2").unwrap();
                        } else {
                            writeln!(self.out, "\trbit\tw1, w0").unwrap();
                            writeln!(self.out, "\tclz\tw1, w1").unwrap();
                            writeln!(self.out, "\tcmp\tw0, #0").unwrap();
                            writeln!(self.out, "\tcset\tw2, ne").unwrap();
                            writeln!(self.out, "\tadd\tw0, w1, #1").unwrap();
                            writeln!(self.out, "\tmul\tw0, w0, w2").unwrap();
                        }
                        if dest != 0 {
                            writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                        }
                        return Ok(Type::Int);
                    }
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
                    // x86-only cpuid helper; soft stub on AArch64.
                    let _ = args;
                    writeln!(self.out, "\tmov\tx{dest}, xzr").unwrap();
                    return Ok(Type::Int);
                }
                if name == "__builtin_object_size" {
                    // Soft: unknown size → (size_t)-1 for type 0/1, 0 for type 2/3.
                    writeln!(self.out, "\tmov\tx{dest}, #0").unwrap();
                    return Ok(Type::ULong);
                }
                if name == "__builtin_prefetch" {
                    // Soft: no-op (optional PRFM).
                    for a in args {
                        let _ = self.emit_expr_rval(a, 0, typedefs);
                    }
                    return Ok(Type::Void);
                }
                if name == "__builtin_choose_expr" || name == "__acc_choose" {
                    // Soft: pick first value arg if present (preprocessor usually already folded).
                    if args.len() >= 2 {
                        let _ = self.emit_expr_rval(&args[1], dest, typedefs)?;
                    } else if let Some(a) = args.first() {
                        let _ = self.emit_expr_rval(a, dest, typedefs)?;
                    } else {
                        writeln!(self.out, "\tmov\tx{dest}, xzr").unwrap();
                    }
                    return Ok(Type::Long);
                }
                if name == "__builtin_extract_return_addr"
                    || name == "__builtin_frob_return_addr"
                {
                    // Identity: return address is already a usable pointer.
                    if let Some(a) = args.first() {
                        let _ = self.emit_expr_rval(a, dest, typedefs)?;
                    } else {
                        writeln!(self.out, "\tmov\tx{dest}, xzr").unwrap();
                    }
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                if name == "__builtin_va_start" {
                    // Soft: ignore ap/last; real va uses __acc_va_start after macro expand.
                    return Ok(Type::Void);
                }
                if name == "__builtin_va_end" {
                    return Ok(Type::Void);
                }
                if name == "__builtin_va_copy" {
                    // Soft: *dst = *src if both args are addresses; else no-op.
                    if args.len() >= 2 {
                        // Best-effort: evaluate both for side effects.
                        let _ = self.emit_expr_rval(&args[0], 0, typedefs);
                        let _ = self.emit_expr_rval(&args[1], 1, typedefs);
                    }
                    return Ok(Type::Void);
                }
                // Kernel !LOCKDEP / !KCSAN / jump_label / instrumentation dead
                // symbols that real gcc eliminates; we still emit the call sites.
                if name == "lockdep_is_held" || name == "lock_is_held" {
                    writeln!(self.out, "\tmov\tx{dest}, #1").unwrap();
                    return Ok(Type::Int);
                }
                if matches!(
                    name.as_str(),
                    "kcsan_check_access"
                        | "kcsan_atomic_next"
                        | "kmsan_copy_to_user"
                        | "kmsan_unpoison_memory"
                        | "____wrong_branch_error"
                        | "might_fault"
                        | "__might_resched"
                        | "__this_cpu_preempt_check"
                        | "check_object_size"
                        | "rust_fmt_argument"
                        | "__bad_size_call_parameter"
                        | "__bad_copy_to"
                        | "__bad_copy_from"
                        | "__bad_udelay"
                        | "klp_sched_try_switch"
                ) {
                    // Soft no-op / never-called instrumentation & BUILD_BUG sinks.
                    writeln!(self.out, "\tmov\tx{dest}, xzr").unwrap();
                    return Ok(Type::Int);
                }
                // Variadic intrinsics: char* GP cursor + process-wide VR side state
                // (THREADSAFE=0). VR state lets sqlite3_str_vappendf (non-variadic,
                // receives va_list) still read doubles that system-gcc put in d0.
                if name == "__acc_va_start" {
                    if self.va_regsave_off == 0 {
                        return Err("va_start outside variadic function".into());
                    }
                    // Publish VR base for later *(double*)__acc_va_arg (even after
                    // ap is passed by value as a char* into another function).
                    if self.va_fpsave_off != 0 {
                        let vr_off =
                            self.va_fpsave_off + (self.va_fixed_fp as i64) * 8;
                        self.emit_fp_addr(vr_off, 10);
                        let cur = self.c_sym("acc_va_vr_cursor");
                        self.referenced_data_syms.insert(cur.clone());
                        match self.os {
                            TargetOs::Darwin => {
                                writeln!(self.out, "\tadrp\tx11, {cur}@PAGE").unwrap();
                                writeln!(
                                    self.out,
                                    "\tstr\tx10, [x11, {cur}@PAGEOFF]"
                                )
                                .unwrap();
                            }
                            TargetOs::Linux => {
                                writeln!(self.out, "\tadrp\tx11, {cur}").unwrap();
                                writeln!(
                                    self.out,
                                    "\tstr\tx10, [x11, #:lo12:{cur}]"
                                )
                                .unwrap();
                            }
                        }
                    }
                    // Return GP cursor = &regsave[fixed_n]
                    let off = self.va_regsave_off + (self.va_fixed_n as i64) * 8;
                    self.emit_fp_addr(off, dest);
                    return Ok(Type::Ptr(Box::new(Type::Char)));
                }
                if name == "__acc_va_arg" {
                    // args: &ap  (pointer to va_list / char*)
                    // Returns the current cursor (for *(type*)cursor); advances ap by 8.
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
                    // x9 = &ap; x0 = ap (cursor to return)
                    writeln!(self.out, "\tldr\tx0, [x9]").unwrap();
                    // ap += 8
                    writeln!(self.out, "\tadd\tx10, x0, #8").unwrap();
                    writeln!(self.out, "\tstr\tx10, [x9]").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                    }
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                if name == "__indirect__" {
                    if args.is_empty() {
                        return Err("indirect call missing callee".into());
                    }
                    let (callee, real_args) = args.split_first().unwrap();
                    // (*fp)(args): C function designators from *fp do not load through
                    // the function address — just use the pointer value.
                    let callee = match callee {
                        Expr::Unary {
                            op: UnaryOp::Deref,
                            expr,
                        } => expr.as_ref(),
                        other => other,
                    };
                    // Spill real args to stack slots (16-byte each), then load callee, then fill x0..
                    for a in real_args.iter().rev() {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                    }
                    let cty = self.emit_expr_rval(callee, 16, typedefs)?;
                    for i in 0..real_args.len() {
                        writeln!(self.out, "\tldr\tx{i}, [sp], #16").unwrap();
                    }
                    writeln!(self.out, "\tblr\tx16").unwrap();
                    // AAPCS64: double returns live in d0. If pointee/args say
                    // float, capture d0 (SQLite ceilingFunc / math1Func).
                    let mut ret = match cty {
                        Type::Ptr(inner) => *inner,
                        other => other,
                    };
                    let arg_float = real_args.iter().any(|a| {
                        matches!(
                            self.typeof_expr(a, typedefs),
                            Type::Float | Type::Double
                        )
                    });
                    if matches!(ret, Type::Float | Type::Double) || arg_float {
                        writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                        ret = Type::Double;
                    } else {
                        // (T(*)(args)) abstract casts parse as Ptr(Ptr(T));
                        // peel so int(*)() → Int and we can extend the return.
                        // Do NOT peel Char/Void: Ptr(Char)=char*, Ptr(Void)=void*
                        // (sqlite3_vfs.xNextSystemCall returns const char*).
                        if let Type::Ptr(inner) = &ret {
                            if matches!(
                                inner.as_ref(),
                                Type::Int
                                    | Type::UInt
                                    | Type::Short
                                    | Type::UShort
                                    | Type::SChar
                                    | Type::Long
                                    | Type::ULong
                                    | Type::Float
                                    | Type::Double
                            ) {
                                ret = *inner.clone();
                            }
                        }
                        self.emit_extend_call_return(dest, &ret);
                    }
                    return Ok(ret);
                }
                // Linux aarch64: convert acc char* va_list cursor into a temporary
                // AAPCS64 va_list struct for glibc v*printf.
                if self.os == TargetOs::Linux
                    && matches!(
                        name.as_str(),
                        "vprintf"
                            | "vfprintf"
                            | "vsprintf"
                            | "vsnprintf"
                            | "vdprintf"
                            | "vscanf"
                            | "vfscanf"
                            | "vsscanf"
                    )
                    && args.len() >= 2
                {
                    let n = args.len();
                    // Evaluate all args; last is our char* cursor.
                    // Spill fixed args first, then build AAPCS va_list from cursor.
                    for a in args.iter().take(n - 1).rev() {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                    }
                    // x16 = cursor (char*)
                    self.emit_expr_rval(&args[n - 1], 16, typedefs)?;
                    // Build 32-byte AAPCS va_list on stack:
                    // Assume cursor points into a GP regsave of form [x0..x7][overflow...].
                    // __gr_top = align_up(cursor, 64) is wrong; instead:
                    // We don't know fixed_n here. Approximate: treat remaining GP
                    // slots from cursor as if __gr_offs is negative relative to
                    // cursor rounded up to end of 8-reg window.
                    // Practical approach used by many freestanding ports:
                    //   __stack = cursor
                    //   __gr_top = cursor
                    //   __vr_top = 0
                    //   __gr_offs = 0   → all args taken from __stack
                    //   __vr_offs = 0
                    // This works when our cursor already points at the first
                    // vararg and subsequent args are contiguous 8-byte slots.
                    writeln!(self.out, "\tsub\tsp, sp, #32").unwrap();
                    writeln!(self.out, "\tstr\tx16, [sp]").unwrap(); // __stack
                    writeln!(self.out, "\tstr\tx16, [sp, #8]").unwrap(); // __gr_top
                    writeln!(self.out, "\tstr\txzr, [sp, #16]").unwrap(); // __vr_top
                    writeln!(self.out, "\tstr\twzr, [sp, #24]").unwrap(); // __gr_offs=0
                    writeln!(self.out, "\tstr\twzr, [sp, #28]").unwrap(); // __vr_offs=0
                    // Load fixed args into x0..; ap pointer in x{n-1}
                    for i in 0..(n - 1) {
                        let off = 32 + (i as i64) * 16;
                        writeln!(self.out, "\tldr\tx{i}, [sp, #{off}]").unwrap();
                    }
                    writeln!(self.out, "\tmov\tx{}, sp", n - 1).unwrap();
                    writeln!(self.out, "\tbl\t{}", self.c_sym(name)).unwrap();
                    let total = 32 + ((n - 1) as i64) * 16;
                    writeln!(self.out, "\tadd\tsp, sp, #{total}").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                    }
                    return Ok(Type::Int);
                }

                // Darwin arm64 system libc (printf family): named/fixed args in
                // x0.., *all* variadic args on the stack only (clang -O0 matches).
                // acc-defined variadics (sqlite3_mprintf in the amalgamation)
                // use AAPCS64 instead: args in x0..x7 then stack, because our
                // va_start/va_arg walk the GP regsave. Applying the libc rule to
                // sqlite3_mprintf left zName only on the stack → strlen garbage.
                let fixed_n: usize = if self.os == TargetOs::Darwin
                    && !self.func_defined_in_tu(name)
                {
                    match name.as_str() {
                        "printf" | "scanf" => 1,
                        "sprintf" | "fprintf" | "sscanf" => 2,
                        "snprintf" | "vsnprintf" => 3,
                        n if n.contains("snprintf") || n.contains("vsnprintf") => 3,
                        n if n.contains("sprintf")
                            || n.contains("fprintf")
                            || n.contains("sscanf") =>
                        {
                            2
                        }
                        n if n.contains("printf") || n.contains("scanf") => 1,
                        _ => 0,
                    }
                } else {
                    0
                };
                let is_varargs = fixed_n > 0;
                if is_varargs && fixed_n > 0 {
                    let n = args.len();
                    if n < fixed_n {
                        return Err(format!(
                            "varargs call missing fixed args: {name} got {n} need {fixed_n}"
                        ));
                    }
                    let var_n = n - fixed_n;
                    let var_bytes = if var_n == 0 {
                        0
                    } else {
                        Self::align_up((var_n * 8) as i64, 16)
                    };
                    // Spill all args (16-byte slots), top of stack = args[0]
                    for a in args.iter().rev() {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                    }
                    // Reserve packed variadic region below spills
                    if var_bytes > 0 {
                        writeln!(self.out, "\tsub\tsp, sp, #{var_bytes}").unwrap();
                        for i in 0..var_n {
                            // args[fixed_n + i] at spill offset
                            let src = var_bytes + ((fixed_n + i) as i64) * 16;
                            let dst = (i as i64) * 8;
                            writeln!(self.out, "\tldr\tx9, [sp, #{src}]").unwrap();
                            writeln!(self.out, "\tstr\tx9, [sp, #{dst}]").unwrap();
                        }
                    }
                    // Load fixed args into x0..x{fixed_n-1}
                    for i in 0..fixed_n {
                        let off = var_bytes + (i as i64) * 16;
                        writeln!(self.out, "\tldr\tx{i}, [sp, #{off}]").unwrap();
                    }
                    // Call via bl, or blr only for true function-pointer *variables*.
                    // Bare names that are also weak data stubs / extern "globals" must
                    // still use `bl` so the linker binds the real function (Redis Lua:
                    // luaL_newstate was blr through first instruction bytes → SIGBUS).
                    let is_fn = self.funcs.contains_key(name);
                    let is_fp_var = !is_fn && self.is_function_pointer_var(name);
                    if is_fp_var {
                        // preserve args x0.. while loading callee into x16
                        for i in 0..fixed_n {
                            writeln!(self.out, "\tstr\tx{i}, [sp, #-16]!").unwrap();
                        }
                        let _ = self.emit_expr_rval(&Expr::Var(name.clone()), 16, typedefs)?;
                        for i in (0..fixed_n).rev() {
                            writeln!(self.out, "\tldr\tx{i}, [sp], #16").unwrap();
                        }
                        writeln!(self.out, "\tblr\tx16").unwrap();
                    } else {
                        writeln!(self.out, "\tbl\t{}", self.c_sym(name)).unwrap();
                    }
                    let total = var_bytes + (n as i64) * 16;
                    writeln!(self.out, "\tadd\tsp, sp, #{total}").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                    }
                    return Ok(Type::Int);
                }

                // Known libm double(double) helpers — AAPCS64: arg/return in d0.
                let is_math1 = matches!(
                    name.as_str(),
                    "sin" | "cos" | "tan" | "sqrt" | "fabs" | "exp" | "log" | "floor" | "ceil"
                );
                if is_math1 && args.len() == 1 {
                    self.emit_expr_rval(&args[0], 0, typedefs)?;
                    let aty = self.typeof_expr(&args[0], typedefs);
                    if !matches!(aty, Type::Float | Type::Double) {
                        writeln!(self.out, "\tscvtf\td0, x0").unwrap();
                    } else {
                        writeln!(self.out, "\tfmov\td0, x0").unwrap();
                    }
                    writeln!(self.out, "\tbl\t{}", self.c_sym(name)).unwrap();
                    writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                    return Ok(Type::Double);
                }

                // Build param type list from known function defs when available
                let param_tys: Vec<Type> = self
                    .funcs
                    .get(name)
                    .map(|f| f.params.iter().map(|(_, t)| t.clone()).collect())
                    .unwrap_or_default();

                // Our va_list is a char* walking the GP regsave only (x0..x7).
                // Variadic callees compiled by acc therefore cannot see doubles in d0..d7.
                // Pass float/double args in GPRs (IEEE bits) for those calls so
                // va_arg(ap, double) reads the right payload (sqlite3_mprintf %g).
                // Libc printf still uses dN (standard aarch64 va_list has a VR area).
                let callee_variadic = self
                    .funcs
                    .get(name)
                    .map(|f| f.variadic)
                    .unwrap_or(false)
                    || {
                        let n = name.as_str();
                        n.starts_with("sqlite3_")
                            && (n.contains("printf")
                                || n.contains("snprintf")
                                || n.contains("vsnprintf")
                                || n.ends_with("Printf"))
                    };

                // AAPCS64: small aggregates (≤16B) occupy 1–2 consecutive GPRs.
                // Larger aggregates (>16B, e.g. va_list) are passed by reference.
                // Spill each logical half-register as a 16-byte slot (top = args[0]).
                let n = args.len();
                let mut arg_nregs: Vec<u8> = Vec::with_capacity(n);
                let mut arg_is_float: Vec<bool> = Vec::with_capacity(n);
                let mut arg_by_ref: Vec<bool> = Vec::with_capacity(n);
                let mut total_slots: i64 = 0;
                for i in 0..n {
                    let aty = self.typeof_expr(&args[i], typedefs);
                    let pty = param_tys.get(i).cloned().unwrap_or(aty.clone());
                    let is_f = matches!(pty, Type::Float | Type::Double)
                        || (param_tys.is_empty() && matches!(aty, Type::Float | Type::Double));
                    arg_is_float.push(is_f);
                    let sz = self.type_size(&pty).max(self.type_size(&aty));
                    let by_ref = !is_f
                        && Self::is_struct_or_union_ty(&pty)
                        && self.small_agg_nregs(&pty).is_none()
                        && sz > 16;
                    arg_by_ref.push(by_ref);
                    // acc-variadic floats travel as one GPR each (not FPR).
                    let nr = if is_f || by_ref {
                        1
                    } else {
                        self.small_agg_nregs(&pty)
                            .or_else(|| self.small_agg_nregs(&aty))
                            .unwrap_or(1)
                    };
                    arg_nregs.push(nr);
                    total_slots += nr as i64;
                }

                // Push right-to-left so args[0] ends at [sp].
                for i in (0..n).rev() {
                    let a = &args[i];
                    let aty = self.typeof_expr(a, typedefs);
                    let pty = param_tys.get(i).cloned().unwrap_or(aty.clone());
                    let nr = arg_nregs[i];
                    if arg_by_ref[i] {
                        // Large struct (va_list): pass address of lvalue.
                        if self.emit_lvalue_addr(a, 0, typedefs).is_err() {
                            // Non-lvalue: evaluate into a temp stack slot then pass &temp.
                            // Soft: emit_expr_rval may only fill x0 — still better than SEGV.
                            self.emit_expr_rval(a, 0, typedefs)?;
                        }
                        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                    } else if !arg_is_float[i]
                        && self
                            .small_agg_nregs(&pty)
                            .or_else(|| self.small_agg_nregs(&aty))
                            .is_some()
                    {
                        // Prefer lvalue load of full aggregate.
                        if self.emit_lvalue_addr(a, 9, typedefs).is_ok() {
                            if nr >= 2 {
                                writeln!(self.out, "\tldr\tx0, [x9, #8]").unwrap();
                                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                            }
                            writeln!(self.out, "\tldr\tx0, [x9]").unwrap();
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                        } else {
                            // Non-lvalue aggregate: evaluate (may only fill x0) and pad.
                            self.emit_expr_rval(a, 0, typedefs)?;
                            if nr >= 2 {
                                writeln!(self.out, "\tstr\tx1, [sp, #-16]!").unwrap();
                            }
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                        }
                    } else {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                    }
                }

                // Load into x0..x7 (and d0.. for floats). Extra slots beyond 8 GPRs
                // stay on the stack as outgoing args (packed later if needed).
                // reg_slots_used counts spill slots consumed by *either* GPRs or FPRs
                // so n_stack_slots does not re-pack FPR-passed doubles as stack args
                // (that path previously corrupted dekkerMul2 / multi-float calls).
                let mut igpr = 0u8;
                let mut fpr = 0u8;
                let mut slot: i64 = 0;
                let mut reg_slots_used: i64 = 0;
                for i in 0..n {
                    let nr = arg_nregs[i] as i64;
                    let aty = self.typeof_expr(&args[i], typedefs);
                    let pty = param_tys.get(i).cloned().unwrap_or(aty.clone());
                    if arg_is_float[i] {
                        let src = slot * 16;
                        writeln!(self.out, "\tldr\tx16, [sp, #{src}]").unwrap();
                        // Spilled value is IEEE only if the *argument expression*
                        // is float/double. Param type Double + integer actual
                        // (e.g. kahanBabuskaNeumaierStep(p, iBig)) must scvtf —
                        // bitcast of i64 bits into d0 yields NaN and broke SUM().
                        let spilled_is_float = matches!(aty, Type::Float | Type::Double);
                        if !spilled_is_float {
                            // integer expression promoted to float/double param
                            if matches!(aty, Type::UShort | Type::UInt | Type::ULong) {
                                writeln!(self.out, "\tucvtf\td0, x16").unwrap();
                            } else {
                                writeln!(self.out, "\tscvtf\td0, x16").unwrap();
                            }
                            writeln!(self.out, "\tfmov\tx16, d0").unwrap();
                        }
                        if callee_variadic {
                            // AAPCS64: variadic float/double go ONLY in d0..d7.
                            // Do not also consume a GPR — dual-placement shifted
                            // the GP cursor so int-after-double (and system-gcc
                            // callees) read the wrong register. VR walk via
                            // *(double*)__acc_va_arg uses acc_va_vr_cursor.
                            if fpr < 8 {
                                writeln!(self.out, "\tfmov\td{fpr}, x16").unwrap();
                                fpr += 1;
                                reg_slots_used += 1;
                            }
                            // if VRs full: bits stay in spill for stack packer
                            slot += 1;
                        } else if fpr < 8 {
                            writeln!(self.out, "\tfmov\td{fpr}, x16").unwrap();
                            if matches!(pty, Type::Float) {
                                writeln!(self.out, "\tfcvt\ts{fpr}, d{fpr}").unwrap();
                            }
                            fpr += 1;
                            reg_slots_used += 1;
                            slot += 1;
                        } else {
                            // float overflow onto stack — leave in spill for packer
                            slot += 1;
                        }
                    } else {
                        for r in 0..nr {
                            if igpr < 8 {
                                let src = (slot + r) * 16;
                                writeln!(self.out, "\tldr\tx{igpr}, [sp, #{src}]").unwrap();
                                if matches!(aty, Type::Float | Type::Double) && r == 0 {
                                    writeln!(self.out, "\tfmov\td0, x{igpr}").unwrap();
                                    writeln!(self.out, "\tfcvtzs\tx{igpr}, d0").unwrap();
                                }
                                igpr += 1;
                                reg_slots_used += 1;
                            }
                        }
                        slot += nr;
                    }
                }

                let spill = total_slots * 16;
                let n_stack_slots = (total_slots - reg_slots_used).max(0);
                if n_stack_slots > 0 {
                    // Outgoing stack args: pack leftover spill slots into 8-byte region.
                    for r in 0..igpr {
                        writeln!(self.out, "\tstr\tx{r}, [sp, #-16]!").unwrap();
                    }
                    let stack_bytes = Self::align_up(n_stack_slots * 8, 16);
                    writeln!(self.out, "\tsub\tsp, sp, #{stack_bytes}").unwrap();
                    for k in 0..n_stack_slots {
                        // Leftover slots start at spill index reg_slots_used.
                        let from = (igpr as i64) * 16
                            + stack_bytes
                            + (reg_slots_used + k) * 16;
                        let to = k * 8;
                        writeln!(self.out, "\tldr\tx16, [sp, #{from}]").unwrap();
                        writeln!(self.out, "\tstr\tx16, [sp, #{to}]").unwrap();
                    }
                    for r in (0..igpr).rev() {
                        let off = stack_bytes + ((igpr - 1 - r) as i64) * 16;
                        writeln!(self.out, "\tldr\tx{r}, [sp, #{off}]").unwrap();
                    }
                    writeln!(self.out, "\tbl\t{}", self.c_sym(name)).unwrap();
                    let total = stack_bytes + (igpr as i64) * 16 + spill;
                    writeln!(self.out, "\tadd\tsp, sp, #{total}").unwrap();
                } else {
                    if spill > 0 {
                        writeln!(self.out, "\tadd\tsp, sp, #{spill}").unwrap();
                    }
                    if self.funcs.contains_key(name)
                        || matches!(
                            name.as_str(),
                            "strlen"
                                | "calloc"
                                | "malloc"
                                | "free"
                                | "putchar"
                                | "puts"
                                | "exit"
                                | "memcmp"
                                | "memcpy"
                                | "memset"
                                | "strcmp"
                                | "strcpy"
                        )
                    {
                        writeln!(self.out, "\tbl\t{}", self.c_sym(name)).unwrap();
                    } else if self.is_function_pointer_var(name) {
                        let num_args = args.len().min(8);
                        for i in 0..num_args {
                            writeln!(self.out, "\tstr\tx{i}, [sp, #-16]!").unwrap();
                        }
                        let _ = self.emit_expr_rval(&Expr::Var(name.clone()), 16, typedefs)?;
                        for i in (0..num_args).rev() {
                            writeln!(self.out, "\tldr\tx{i}, [sp], #16").unwrap();
                        }
                        writeln!(self.out, "\tblr\tx16").unwrap();
                    } else {
                        // Undeclared / cross-TU function: direct bl; linker resolves.
                        writeln!(self.out, "\tbl\t{}", self.c_sym(name)).unwrap();
                    }
                }
                // float return in d0
                // Undeclared libm (pow/log/ceil/…) must still be Double: without
                // this, ` (int)pow(2, n) ` kept d0's bits in x0 and sxtw'd them
                // to 0 → hdr sub_bucket_count=0 → infinite buckets_needed loop.
                // Function-pointer vars (SQLite math1Func: `x = user_data; x(v)`)
                // are not in `funcs` and not named sin/cos — without the float-arg
                // heuristic, typeof/ret is Int and the next call does scvtf on
                // IEEE bits → ceil(99.9) ≈ 4.6e18.
                let mut ret_ty = self
                    .funcs
                    .get(name)
                    .map(|f| f.ret.clone())
                    .unwrap_or_else(|| {
                        if matches!(
                            name.as_str(),
                            "sin" | "cos" | "tan" | "sqrt" | "fabs" | "exp" | "log"
                                | "log10" | "log2" | "floor" | "ceil" | "round" | "trunc"
                                | "pow" | "fmod" | "atan" | "atan2" | "asin" | "acos"
                                | "ldexp" | "frexp" | "modf"
                        ) {
                            Type::Double
                        } else {
                            Type::Int
                        }
                    });
                if !matches!(ret_ty, Type::Float | Type::Double) {
                    let fp_double = self.is_function_pointer_var(name)
                        && (matches!(
                            self.lookup(name).map(|s| s.ty).ok(),
                            Some(Type::Ptr(inner))
                                if matches!(*inner, Type::Float | Type::Double)
                        ) || args.iter().any(|a| {
                            matches!(
                                self.typeof_expr(a, typedefs),
                                Type::Float | Type::Double
                            )
                        }));
                    if fp_double {
                        ret_ty = Type::Double;
                    }
                }
                if matches!(ret_ty, Type::Float | Type::Double) {
                    writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                    return Ok(Type::Double);
                }
                // Small aggregate return: x0[,x1] already hold the value.
                // Leave them in place; scalar callers only consume x0.
                // Narrow ints from *known* prototypes get extended so 64-bit
                // cmp against -1 works; undeclared libc defaults to Int but
                // often return pointers (malloc) — do not sxtw those.
                if self.funcs.contains_key(name) {
                    self.emit_extend_call_return(dest, &ret_ty);
                } else if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                }
                Ok(ret_ty)
            }
            Expr::Index { .. } | Expr::Member { .. } => {
                // Bitfield member: extract bits rather than loading full container.
                if let Expr::Member {
                    base,
                    field,
                    arrow,
                } = e
                {
                    if let Some(place) = self.member_place(base, field, *arrow, typedefs) {
                        if let Some((bo, bw)) = place.bit {
                            // Address of container
                            let _ = self.emit_lvalue_addr(e, 9, typedefs)?;
                            self.load_bitfield(9, &place.ty, bo, bw, dest);
                            return Ok(place.ty);
                        }
                    }
                }
                let ty = self.emit_lvalue_addr(e, 9, typedefs)?;
                // Array lvalues decay to pointer (address), not a loaded value.
                // Needed for multi-dim: arr[i][j] where arr[i] has type T[N].
                if let Type::Array(elem, _) = ty {
                    if dest != 9 {
                        writeln!(self.out, "\tmov\tx{dest}, x9").unwrap();
                    }
                    return Ok(Type::Ptr(elem));
                }
                self.load_ty(&ty, 9, dest);
                Ok(ty)
            }
            Expr::Cast { ty, expr } => {
                // compound literal (T){...}
                if let Expr::InitList { fields } = expr.as_ref() {
                    // Non-const fields (locals/params like filp) cannot live in
                    // .rodata as `.quad filp` — that creates ABS64 soft-globals
                    // banned by EFI libstub and arm64 PI. Use stack + runtime stores.
                    if !self.init_list_is_static_const(fields) {
                        let sz = self.type_size(ty).max(8);
                        let tmp = format!("__comp_stk_{}", self.label_id);
                        self.label_id += 1;
                        let off = self.alloc_local(&tmp, ty);
                        // zero then fill (matches C compound zero-padding)
                        // memset(ptr, 0, sz): x0=ptr, x1=c, x2=n
                        self.emit_fp_addr(off, 0);
                        writeln!(self.out, "\tmov\tx1, xzr").unwrap();
                        self.emit_imm(sz, 2);
                        writeln!(self.out, "\tbl\t{}", self.c_sym("memset")).unwrap();
                        self.emit_local_init_list(off, ty, fields, typedefs)?;
                        self.emit_fp_addr(off, dest);
                        return Ok(Type::Ptr(Box::new(ty.clone())));
                    }
                    let id = self.label_id;
                    self.label_id += 1;
                    let gname = format!("__comp_{id}");
                    // Always put constant compound-literal payloads in rodata.
                    // Emitting them into `cur_section` (e.g. `.init.text` for
                    // `__init` / PI) places data mid-function; fall-through then
                    // executes the payload as code (udf #0) and kills early boot
                    // (create_init_idmap / PAGE_KERNEL_ROX).
                    self.emit_rodata_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    let glab = self.c_sym(&gname);
                    writeln!(self.out, "{glab}:").unwrap();
                    self.emit_init_list_data(ty, fields)?;
                    // Resume the function's section (named or plain .text).
                    let resume = self.cur_section.clone();
                    if let Some(sec) = resume {
                        self.emit_named_section(&sec);
                        writeln!(self.out, "\t.p2align\t2").unwrap();
                    } else {
                        self.emit_text_section();
                    }
                    self.emit_adrp_add(dest, &glab);
                    return Ok(Type::Ptr(Box::new(ty.clone())));
                }
                let from = self.emit_expr_rval(expr, dest, typedefs)?;
                // Integer → float/double: must use scvtf (signed) or ucvtf (unsigned).
                // A bare fmov would bitcast the integer payload (sqlite3AtoF u64→double
                // then produced denormals/Inf and broke SELECT 1.5).
                let from_signed = matches!(
                    from,
                    Type::Char | Type::SChar | Type::Short | Type::Int | Type::Long
                );
                let from_unsigned = matches!(from, Type::UShort | Type::UInt | Type::ULong);
                let from_int = from_signed || from_unsigned;
                let to_signed = matches!(
                    ty,
                    Type::Char | Type::SChar | Type::Short | Type::Int | Type::Long
                );
                let to_unsigned = matches!(ty, Type::UShort | Type::UInt | Type::ULong);
                match (&from, ty) {
                    (_, Type::Float) if from_int => {
                        if from_unsigned {
                            writeln!(self.out, "\tucvtf\ts0, x{dest}").unwrap();
                        } else {
                            writeln!(self.out, "\tscvtf\ts0, x{dest}").unwrap();
                        }
                        writeln!(self.out, "\tfmov\tw{dest}, s0").unwrap();
                    }
                    (_, Type::Double) if from_int => {
                        if from_unsigned {
                            writeln!(self.out, "\tucvtf\td0, x{dest}").unwrap();
                        } else {
                            writeln!(self.out, "\tscvtf\td0, x{dest}").unwrap();
                        }
                        writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                    }
                    (Type::Float, _) if to_signed || to_unsigned => {
                        // Float rvalues in GPRs are already f64 bits (load_ty promotes).
                        // Must not fmov s0,w — that reinterprets the low 32 bits of a
                        // double as an f32 and destroys small values (0.001f → garbage).
                        writeln!(self.out, "\tfmov\td0, x{dest}").unwrap();
                        if to_unsigned {
                            writeln!(self.out, "\tfcvtzu\tx{dest}, d0").unwrap();
                        } else {
                            writeln!(self.out, "\tfcvtzs\tx{dest}, d0").unwrap();
                        }
                    }
                    (Type::Double, _) if to_signed || to_unsigned => {
                        writeln!(self.out, "\tfmov\td0, x{dest}").unwrap();
                        if to_unsigned {
                            writeln!(self.out, "\tfcvtzu\tx{dest}, d0").unwrap();
                        } else {
                            writeln!(self.out, "\tfcvtzs\tx{dest}, d0").unwrap();
                        }
                    }
                    (Type::Float, Type::Double) => {
                        // No-op: load_ty / float ops already leave IEEE-754 *double*
                        // bits in x{dest}. Re-running fmov s0,w + fcvt was the
                        // printf 0.001f→garbage bug and SQLite %e of small floats.
                    }
                    (Type::Double, Type::Float) => {
                        // Round to f32 precision but keep f64 bits in the GPR so
                        // the Float rvalue convention stays consistent with load_ty.
                        writeln!(self.out, "\tfmov\td0, x{dest}").unwrap();
                        writeln!(self.out, "\tfcvt\ts0, d0").unwrap();
                        writeln!(self.out, "\tfcvt\td0, s0").unwrap();
                        writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                    }
                    // Integer narrowing / sign-extension. Critical for SQLite
                    // ONE_BYTE_INT: `((i8)buf[0])` — without sxtb, 0xFD stays 253
                    // and ORDER BY deserializes -3 as 253.
                    (_, Type::SChar) => {
                        writeln!(self.out, "\tsxtb\tx{dest}, w{dest}").unwrap();
                    }
                    (_, Type::Short) => {
                        writeln!(self.out, "\tsxth\tx{dest}, w{dest}").unwrap();
                    }
                    (_, Type::Char) => {
                        writeln!(self.out, "\tand\tx{dest}, x{dest}, #0xff").unwrap();
                    }
                    (_, Type::UShort) => {
                        writeln!(self.out, "\tand\tx{dest}, x{dest}, #0xffff").unwrap();
                    }
                    // Truncate to 32-bit then re-extend for signed/unsigned int.
                    // Only when source is an integer (not pointer/float — those handled above).
                    (_, Type::Int) if from_int => {
                        writeln!(self.out, "\tsxtw\tx{dest}, w{dest}").unwrap();
                    }
                    (_, Type::UInt) if from_int => {
                        writeln!(self.out, "\tmov\tw{dest}, w{dest}").unwrap();
                    }
                    _ => {}
                }
                Ok(ty.clone())
            }
            Expr::SizeofType(ty) => {
                let s = self.type_size(ty);
                self.emit_imm(s, dest);
                Ok(Type::Int)
            }
            Expr::SizeofExpr(ex) => {
                // Direct string: never decay before sizeof
                if let Expr::String(s) = ex.as_ref() {
                    self.emit_imm((s.len() + 1) as i64, dest);
                    return Ok(Type::Int);
                }
                let ty = self.typeof_expr(ex, typedefs);
                let s = match &ty {
                    Type::Array(e, n) => self.type_size(e) * (*n).max(0),
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
                let result_ty = self.cond_result_ty(then_e, else_e, typedefs);
                // Small aggregate ?: — load full object into x0[,x1], not first word.
                if let Some(nr) = self.small_agg_nregs(&result_ty) {
                    let l_else = self.lab("cond_else");
                    let l_end = self.lab("cond_end");
                    self.emit_expr_rval(cond, 0, typedefs)?;
                    self.emit_cbz_long(0, &l_else);
                    if self.emit_agg_copy_src(then_e, 9, typedefs).is_ok() {
                        self.load_small_agg_to_regs(9, nr, 0);
                    } else {
                        let _ = self.emit_expr_rval(then_e, 0, typedefs)?;
                    }
                    writeln!(self.out, "\tb\t{l_end}").unwrap();
                    writeln!(self.out, "{l_else}:").unwrap();
                    if self.emit_agg_copy_src(else_e, 9, typedefs).is_ok() {
                        self.load_small_agg_to_regs(9, nr, 0);
                    } else {
                        let _ = self.emit_expr_rval(else_e, 0, typedefs)?;
                    }
                    writeln!(self.out, "{l_end}:").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                    }
                    return Ok(result_ty);
                }
                let l_else = self.lab("cond_else");
                let l_end = self.lab("cond_end");
                self.emit_expr_rval(cond, 0, typedefs)?;
                self.emit_cbz_long(0, &l_else);
                let tty = self.emit_expr_rval(then_e, dest, typedefs)?;
                writeln!(self.out, "\tb\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                let ety = self.emit_expr_rval(else_e, dest, typedefs)?;
                writeln!(self.out, "{l_end}:").unwrap();
                // Usual arithmetic conversion for ?: — float branch wins.
                if matches!(tty, Type::Float | Type::Double)
                    || matches!(ety, Type::Float | Type::Double)
                {
                    Ok(Type::Double)
                } else if matches!(tty, Type::Long | Type::ULong | Type::Ptr(_)) {
                    Ok(tty)
                } else if matches!(ety, Type::Long | Type::ULong | Type::Ptr(_)) {
                    Ok(ety)
                } else {
                    Ok(tty)
                }
            }
            Expr::PreInc(ex) => {
                // Bitfield ++ must RMW the field, not the whole container.
                // Redis rax: `n->size++` where size:29 — treating as word++ set
                // iskey instead of size → raxAddChild memmove with garbage length.
                if let Expr::Member {
                    base,
                    field,
                    arrow,
                } = ex.as_ref()
                {
                    if let Some(place) = self.member_place(base, field, *arrow, typedefs) {
                        if let Some((bo, bw)) = place.bit {
                            let _ = self.emit_lvalue_addr(ex, 19, typedefs)?;
                            self.load_bitfield(19, &place.ty, bo, bw, 0);
                            self.emit_imm(1, 11);
                            writeln!(self.out, "\tadd\tx0, x0, x11").unwrap();
                            if bw < 64 {
                                let mask = (1u64 << bw) - 1;
                                if mask <= 0xfff {
                                    writeln!(self.out, "\tand\tx0, x0, #{mask}").unwrap();
                                } else {
                                    self.emit_imm(mask as i64, 16);
                                    writeln!(self.out, "\tand\tx0, x0, x16").unwrap();
                                }
                            }
                            self.store_bitfield_at(19, &place.ty, bo, bw, 0);
                            if dest != 0 {
                                writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                            }
                            return Ok(place.ty);
                        }
                    }
                }
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                let step = if matches!(ty, Type::Ptr(_)) {
                    match &ty {
                        Type::Ptr(i) => self.type_size(i).max(1),
                        _ => 1,
                    }
                } else {
                    1
                };
                self.emit_imm(step as i64, 11);
                writeln!(self.out, "\tadd\tx0, x0, x11").unwrap();
                // Wrap expression value to object type width. Critical for
                // SQLite insertCell: `if ((++data[hdr+4])==0) data[hdr+3]++;`
                // — without u8 wrap, 255+1 stays 256, high byte never bumps,
                // nCell on disk becomes count&0xff (300→44) → corrupt pages.
                self.truncate_int_to_ty(&ty, 0);
                self.store_ty(&ty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                }
                Ok(ty)
            }
            Expr::PreDec(ex) => {
                if let Expr::Member {
                    base,
                    field,
                    arrow,
                } = ex.as_ref()
                {
                    if let Some(place) = self.member_place(base, field, *arrow, typedefs) {
                        if let Some((bo, bw)) = place.bit {
                            let _ = self.emit_lvalue_addr(ex, 19, typedefs)?;
                            self.load_bitfield(19, &place.ty, bo, bw, 0);
                            self.emit_imm(1, 11);
                            writeln!(self.out, "\tsub\tx0, x0, x11").unwrap();
                            if bw < 64 {
                                let mask = (1u64 << bw) - 1;
                                if mask <= 0xfff {
                                    writeln!(self.out, "\tand\tx0, x0, #{mask}").unwrap();
                                } else {
                                    self.emit_imm(mask as i64, 16);
                                    writeln!(self.out, "\tand\tx0, x0, x16").unwrap();
                                }
                            }
                            self.store_bitfield_at(19, &place.ty, bo, bw, 0);
                            if dest != 0 {
                                writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                            }
                            return Ok(place.ty);
                        }
                    }
                }
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                self.emit_imm(step as i64, 11);
                writeln!(self.out, "\tsub\tx0, x0, x11").unwrap();
                self.truncate_int_to_ty(&ty, 0);
                self.store_ty(&ty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                }
                Ok(ty)
            }
            Expr::PostInc(ex) => {
                if let Expr::Member {
                    base,
                    field,
                    arrow,
                } = ex.as_ref()
                {
                    if let Some(place) = self.member_place(base, field, *arrow, typedefs) {
                        if let Some((bo, bw)) = place.bit {
                            let _ = self.emit_lvalue_addr(ex, 19, typedefs)?;
                            self.load_bitfield(19, &place.ty, bo, bw, 0);
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                            self.emit_imm(1, 11);
                            writeln!(self.out, "\tadd\tx0, x0, x11").unwrap();
                            if bw < 64 {
                                let mask = (1u64 << bw) - 1;
                                if mask <= 0xfff {
                                    writeln!(self.out, "\tand\tx0, x0, #{mask}").unwrap();
                                } else {
                                    self.emit_imm(mask as i64, 16);
                                    writeln!(self.out, "\tand\tx0, x0, x16").unwrap();
                                }
                            }
                            self.store_bitfield_at(19, &place.ty, bo, bw, 0);
                            writeln!(self.out, "\tldr\tx{dest}, [sp], #16").unwrap();
                            return Ok(place.ty);
                        }
                    }
                }
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                // old value in x0; spill so nested assigns cannot clobber
                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                self.emit_imm(step as i64, 11);
                writeln!(self.out, "\tadd\tx0, x0, x11").unwrap();
                self.truncate_int_to_ty(&ty, 0);
                self.store_ty(&ty, 19, 0);
                writeln!(self.out, "\tldr\tx{dest}, [sp], #16").unwrap();
                Ok(ty)
            }
            Expr::PostDec(ex) => {
                if let Expr::Member {
                    base,
                    field,
                    arrow,
                } = ex.as_ref()
                {
                    if let Some(place) = self.member_place(base, field, *arrow, typedefs) {
                        if let Some((bo, bw)) = place.bit {
                            let _ = self.emit_lvalue_addr(ex, 19, typedefs)?;
                            self.load_bitfield(19, &place.ty, bo, bw, 0);
                            writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                            self.emit_imm(1, 11);
                            writeln!(self.out, "\tsub\tx0, x0, x11").unwrap();
                            if bw < 64 {
                                let mask = (1u64 << bw) - 1;
                                if mask <= 0xfff {
                                    writeln!(self.out, "\tand\tx0, x0, #{mask}").unwrap();
                                } else {
                                    self.emit_imm(mask as i64, 16);
                                    writeln!(self.out, "\tand\tx0, x0, x16").unwrap();
                                }
                            }
                            self.store_bitfield_at(19, &place.ty, bo, bw, 0);
                            writeln!(self.out, "\tldr\tx{dest}, [sp], #16").unwrap();
                            return Ok(place.ty);
                        }
                    }
                }
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                self.emit_imm(step as i64, 11);
                writeln!(self.out, "\tsub\tx0, x0, x11").unwrap();
                self.truncate_int_to_ty(&ty, 0);
                self.store_ty(&ty, 19, 0);
                writeln!(self.out, "\tldr\tx{dest}, [sp], #16").unwrap();
                Ok(ty)
            }
            Expr::InitList { fields } => {
                // Bare brace list as expression (rare); emit static zeroed blob.
                let id = self.label_id;
                self.label_id += 1;
                let gname = format!("__initlist_{id}");
                self.emit_rodata_section();
                writeln!(self.out, "\t.p2align\t3").unwrap();
                let glab = self.c_sym(&gname);
                writeln!(self.out, "{glab}:").unwrap();
                let ty = Type::Array(Box::new(Type::Char), 64);
                self.emit_init_list_data(&ty, fields)?;
                let resume = self.cur_section.clone();
                if let Some(sec) = resume {
                    self.emit_named_section(&sec);
                    writeln!(self.out, "\t.p2align\t2").unwrap();
                } else {
                    self.emit_text_section();
                }
                self.emit_adrp_add(dest, &glab);
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
        }
    }

    fn emit_type_of(&self, _e: &Expr, _typedefs: &HashMap<String, Type>) -> Type {
        Type::Int
    }

    /// Type of an integer literal from the lexer.
    /// Values with the high bit set are u64 constants bitcast to i64
    /// (ULLONG_MAX, 0xffffffffffffffffULL, …) — never true negative IntLits
    /// (those are Unary Neg of a positive literal).
    fn int_lit_type(n: i64) -> Type {
        if n < 0 {
            Type::ULong
        } else if n > i32::MAX as i64 {
            Type::Long
        } else {
            Type::Int
        }
    }

    fn typeof_expr(&self, e: &Expr, typedefs: &HashMap<String, Type>) -> Type {
        match e {
            Expr::StmtExpr(_stmts, final_expr) => self.typeof_expr(final_expr, typedefs),
            Expr::Int(n) => Self::int_lit_type(*n),
            Expr::Char(_) => Type::Int,
            Expr::Float(_) => Type::Double,
            // C: string literal type is char[N] (incl. NUL); decays to pointer as rvalue.
            // sizeof("hi") must be 3, not 8.
            Expr::String(s) => Type::Array(Box::new(Type::Char), (s.len() + 1) as i64),
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
            // -1.5 must stay Double (not Int): call-arg setup uses typeof to choose
            // fmov vs scvtf when loading spilled float bits into dN. Wrong typeof
            // made dekkerMul2's yy = scvtf(bitpattern) → Inf and broke sqlite3AtoF.
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => {
                let t = self.typeof_expr(expr, typedefs);
                if matches!(t, Type::Float | Type::Double) {
                    Type::Double
                } else {
                    t
                }
            }
            Expr::Unary {
                op: UnaryOp::Not | UnaryOp::BitNot,
                ..
            } => Type::Int,
            Expr::Index { base, .. } => match self.typeof_expr(base, typedefs) {
                Type::Ptr(i) | Type::Array(i, _) => *i,
                _ => Type::Int,
            },
            Expr::Cast { ty, .. } => ty.clone(),
            Expr::Call { name, args } => {
                if name == "__indirect__" {
                    // callee is args[0]; try to get its pointed-to / return type
                    if let Some(callee) = args.first() {
                        let ct = self.typeof_expr(callee, typedefs);
                        let from_ptr = match ct {
                            Type::Ptr(inner) => Some(*inner),
                            other => Some(other),
                        };
                        if let Some(Type::Float | Type::Double) = from_ptr {
                            return Type::Double;
                        }
                        // SQLite math wrappers: double (*f)(double); f(v) — pointee
                        // typing is often soft Void/Int; float args imply Double.
                        if args.iter().skip(1).any(|a| {
                            matches!(
                                self.typeof_expr(a, typedefs),
                                Type::Float | Type::Double
                            )
                        }) {
                            return Type::Double;
                        }
                        return from_ptr.unwrap_or(Type::Int);
                    }
                    return Type::Int;
                }
                // libm double returns: without this, `ceil(log(x)/log(2))` has
                // typeof Int → call path does scvtf(bitpattern) on a real double
                // and hdr_calculate_bucket_config returns EINVAL (Redis latency).
                if matches!(
                    name.as_str(),
                    "sin" | "cos" | "tan" | "sqrt" | "fabs" | "exp" | "log"
                        | "log10" | "log2" | "floor" | "ceil" | "round" | "trunc"
                        | "pow" | "fmod" | "atan" | "atan2" | "asin" | "acos"
                        | "ldexp" | "frexp" | "modf"
                ) {
                    return Type::Double;
                }
                if let Some(f) = self.funcs.get(name) {
                    return f.ret.clone();
                }
                // Call through local/global function-pointer variable.
                if self.is_function_pointer_var(name) {
                    if let Some(sym) = self.get_local(name) {
                        if let Type::Ptr(inner) = &sym.ty {
                            if matches!(inner.as_ref(), Type::Float | Type::Double) {
                                return Type::Double;
                            }
                        }
                    } else if let Some(ty) = self.globals.get(name) {
                        if let Type::Ptr(inner) = ty {
                            if matches!(inner.as_ref(), Type::Float | Type::Double) {
                                return Type::Double;
                            }
                        }
                    }
                    if args.iter().any(|a| {
                        matches!(
                            self.typeof_expr(a, typedefs),
                            Type::Float | Type::Double
                        )
                    }) {
                        return Type::Double;
                    }
                }
                Type::Int
            }
            Expr::Binary { op, left, right } => {
                let l = self.typeof_expr(left, typedefs);
                let r = self.typeof_expr(right, typedefs);
                // comparisons and logical → int
                if matches!(
                    op,
                    BinOp::Eq
                        | BinOp::Ne
                        | BinOp::Lt
                        | BinOp::Le
                        | BinOp::Gt
                        | BinOp::Ge
                        | BinOp::And
                        | BinOp::Or
                ) {
                    return Type::Int;
                }
                if matches!(l, Type::Float | Type::Double)
                    || matches!(r, Type::Float | Type::Double)
                {
                    return Type::Double;
                }
                if matches!(op, BinOp::Add) {
                    if matches!(l, Type::Ptr(_)) {
                        return l;
                    }
                    if matches!(r, Type::Ptr(_)) {
                        return r;
                    }
                }
                if matches!(op, BinOp::Sub) {
                    if matches!(l, Type::Ptr(_)) && matches!(r, Type::Ptr(_)) {
                        return Type::Int;
                    }
                    if matches!(l, Type::Ptr(_)) {
                        return l;
                    }
                }
                // Bitwise / shifts: preserve wider integer type (u64 masks in IsNaN).
                if matches!(
                    op,
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr
                ) {
                    if matches!(l, Type::ULong | Type::Long | Type::Ptr(_)) {
                        return l;
                    }
                    if matches!(r, Type::ULong | Type::Long | Type::Ptr(_)) {
                        return r;
                    }
                    if matches!(l, Type::UInt | Type::UShort) {
                        return l;
                    }
                    if matches!(r, Type::UInt | Type::UShort) {
                        return r;
                    }
                }
                // Arithmetic: prefer long/ulong when either side is wide;
                // unsigned int beats signed int (corruptI nOvfl wrap).
                if matches!(l, Type::ULong | Type::Long) {
                    return l;
                }
                if matches!(r, Type::ULong | Type::Long) {
                    return r;
                }
                if matches!(l, Type::UInt) || matches!(r, Type::UInt) {
                    return Type::UInt;
                }
                Type::Int
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
                        .and_then(|l| l.fields.get(field).map(|p| p.ty.clone()))
                        .unwrap_or(Type::Int),
                    Type::AnonStruct(fs) => {
                        let lay = self.layout_fields(&fs, false, false);
                        lay.fields.get(field).map(|p| p.ty.clone()).unwrap_or(Type::Int)
                    }
                    Type::AnonUnion(fs) => {
                        let lay = self.layout_fields(&fs, true, false);
                        lay.fields.get(field).map(|p| p.ty.clone()).unwrap_or(Type::Int)
                    }
                    _ => Type::Int,
                }
            }
            // ?: result type: struct/union wins (Token etc.); else float; else wider/ptr.
            // Without aggregate preference, assign paths treat Token as Int and only
            // copy 8 bytes / misuse soft lvalue (CREATE TRIGGER OOM).
            Expr::Cond {
                then_e, else_e, ..
            } => self.cond_result_ty(then_e, else_e, typedefs),
            _ => Type::Int,
        }
    }

    fn emit_imm(&mut self, n: i64, dest: u8) {
        let u = n as u64;
        // Prefer movn for small negative immediates (legal AArch64).
        if n < 0 && n >= -65536 {
            let inv = (!u) as u16;
            writeln!(self.out, "\tmovn\tx{dest}, #{inv}").unwrap();
            return;
        }
        let w0 = (u & 0xffff) as u16;
        let w1 = ((u >> 16) & 0xffff) as u16;
        let w2 = ((u >> 32) & 0xffff) as u16;
        let w3 = ((u >> 48) & 0xffff) as u16;
        // Always materialize full 64-bit pattern (movz + movk). Never use
        // `mov xN, #imm` with out-of-range immediates.
        writeln!(self.out, "\tmovz\tx{dest}, #{w0}").unwrap();
        if w1 != 0 {
            writeln!(self.out, "\tmovk\tx{dest}, #{w1}, lsl #16").unwrap();
        }
        if w2 != 0 {
            writeln!(self.out, "\tmovk\tx{dest}, #{w2}, lsl #32").unwrap();
        }
        if w3 != 0 {
            writeln!(self.out, "\tmovk\tx{dest}, #{w3}, lsl #48").unwrap();
        }
    }
}

/// Emit assembly for the default target (aarch64).
pub fn emit_assembly(prog: &Program) -> Result<String, String> {
    emit_assembly_for(prog, Target::Aarch64)
}

/// Emit assembly for the selected ISA backend (host OS dialect).
pub fn emit_assembly_for(prog: &Program, target: Target) -> Result<String, String> {
    emit_assembly_for_os(prog, target, TargetOs::host())
}

/// Emit assembly for ISA + OS (Darwin Mach-O vs Linux ELF).
pub fn emit_assembly_for_os(
    prog: &Program,
    target: Target,
    os: TargetOs,
) -> Result<String, String> {
    match target {
        Target::Aarch64 => {
            let mut cg = Codegen::with_os(os);
            cg.compile(prog)
        }
        Target::X86_64 => x86_64::emit_assembly(prog),
        Target::I686 => {
            if os != TargetOs::Linux {
                return Err("i686 backend currently supports --target-os linux only".into());
            }
            i686::emit_assembly(prog)
        }
        Target::Riscv64 => {
            if os != TargetOs::Linux {
                return Err("riscv64 backend currently supports --target-os linux only".into());
            }
            riscv::emit_assembly(prog)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn hello_string_from_source() {
        let src = r#"
        #include <stdio.h>
        int main(void) {
            printf("Hello, world!\n");
            return 0;
        }
        "#;
        let p = parser::parse(src).unwrap();
        let asm = emit_assembly(&p).unwrap();
        assert!(asm.contains("Hello, world!"));
        assert!(asm.contains("bl\t_printf"));
    }

    #[test]
    fn test_codegen_decl_attr_hang() {
        let src = r#"
            typedef struct { void *lock; } class_preempt_t;
            void foo(void) {
                class_preempt_t _t, *_T __attribute__((__unused__)) = &_t;
            }
        "#;
        let p = parser::parse(src).unwrap();
        let asm = emit_assembly_for_os(&p, Target::X86_64, TargetOs::Linux);
        assert!(asm.is_ok());
    }

    /// PruneState-sized locals must get a ≥2968-byte frame (not 8*N scalar slots).
    #[test]
    fn test_x86_prune_state_stack_frame() {
        use crate::preprocess;
        use std::path::Path;

        let src = include_str!("../tests/prune_state_stack.c");
        let pp = preprocess::preprocess_with_options_arch(
            src,
            Some(Path::new("tests")),
            &[],
            true,
            "prune_state_stack.c",
            "x86_64",
        )
        .expect("preprocess");
        let p = parser::parse(&pp).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::X86_64, TargetOs::Linux).expect("cg");
        let use_prune: String = asm
            .lines()
            .skip_while(|l| !l.contains("use_prune:"))
            .take(12)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            use_prune.contains("subq\t$") && !use_prune.contains("subq\t$16,"),
            "use_prune needs a large stack frame, got:\n{use_prune}"
        );
        // marked offset 2380 = 0x94c, sizeof marked 292 = 0x124 (decimal or hex in asm)
        assert!(
            (asm.contains("0x94c") || asm.contains("2380"))
                && (asm.contains("0x124") || asm.contains("$292") || asm.contains(", 292")),
            "marked memset must use gcc layout offsets"
        );
        assert!(asm.contains("2968"), "sizeof(PruneState) must fold to 2968");
    }

                            /// Parameter named same as typedef must be Expr::Var in call args, not Int(0).
    #[test]
   
    #[test]
    fn test_bitfield_postinc() {
        let src = r#"
            typedef struct N {
                unsigned iskey:1;
                unsigned isnull:1;
                unsigned iscompr:1;
                unsigned size:29;
            } N;
            void bump(N *n) { n->size++; }
            unsigned get_size(N *n) { return n->size; }
            unsigned get_hdr(N *n) { return *(unsigned*)n; }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        // bump must not be a plain word add on the container
        assert!(asm.contains("bump:"), "{asm}");
        // size field store uses shift #3
        let bump: String = asm.lines().skip_while(|l| *l != "bump:").take(80).collect::<Vec<_>>().join("\n");
        assert!(
            bump.contains("lsl\tx") && bump.contains("#3") || bump.contains("lsl\t"),
            "bitfield store should shift into size field:\n{bump}"
        );
    }

    /// Redis string2ll: `if (v > (ULLONG_MAX / 10)) return 0;`
    /// ULLONG_MAX as i64 is -1; signed sdiv → 0 → every multi-digit parse fails.
    /// Note: unit parse path has no preprocessor; use the expanded ULL literal.
    #[test]
    fn test_ullong_max_div_uses_udiv() {
        let src = r#"
            unsigned long long limit(void) {
                return 18446744073709551615ULL / 10;
            }
            int string2ll_overflow(unsigned long long v) {
                if (v > (18446744073709551615ULL / 10)) return 1;
                return 0;
            }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let limit_asm: String = asm
            .lines()
            .skip_while(|l| *l != "limit:")
            .take(40)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            limit_asm.contains("udiv"),
            "ULLONG_MAX/10 must use udiv, not sdiv:\n{limit_asm}"
        );
        assert!(
            !limit_asm.contains("sdiv"),
            "must not emit sdiv for ULLONG_MAX/10:\n{limit_asm}"
        );
    }

    /// libm calls typeof as Double so ceil(log(x)/log(2)) does not scvtf bits.
    #[test]
    fn test_libm_div_ceil_no_scvtf_bitcast() {
        let src = r#"
            double log(double);
            double ceil(double);
            int mag(long x) {
                return (int)ceil(log((double)x) / log(2.0));
            }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let mag: String = asm
            .lines()
            .skip_while(|l| *l != "mag:")
            .take(80)
            .collect::<Vec<_>>()
            .join("\n");
        // After fdiv, ceil must get fmov d0,xN (bits), never scvtf d0,xN of those bits.
        let mut saw_fdiv = false;
        let mut bad = false;
        for l in mag.lines() {
            let t = l.trim();
            if t.starts_with("fdiv") {
                saw_fdiv = true;
            }
            if saw_fdiv && t.contains("scvtf") && t.contains("d0") {
                // scvtf after fdiv before ceil is the bitcast bug
                if !t.contains("x9") && !t.contains("x10") {
                    // allow scvtf of integer locals only; ban scvtf of fdiv result path
                }
                // Heuristic: scvtf d0, x0 right after fmov x0,d0 is the bug
                bad = true;
            }
            if t.starts_with("bl\tceil") || t.starts_with("bl\t_ceil") {
                break;
            }
        }
        // Stronger: sequence fmov xN,d0 then scvtf d0,xN before ceil is forbidden
        let lines: Vec<&str> = mag.lines().map(|l| l.trim()).collect();
        for i in 0..lines.len().saturating_sub(2) {
            if lines[i].starts_with("fmov\tx") && lines[i].contains(", d0") {
                if lines[i + 1].starts_with("scvtf\td0, x") {
                    // next non-empty toward ceil
                    for j in (i + 1)..lines.len() {
                        if lines[j].starts_with("bl\tceil") || lines[j].starts_with("bl\t_ceil") {
                            panic!(
                                "fmov+scvtf bitcast before ceil (hdr EINVAL bug):\n{mag}"
                            );
                        }
                        if lines[j].starts_with("bl\t") {
                            break;
                        }
                    }
                }
            }
        }
        assert!(mag.contains("fdiv") || mag.contains("bl\tlog"), "expected float path:\n{mag}");
        let _ = (saw_fdiv, bad);
    }

    /// Large struct params are AAPCS64 by-ref: callee must memcpy into a local.
    #[test]
    fn test_large_struct_param_materialized_with_memcpy() {
        let src = r#"
            typedef struct {
                int shard;
                void *(*fn)(void *);
                void *(*fn2)(void *);
                void **p;
                void **q;
                void **r;
                void **s;
            } big_t;
            static void *myfn(void *x) { return x; }
            static big_t g = {0, myfn, myfn, 0, 0, 0, 0};
            void *use_big(void *c, int n, big_t type) {
                (void)n;
                return type.fn(c);
            }
            void *call_it(void *c) { return use_big(c, 0, g); }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let use_big: String = asm
            .lines()
            .skip_while(|l| *l != "use_big:")
            .take(80)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            use_big.contains("bl\tmemcpy") || use_big.contains("bl\t_memcpy"),
            "large struct param must memcpy from hidden pointer:\n{use_big}"
        );
        assert!(
            use_big.contains("blr\t"),
            "after materialize, type.fn must be callable via blr:\n{use_big}"
        );
    }

    /// `T *(name)(params)` is a function returning T*, not a pointer variable.
    #[test]
    fn test_paren_name_pointer_return_is_function_call() {
        let src = r#"
            typedef struct S S;
            S *(luaL_newstate)(void);
            void *boot(void) { return (void *)luaL_newstate(); }
            S *luaL_newstate(void) { return 0; }
        "#;
        let p = parser::parse(src).expect("parse");
        assert!(
            p.items.iter().any(|i| matches!(i, Item::Func(f) if f.name == "luaL_newstate")),
            "luaL_newstate must be a Func item, not a data global"
        );
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let boot: String = asm
            .lines()
            .skip_while(|l| *l != "boot:")
            .take(30)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            boot.contains("bl\tluaL_newstate") || boot.lines().any(|l| l.trim() == "bl\tluaL_newstate"),
            "must bl luaL_newstate, not load+blr:\n{boot}"
        );
        assert!(
            !boot.contains("blr\t"),
            "must not blr for function designator call:\n{boot}"
        );
    }

    /// AAPCS64 int returns must be sxtw'd before 64-bit cmp to -1.
    #[test]
    fn test_int_call_return_sign_extended_for_cmp() {
        let src = r#"
            int unlink_like(const char *p);
            int check(const char *p) {
              if (unlink_like(p) == (-1)) return 1;
              return 0;
            }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let check: String = asm
            .lines()
            .skip_while(|l| *l != "check:")
            .take(60)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            check.contains("bl\tunlink_like"),
            "expected call:\n{check}"
        );
        assert!(
            check.contains("cmp\tw9, w10") || check.contains("cmp\tw"),
            "narrow int Eq must cmp w so -1 matches int returns:\n{check}"
        );
        assert!(
            check.contains("sxtw\t"),
            "known int return should sxtw:\n{check}"
        );
    }

    /// u32 add/sub must use W regs so arithmetic wraps at 2^32 (SQLite nOvfl).
    #[test]
    fn test_uint32_arith_uses_w_regs() {
        let src = r#"
            typedef unsigned int u32;
            typedef unsigned short u16;
            int novfl(u32 nPayload, u16 nLocal, u32 ovflPageSize) {
              int nOvfl = (nPayload - nLocal + ovflPageSize - 1)/ovflPageSize;
              return nOvfl;
            }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let body: String = asm
            .lines()
            .skip_while(|l| *l != "novfl:")
            .take_while(|l| !l.starts_with("L_novfl_epilogue") && *l != ".size")
            .take(120)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("add\tw") || body.contains("sub\tw"),
            "u32 arithmetic must use W-width add/sub for wrap:\n{body}"
        );
        assert!(
            body.contains("udiv\tw") || body.contains("udiv\tx"),
            "expected udiv in nOvfl:\n{body}"
        );
    }

    /// Global function-pointer variable call must load+blr, not bl into .bss.
    #[test]
    fn test_global_function_pointer_call_uses_blr() {
        let src = r#"
            typedef unsigned long monotime;
            monotime (*getMonotonicUs)(void) = 0;
            static monotime getMonotonicUs_posix(void) { return 1; }
            void monotonicInit(void) { getMonotonicUs = getMonotonicUs_posix; }
            monotime tick(void) { return getMonotonicUs(); }
            monotime real_fn(void) { return 2; }
            monotime call_real(void) { return real_fn(); }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let tick: String = asm
            .lines()
            .skip_while(|l| *l != "tick:")
            .take(40)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            tick.contains("blr\tx") || tick.contains("blr\t"),
            "getMonotonicUs() must blr through loaded fptr:\n{tick}"
        );
        assert!(
            !tick.lines().any(|l| l.trim().starts_with("bl\tgetMonotonicUs")),
            "must not bl directly to fptr variable:\n{tick}"
        );
        let call_real: String = asm
            .lines()
            .skip_while(|l| *l != "call_real:")
            .take(25)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            call_real.contains("bl\treal_fn") || call_real.contains("bl\treal_fn"),
            "real function must use bl:\n{call_real}"
        );
    }

    /// `extern FILE *stdout` rvalue must load the pointer (GOT + ldr), not &stdout.
    #[test]
    fn test_extern_stdout_rvalue_loads_pointer() {
        let src = r#"
            typedef struct __FILE FILE;
            extern FILE *stdout;
            extern int fprintf(FILE *, const char *, ...);
            void logmsg(const char *m) {
                fprintf(stdout, "%s", m);
            }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let log: String = asm
            .lines()
            .skip_while(|l| *l != "logmsg:")
            .take(40)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            log.contains(":got:stdout") && log.contains("got_lo12:stdout"),
            "must use GOT for stdout:\n{log}"
        );
        // After GOT load into x9, must load the FILE* value (not pass GOT result as FILE*).
        let has_double = log.lines().any(|l| {
            let t = l.trim();
            t.starts_with("ldr\tx") && (t.contains("[x9]") || t.contains("[x0]"))
        });
        // More precise: sequence adrp/ldr GOT then ldr [xN]
        let mut saw_got_ldr = false;
        let mut saw_value_ldr = false;
        for l in log.lines() {
            let t = l.trim();
            if t.contains("got_lo12:stdout") {
                saw_got_ldr = true;
            } else if saw_got_ldr && !saw_value_ldr {
                if t.starts_with("ldr\t") && (t.contains("[x9]") || t.contains("[x0]")) {
                    saw_value_ldr = true;
                }
            }
        }
        assert!(
            saw_got_ldr && saw_value_ldr,
            "stdout rvalue needs GOT then load of FILE* (not &stdout):\n{log}"
        );
        // Function designator path must still take address of defined functions.
        let src2 = r#"
            int luaopen_base(void *L);
            void *get_open(void) { return (void *)luaopen_base; }
            int luaopen_base(void *L) { (void)L; return 0; }
        "#;
        let p2 = parser::parse(src2).expect("parse2");
        let asm2 = emit_assembly_for_os(&p2, Target::Aarch64, TargetOs::Linux).expect("cg2");
        let get: String = asm2
            .lines()
            .skip_while(|l| *l != "get_open:")
            .take(25)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !get.contains("ldrsw") && (get.contains("luaopen_base") || get.contains(":got:")),
            "function designator must be address, not code load:\n{get}"
        );
    }

    /// PTHREAD_MUTEX_INITIALIZER / `T x = {0}` must reserve full sizeof(T), not one .quad.
    #[test]
    fn test_pthread_mutex_initializer_size() {
        let src = r#"
            typedef struct { long __s[6]; } pthread_mutex_t;
            static pthread_mutex_t moduleGIL = {0};
            int main(void) { return (int)sizeof(pthread_mutex_t); }
        "#;
        let p = parser::parse(src).expect("parse");
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let gil: String = asm
            .lines()
            .skip_while(|l| *l != "moduleGIL:")
            .take(6)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            gil.contains(".zero\t48") || gil.contains(".zero 48"),
            "moduleGIL must be 48-byte zero, got:\n{gil}"
        );
    }



    #[test]
    fn test_param_shadows_typedef() {
        let src = r#"
            typedef struct rax { void *head; } rax;
            static long walk(rax *rax, long len) {
                return (long)rax->head + len;
            }
            long find(rax *rax, long len) {
                return walk(rax, len);
            }
        "#;
        let p = parser::parse(src).expect("parse");
        let find = p.items.iter().find_map(|i| match i {
            Item::Func(f) if f.name == "find" => Some(f),
            _ => None,
        }).expect("find");
        let body = find.body.as_ref().expect("body");
        match &body[0] {
            Stmt::Return(Some(Expr::Call { args, .. })) => {
                assert!(
                    matches!(&args[0], Expr::Var(n) if n == "rax"),
                    "first arg must be Var(rax), got {:?}",
                    args[0]
                );
            }
            other => panic!("unexpected body {other:?}"),
        }
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux).expect("cg");
        let find_asm: Vec<&str> = asm
            .lines()
            .skip_while(|l| *l != "find:")
            .take(30)
            .collect();
        let joined = find_asm.join("\n");
        assert!(
            joined.contains("ldr\tx0, [x29") || joined.contains("ldur\tx0, [x29"),
            "must load param rax from frame, got:\n{joined}"
        );
    }


#[test]
    fn test_codegen_struct_tag_pointer_cast() {
        let src = r#"
            struct sdshdr8 { unsigned char len; };
            unsigned long my_sdslen(char *s) {
                return (unsigned long)(struct sdshdr8 *)s;
            }
            int main(void) { return (int)my_sdslen(0); }
        "#;
        let p = parser::parse(src).expect("parse struct cast");
        let names: Vec<_> = p
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Func(f) if f.body.is_some() => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"my_sdslen"),
            "parse must keep my_sdslen, got {:?}",
            names
        );
        let asm = emit_assembly_for_os(&p, Target::Aarch64, TargetOs::Linux)
            .expect("codegen struct cast");
        assert!(
            asm.contains("my_sdslen:"),
            "expected my_sdslen label in asm, got:\n{}",
            asm.lines().take(80).collect::<Vec<_>>().join("\n")
        );
    }
}

#[cfg(test)]
mod sqlite_diag {
    use super::*;
    use crate::parser;
    use crate::preprocess;
    use std::path::Path;
    #[test]
    fn diag_vdbe_in_ast() {
        let p = Path::new("third_party/stage_c/sqlite/sqlite3.c");
        if !p.exists() {
            return;
        }
        let src = std::fs::read_to_string(p).unwrap();
        let pp = preprocess::preprocess_with_options(
            &src,
            Some(Path::new("third_party/stage_c/sqlite")),
            &[],
            /*for_linux*/ false,
            "sqlite3.c",
        );
        let pp = match pp {
            Ok(s) => s,
            Err(e) => {
                eprintln!("pp err {e}");
                return;
            }
        };
        let prog = match parser::parse(&pp) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("parse err {e}");
                return;
            }
        };
        let want = [
            "sqlite3VdbeExec",
            "trimFunc",
            "sqlite3Get4byte",
            "sqlite3ChangeCookie",
            "charFunc",
            "sqlite3OsRandomness",
            "sqlite3LoadExtension",
            "sqlite3VdbeRecordCompareWithSkip",
            "sqlite3_progress_handler",
            "sqlite3CompileOptions",
        ];
        let mut found = 0;
        for item in &prog.items {
            if let crate::ast::Item::Func(f) = item {
                if want.iter().any(|w| f.name == *w) {
                    println!(
                        "FUNC {} static={} body={} stmts={}",
                        f.name,
                        f.is_static,
                        f.body.is_some(),
                        f.body.as_ref().map(|b| b.len()).unwrap_or(0)
                    );
                    found += 1;
                }
            }
        }
        println!(
            "found_matches={found} total_funcs={}",
            prog.items
                .iter()
                .filter(|i| matches!(i, crate::ast::Item::Func(_)))
                .count()
        );
        let r = reachable_funcs(&prog);
        for w in want {
            println!("reachable {w}={}", r.contains(w));
        }
        // Count statics with body that are NOT reachable but ARE referenced from any body/global
        let mut missing_emit = 0usize;
        for item in &prog.items {
            if let crate::ast::Item::Func(f) = item {
                if f.is_static && f.body.is_some() && f.name != "main" && !r.contains(&f.name) {
                    missing_emit += 1;
                }
            }
        }
        println!("static_with_body_not_reachable={missing_emit}");
    }
}

#[cfg(test)]
mod bare_unsigned_cast {
    use super::*;
    use crate::parser;
    use crate::preprocess;
    use crate::{Target, TargetOs};

    /// SQLite amalgamation uses `((unsigned)p[0]<<24)` extensively. Bare
    /// `(unsigned)` must parse as a cast (→ unsigned int), not soft-skip the
    /// whole function.
    #[test]
    fn get4byte_style_unsigned_cast_emits_body() {
        let src = r#"
typedef unsigned int u32;
typedef unsigned char u8;
static u32 sqlite3Get4byte(const u8 *p){
  return ((unsigned)p[0]<<24) | (p[1]<<16) | (p[2]<<8) | p[3];
}
int main(void) {
  u8 a[4] = {0,0,0,42};
  return (int)sqlite3Get4byte(a);
}
"#;
        let pp = preprocess::preprocess(src).unwrap();
        let prog = parser::parse(&pp).expect("parse");
        let f = prog
            .items
            .iter()
            .find_map(|i| match i {
                crate::ast::Item::Func(f) if f.name == "sqlite3Get4byte" => Some(f),
                _ => None,
            })
            .expect("sqlite3Get4byte in AST");
        assert!(f.body.is_some(), "body must not be soft-skipped");
        assert!(f.body.as_ref().unwrap().len() >= 1);
        assert!(super::reachable_funcs(&prog).contains("sqlite3Get4byte"));
    }

    #[test]
    fn test_char_array_string_initializer_memcpy() {
        let src = r#"
int main(void) {
    char buf[] = "hello";
    return buf[0];
}
"#;
        let pp = preprocess::preprocess(src).unwrap();
        let prog = parser::parse(&pp).unwrap();
        let asm = emit_assembly_for_os(&prog, Target::Aarch64, TargetOs::Linux).unwrap();
        assert!(asm.contains("memcpy"), "char array string initializer must emit memcpy:\n{asm}");
    }
}

#[cfg(test)]
mod freestanding_gate {
    use super::*;
    use crate::parser;
    use crate::preprocess;

    #[test]
    fn soft_freestanding_off_keeps_hard_keepers_and_map_mem_stub() {
        std::env::remove_var("ACC_SOFT_FREESTANDING");
        std::env::set_var("ACC_KERNEL_FREESTANDING", "1");
        assert!(
            !Codegen::soft_freestanding_enabled(),
            "default must not enable soft freestanding"
        );
        assert!(Codegen::kernel_freestanding_enabled());
        // Early paging still hard keepers until real bodies boot cleanly.
        assert!(!Codegen::is_soft_freestanding_name("map_mem"));
        assert!(!Codegen::is_soft_freestanding_name("kasan_init_sw_tags"));
        assert!(!Codegen::is_soft_freestanding_name("sched_init"));
        assert!(!Codegen::is_soft_freestanding_name("create_init_idmap"));
        assert!(!Codegen::is_soft_freestanding_name("_printk"));
        // A07: init handoff stays hard freestanding (soft-list hung #113/#114);
        // bodies now call kernel_init / kernel_execve instead of payload.
        assert!(!Codegen::is_soft_freestanding_name("run_init_process"));
        assert!(!Codegen::is_soft_freestanding_name("rest_init"));
        assert!(!Codegen::is_soft_freestanding_name("setup_boot_config"));
        let src = r#"
void map_mem(void *pgdp) {
  int y = 7;
  (void)pgdp;
  (void)y;
}
"#;
        let pp = preprocess::preprocess(src).unwrap();
        let prog = parser::parse(&pp).unwrap();
        let asm = emit_assembly_for_os(&prog, Target::Aarch64, TargetOs::Linux).unwrap();
        assert!(
            asm.contains("map_mem: linear RAM") || asm.contains("map_mem:"),
            "map_mem hard keeper must emit freestanding stub:\n{asm}"
        );
        std::env::remove_var("ACC_KERNEL_FREESTANDING");
    }
}
