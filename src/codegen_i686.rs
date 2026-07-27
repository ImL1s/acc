//! i686 (IA-32) code generator — minimal ILP32 SysV emitter (agent B03 / Phase E.1).
//!
//! # Status
//! Emits enough AT&T assembly for `oracles/hello` (and similar) under Linux ELF.
//! **Not** wired into `Target` / `driver` / `main` — integrator owns that merge.
//! See `docs/notes/i686_backend.md`.
//!
//! # ABI
//! - ILP32: `int`/`long`/`pointer` = 4 bytes
//! - cdecl: all args on stack; return in `%eax`
//! - 16-byte SP alignment before `call`
//! - Linux ELF symbols (no underscore). Link with `gcc -m32 -no-pie` (absolute addrs).

use crate::ast::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

/// Pointer / register width in bytes (ILP32).
pub const PTR_SIZE: i64 = 4;
/// Stack slot alignment (bytes) before `call`.
#[allow(dead_code)]
pub const STACK_ALIGN: i64 = 16;

/// SysV i386: integer/pointer return register.
#[allow(dead_code)]
pub const RET_REG: &str = "%eax";
/// High half of 64-bit return (e.g. `long long`).
#[allow(dead_code)]
pub const RET_REG_HI: &str = "%edx";
/// Frame pointer.
#[allow(dead_code)]
pub const FP_REG: &str = "%ebp";
/// Stack pointer.
#[allow(dead_code)]
pub const SP_REG: &str = "%esp";

/// Callee-saved GPRs (must restore before `ret`).
#[allow(dead_code)]
pub const CALLEE_SAVED: [&str; 4] = ["%ebx", "%esi", "%edi", "%ebp"];

/// Linux ELF: no Mach-O underscore prefix (i686 path is ELF / qemu-i386).
fn sym(name: &str) -> String {
    name.to_string()
}

fn reg(n: u8) -> &'static str {
    match n {
        0 => "%eax",
        1 => "%ecx",
        2 => "%edx",
        3 => "%ebx",
        9 => "%ecx", // secondary temp (addr/load scratch)
        10 => "%edx",
        19 => "%ebx", // lvalue addr temp (callee-saved)
        _ => "%eax",
    }
}

fn reg_b(n: u8) -> &'static str {
    match n {
        0 => "%al",
        1 => "%cl",
        2 => "%dl",
        3 => "%bl",
        9 => "%cl",
        10 => "%dl",
        19 => "%bl",
        _ => "%al",
    }
}

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
}

#[derive(Clone)]
struct Sym {
    ty: Type,
    storage: Storage,
}

struct Codegen {
    out: String,
    strings: Vec<String>,
    layouts: HashMap<String, Layout>,
    globals: HashMap<String, Type>,
    funcs: HashMap<String, Function>,
    scopes: Vec<HashMap<String, Sym>>,
    stack_size: i64,
    label_id: usize,
    break_stack: Vec<String>,
    continue_stack: Vec<String>,
    pending_case_labs: VecDeque<String>,
    goto_labels_defined: HashSet<String>,
    func_name: String,
    ptr_relocs: Vec<(usize, String)>,
}

impl Codegen {
    fn new() -> Self {
        Self {
            out: String::new(),
            strings: Vec::new(),
            layouts: HashMap::new(),
            globals: HashMap::new(),
            funcs: HashMap::new(),
            scopes: vec![HashMap::new()],
            stack_size: 0,
            label_id: 0,
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            pending_case_labs: VecDeque::new(),
            goto_labels_defined: HashSet::new(),
            func_name: String::new(),
            ptr_relocs: Vec::new(),
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

    fn type_size(&self, ty: &Type) -> i64 {
        match ty.unqual() {
            Type::Void => 0,
            Type::Char | Type::SChar | Type::UChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt => 4,
            Type::Long | Type::ULong => 4, // ILP32
            Type::Float => 4,
            Type::Double => 8,
            Type::Ptr(_) => PTR_SIZE,
            Type::Array(e, n) => self.type_size(e) * n,
            Type::Struct(n) | Type::Union(n) => self.layouts.get(n).map(|l| l.size).unwrap_or(4),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).size,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).size,
            Type::Const(_) => unreachable!(),
        }
    }

    fn stack_slot_size(&self, ty: &Type) -> i64 {
        match ty.unqual() {
            Type::Array(e, n) => self.type_size(e) * n,
            other => self.type_size(other).max(4),
        }
    }

    fn type_align(&self, ty: &Type) -> i64 {
        match ty.unqual() {
            Type::Void => 1,
            Type::Char | Type::SChar | Type::UChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt | Type::Float | Type::Long | Type::ULong | Type::Ptr(_) => 4,
            Type::Double => 4, // i386 SysV: double align 4
            Type::Array(e, _) => self.type_align(e),
            Type::Struct(n) | Type::Union(n) => self.layouts.get(n).map(|l| l.align).unwrap_or(4),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).align,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).align,
            Type::Const(_) => unreachable!(),
        }
    }

    fn layout_fields(&self, fields: &[Field], is_union: bool, packed: bool) -> Layout {
        let mut map = HashMap::new();
        let mut off = 0i64;
        let mut max_align = 1i64;
        let mut max_size = 0i64;
        for f in fields {
            // Anonymous nested struct/union: promote inner fields (GCC layout).
            if f.name.is_empty() {
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
                        for (fnm, (foff, fty)) in &nested.fields {
                            map.insert(fnm.clone(), (*foff, fty.clone()));
                        }
                        max_size = max_size.max(nested.size);
                    } else {
                        off = Self::align_up(off, nalign);
                        for (fnm, (foff, fty)) in &nested.fields {
                            map.insert(fnm.clone(), (off + foff, fty.clone()));
                        }
                        off += nested.size;
                    }
                    continue;
                }
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
                off = Self::align_up(off, al);
                if !f.name.is_empty() {
                    map.insert(f.name.clone(), (off, f.ty.clone()));
                }
                off += sz;
            }
        }
        let final_align = if packed { 1 } else { max_align.max(1) };
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
        for _ in 0..8 {
            for (name, is_union, packed, fields) in &prog.type_layouts {
                if fields.is_empty() {
                    continue;
                }
                let lay = self.layout_fields(fields, *is_union, *packed);
                self.layouts.insert(name.clone(), lay);
            }
            for item in &prog.items {
                match item {
                    Item::StructDef { name, fields } if !fields.is_empty() => {
                        let lay = self.layout_fields(fields, false, false);
                        self.layouts.insert(name.clone(), lay);
                    }
                    Item::UnionDef { name, fields } if !fields.is_empty() => {
                        let lay = self.layout_fields(fields, true, false);
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

    fn alloc_local(&mut self, name: &str, ty: &Type) -> i64 {
        let sz = self.stack_slot_size(ty).max(4);
        self.stack_size = Self::align_up(self.stack_size + sz, 4);
        let offset = -self.stack_size;
        self.insert_local(
            name.to_string(),
            Sym {
                ty: ty.clone(),
                storage: Storage::Local { offset },
            },
        );
        offset
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
        Err(format!("i686: unknown symbol '{name}'"))
    }

    fn compile(&mut self, prog: &Program) -> Result<String, String> {
        self.out.clear();
        self.collect_layouts(prog);

        let mut typedefs = HashMap::new();
        for item in &prog.items {
            if let Item::Typedef { name, ty } = item {
                typedefs.insert(name.clone(), ty.clone());
            }
            if let Item::Func(f) = item {
                if f.body.is_some() {
                    self.funcs.insert(f.name.clone(), f.clone());
                }
            }
            if let Item::Global(g) = item {
                self.globals.insert(g.name.clone(), g.ty.clone());
            }
        }

        // Always Linux ELF dialect (qemu-i386 path).
        writeln!(self.out, "\t.text").unwrap();
        writeln!(self.out, "\t.p2align\t4, 0x90").unwrap();

        let reachable = super::reachable_funcs(prog);
        let mut emitted_syms = std::collections::HashSet::new();
        for item in &prog.items {
            if let Item::Func(f) = item {
                if f.body.is_none() {
                    continue;
                }
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
        }

        for item in &prog.items {
            if let Item::Global(g) = item {
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
                if g.init.is_none() && emitted_syms.insert(g.name.clone()) {
                    self.emit_global(g)?;
                }
            }
        }

        if !self.strings.is_empty() {
            writeln!(self.out, "\n\t.section\t.rodata").unwrap();
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

        Ok(self.out.clone())
    }


    fn emit_global(&mut self, g: &VarDecl) -> Result<(), String> {
        let size = self.type_size(&g.ty).max(1);
        let s = sym(&g.name);
        if !g.is_static {
            writeln!(self.out, "\n\t.globl\t{s}").unwrap();
        } else {
            writeln!(self.out, "").unwrap();
        }
        if let Some(Expr::Int(n) | Expr::Char(n)) = &g.init {
            writeln!(self.out, "\t.data").unwrap();
            writeln!(self.out, "\t.p2align\t2").unwrap();
            writeln!(self.out, "{s}:").unwrap();
            if size <= 1 {
                writeln!(self.out, "\t.byte\t{n}").unwrap();
            } else if size <= 2 {
                writeln!(self.out, "\t.short\t{n}").unwrap();
            } else {
                writeln!(self.out, "\t.long\t{n}").unwrap();
            }
        } else if let Some(Expr::String(st)) = &g.init {
            let id = self.intern_str(st);
            writeln!(self.out, "\t.data").unwrap();
            writeln!(self.out, "\t.p2align\t2").unwrap();
            writeln!(self.out, "{s}:").unwrap();
            writeln!(self.out, "\t.long\tl_str_{id}").unwrap();
        } else if let Some(Expr::Unary {
            op: UnaryOp::Addr,
            expr,
        }) = &g.init
        {
            if let Expr::Var(v) = expr.as_ref() {
                writeln!(self.out, "\t.data").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                writeln!(self.out, "\t.long\t{}", sym(v)).unwrap();
            } else {
                writeln!(self.out, "\t.bss").unwrap();
                writeln!(self.out, "\t.p2align\t2").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                writeln!(self.out, "\t.zero\t{size}").unwrap();
            }
        } else if let Some(Expr::InitList { fields }) = &g.init {
            writeln!(self.out, "\t.data").unwrap();
            writeln!(self.out, "\t.p2align\t2").unwrap();
            writeln!(self.out, "{s}:").unwrap();
            self.emit_init_list_data(&g.ty, fields)?;
        } else {
            writeln!(self.out, "\t.bss").unwrap();
            writeln!(self.out, "\t.p2align\t2").unwrap();
            writeln!(self.out, "{s}:").unwrap();
            writeln!(self.out, "\t.zero\t{size}").unwrap();
        }
        Ok(())
    }

    fn emit_scalar_data(&mut self, ty: &Type, e: &Expr) -> Result<(), String> {
        match e {
            Expr::Int(n) | Expr::Char(n) => {
                let sz = self.type_size(ty);
                if sz <= 1 {
                    writeln!(self.out, "\t.byte\t{n}").unwrap();
                } else if sz <= 2 {
                    writeln!(self.out, "\t.short\t{n}").unwrap();
                } else {
                    writeln!(self.out, "\t.long\t{n}").unwrap();
                }
            }
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => {
                if let Expr::Var(v) = expr.as_ref() {
                    writeln!(self.out, "\t.long\t{}", sym(v)).unwrap();
                } else {
                    let sz = self.type_size(ty).max(1);
                    writeln!(self.out, "\t.zero\t{sz}").unwrap();
                }
            }
            // Nested aggregate / array initializers (e.g. `S a[1] = {{1,{2,3}}}`).
            Expr::InitList { fields } => self.emit_init_list_data(ty, fields)?,
            _ => {
                let sz = self.type_size(ty).max(1);
                writeln!(self.out, "\t.zero\t{sz}").unwrap();
            }
        }
        Ok(())
    }

    fn emit_init_list_data(
        &mut self,
        ty: &Type,
        fields_in: &[(Option<String>, Expr)],
    ) -> Result<(), String> {
        match ty {
            Type::Struct(name) | Type::Union(name) => {
                if let Some(lay) = self.layouts.get(name).cloned() {
                    self.emit_struct_init_data(&lay, fields_in)?;
                } else {
                    writeln!(self.out, "\t.zero\t32").unwrap();
                }
            }
            Type::AnonStruct(fs) | Type::AnonUnion(fs) => {
                let is_union = matches!(ty, Type::AnonUnion(_));
                let lay = self.layout_fields(fs, is_union, false);
                self.emit_struct_init_data(&lay, fields_in)?;
            }
            Type::Array(elem, n) => {
                let mut slots: Vec<Option<&Expr>> = vec![None; (*n as usize).max(1)];
                let mut cur = 0usize;
                let mut high = 0usize;
                for (des, e) in fields_in {
                    if let Some(d) = des {
                        if let Ok(i) = d.parse::<usize>() {
                            cur = i;
                        }
                    }
                    high = high.max(cur + 1);
                    if slots.len() <= cur {
                        slots.resize(cur + 1, None);
                    }
                    slots[cur] = Some(e);
                    cur += 1;
                }
                let count = (*n as usize).max(high).max(slots.len());
                for i in 0..count {
                    if let Some(Some(e)) = slots.get(i) {
                        match e {
                            Expr::InitList { fields } => {
                                self.emit_init_list_data(elem, fields)?;
                            }
                            _ => self.emit_scalar_data(elem, e)?,
                        }
                    } else {
                        writeln!(self.out, "\t.zero\t{}", self.type_size(elem)).unwrap();
                    }
                }
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
        self.ptr_relocs.clear();
        let mut blob = vec![0u8; lay.size as usize];
        let mut positional: Vec<&Expr> = Vec::new();
        for (des, e) in fields_in {
            if des.is_none() {
                positional.push(e);
            }
        }
        let mut pos = 0usize;
        let mut field_order: Vec<(String, i64, Type)> = lay
            .fields
            .iter()
            .map(|(n, (o, t))| (n.clone(), *o, t.clone()))
            .collect();
        field_order.sort_by_key(|(_, o, _)| *o);
        let mut unique_order: Vec<(String, i64, Type)> = Vec::new();
        let mut seen_off = HashSet::new();
        for (fname, off, fty) in field_order {
            if seen_off.insert(off) {
                unique_order.push((fname, off, fty));
            }
        }
        for (fname, off, fty) in &unique_order {
            let expr = fields_in
                .iter()
                .find(|(d, _)| d.as_deref() == Some(fname.as_str()))
                .map(|(_, e)| e)
                .or_else(|| {
                    let e = positional.get(pos)?;
                    pos += 1;
                    Some(*e)
                });
            if let Some(e) = expr {
                self.write_init_expr_to_blob(&mut blob, *off as usize, fty, e)?;
            }
        }
        let mut i = 0usize;
        while i < blob.len() {
            if let Some((rel_off, name)) = self
                .ptr_relocs
                .iter()
                .find(|(o, _)| *o == i)
                .map(|(o, n)| (*o, n.clone()))
            {
                let psz = PTR_SIZE as usize;
                if rel_off > i {
                    writeln!(self.out, "\t.zero\t{}", rel_off - i).unwrap();
                    i = rel_off;
                }
                writeln!(self.out, "\t.long\t{name}").unwrap();
                i += psz;
                continue;
            }
            if i + 4 <= blob.len() {
                let w = u32::from_le_bytes(blob[i..i + 4].try_into().unwrap());
                if w != 0 || i + 4 >= blob.len() {
                    writeln!(self.out, "\t.long\t{w}").unwrap();
                } else {
                    writeln!(self.out, "\t.long\t0").unwrap();
                }
                i += 4;
            } else {
                writeln!(self.out, "\t.byte\t{}", blob[i]).unwrap();
                i += 1;
            }
        }
        Ok(())
    }

    fn write_init_expr_to_blob(
        &mut self,
        blob: &mut [u8],
        off: usize,
        fty: &Type,
        e: &Expr,
    ) -> Result<(), String> {
        match e {
            Expr::Int(n) | Expr::Char(n) => {
                let bytes = (*n as u32).to_le_bytes();
                let sz = self.type_size(fty).min(8) as usize;
                for (i, b) in bytes.iter().take(sz).enumerate() {
                    if off + i < blob.len() {
                        blob[off + i] = *b;
                    }
                }
            }
            Expr::InitList { fields } => {
                // Nested array init: `int sub[2] = {2, 3}` inside a struct.
                if let Type::Array(elem, n) = fty {
                    let esz = self.type_size(elem).max(1) as usize;
                    let count = (*n as usize).max(1);
                    let mut cur = 0usize;
                    for (des, ex) in fields {
                        if let Some(d) = des {
                            if let Ok(i) = d.parse::<usize>() {
                                cur = i;
                            }
                        }
                        if cur < count {
                            self.write_init_expr_to_blob(
                                blob,
                                off + cur * esz,
                                elem,
                                ex,
                            )?;
                        }
                        cur += 1;
                    }
                    return Ok(());
                }
                let sub_lay = match fty {
                    Type::Struct(n) | Type::Union(n) => self
                        .layouts
                        .get(n)
                        .cloned()
                        .unwrap_or(Layout {
                            size: self.type_size(fty),
                            align: 4,
                            fields: HashMap::new(),
                        }),
                    Type::AnonStruct(fs) => self.layout_fields(fs, false, false),
                    Type::AnonUnion(fs) => self.layout_fields(fs, true, false),
                    _ => Layout {
                        size: self.type_size(fty),
                        align: 4,
                        fields: HashMap::new(),
                    },
                };
                let mut sub = vec![0u8; sub_lay.size as usize];
                let mut positional: Vec<&Expr> = Vec::new();
                for (des, ex) in fields {
                    if des.is_none() {
                        positional.push(ex);
                    }
                }
                let mut pos = 0usize;
                let mut order: Vec<(String, i64, Type)> = sub_lay
                    .fields
                    .iter()
                    .map(|(n, (o, t))| (n.clone(), *o, t.clone()))
                    .collect();
                order.sort_by_key(|(_, o, _)| *o);
                for (fname, foff, ft) in &order {
                    let ex = fields
                        .iter()
                        .find(|(d, _)| d.as_deref() == Some(fname.as_str()))
                        .map(|(_, v)| v)
                        .or_else(|| {
                            let v = positional.get(pos)?;
                            pos += 1;
                            Some(*v)
                        });
                    if let Some(ex) = ex {
                        self.write_init_expr_to_blob(&mut sub, *foff as usize, ft, ex)?;
                    }
                }
                for (i, b) in sub.iter().enumerate() {
                    if off + i < blob.len() {
                        blob[off + i] = *b;
                    }
                }
            }
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => {
                if let Expr::Var(v) = expr.as_ref() {
                    self.ptr_relocs.push((off, sym(v)));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn emit_function(
        &mut self,
        f: &Function,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        self.func_name = f.name.clone();
        self.clear_locals();
        self.break_stack.clear();
        self.continue_stack.clear();
        self.pending_case_labs.clear();
        self.goto_labels_defined.clear();
        // Reserve -4(%ebp) for saved %ebx (lvalue / call scratch).
        self.stack_size = 4;

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
        let mut measure = self.scopes.clone();
        let mut measure_size = self.stack_size;
        self.measure_stmts(body, &mut measure, &mut measure_size, typedefs);
        let frame = Self::align_up(measure_size.max(4), 16);

        self.clear_locals();
        self.stack_size = 4;

        let s = sym(&f.name);
        if f.is_static {
            writeln!(self.out, "").unwrap();
        } else {
            writeln!(self.out, "\n\t.globl\t{s}").unwrap();
        }
        writeln!(self.out, "{s}:").unwrap();
        writeln!(self.out, "\tpushl\t%ebp").unwrap();
        writeln!(self.out, "\tmovl\t%esp, %ebp").unwrap();
        writeln!(self.out, "\tsubl\t${frame}, %esp").unwrap();
        writeln!(self.out, "\tmovl\t%ebx, -4(%ebp)").unwrap();

        // Incoming args: 8(%ebp)=arg0, 12(%ebp)=arg1, ...
        for (i, (pname, pty)) in f.params.iter().enumerate() {
            if pname.is_empty() {
                continue;
            }
            let pty = match pty {
                Type::Array(e, _) => Type::Ptr(e.clone()),
                other => other.clone(),
            };
            let off = self.alloc_local(pname, &pty);
            let arg_off = 8i64 + (i as i64) * 4;
            writeln!(self.out, "\tmovl\t{arg_off}(%ebp), %eax").unwrap();
            writeln!(self.out, "\tmovl\t%eax, {off}(%ebp)").unwrap();
        }

        for st in body {
            self.emit_stmt(st, typedefs)?;
        }

        writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
        let end = format!("L_{}_epilogue", f.name);
        writeln!(self.out, "{end}:").unwrap();
        writeln!(self.out, "\tmovl\t-4(%ebp), %ebx").unwrap();
        writeln!(self.out, "\tleave").unwrap();
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
                let sz = self.stack_slot_size(&ty).max(4);
                *stack = Self::align_up(*stack + sz, 4);
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
                then_b, else_b, ..
            } => {
                self.measure_stmt(then_b, locals, stack, typedefs);
                if let Some(e) = else_b {
                    self.measure_stmt(e, locals, stack, typedefs);
                }
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Label(_, body) => {
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
            | Expr::Index {
                base: left,
                index: right,
            } => {
                self.measure_expr(left, locals, stack, typedefs);
                self.measure_expr(right, locals, stack, typedefs);
            }
            Expr::Call { args, .. } => {
                for a in args {
                    self.measure_expr(a, locals, stack, typedefs);
                }
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                self.measure_expr(cond, locals, stack, typedefs);
                self.measure_expr(then_e, locals, stack, typedefs);
                self.measure_expr(else_e, locals, stack, typedefs);
            }
            Expr::Member { base, .. } => self.measure_expr(base, locals, stack, typedefs),
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
            _ => ty.clone(),
        }
    }

    fn emit_stmt(&mut self, st: &Stmt, typedefs: &HashMap<String, Type>) -> Result<(), String> {
        match st {
            Stmt::Empty | Stmt::Asm { .. } => Ok(()),
            Stmt::Block(ss) => {
                self.enter_scope();
                for s in ss {
                    self.emit_stmt(s, typedefs)?;
                }
                self.exit_scope();
                Ok(())
            }
            Stmt::Decl(d) => {
                let ty = self.expand_ty(&d.ty, typedefs);
                let off = self.alloc_local(&d.name, &ty);
                if let Some(ref init) = d.init {
                    self.emit_expr_rval(init, 0, typedefs)?;
                    self.store_at_offset(off, &ty, 0);
                }
                Ok(())
            }
            Stmt::DeclGroup(decls) => {
                for d in decls {
                    self.emit_stmt(&Stmt::Decl(d.clone()), typedefs)?;
                }
                Ok(())
            }
            Stmt::Expr(e) => {
                self.emit_expr_rval(e, 0, typedefs)?;
                Ok(())
            }
            Stmt::Return(None) => {
                writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
                writeln!(self.out, "\tjmp\tL_{}_epilogue", self.func_name).unwrap();
                Ok(())
            }
            Stmt::Return(Some(e)) => {
                self.emit_expr_rval(e, 0, typedefs)?;
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
                writeln!(self.out, "\ttestl\t%eax, %eax").unwrap();
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
                let l_cond = self.lab("while_cond");
                let l_end = self.lab("while_end");
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_cond.clone());
                writeln!(self.out, "{l_cond}:").unwrap();
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\ttestl\t%eax, %eax").unwrap();
                writeln!(self.out, "\tje\t{l_end}").unwrap();
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "\tjmp\t{l_cond}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                Ok(())
            }
            Stmt::DoWhile { body, cond } => {
                let l_body = self.lab("do_body");
                let l_cond = self.lab("do_cond");
                let l_end = self.lab("do_end");
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_cond.clone());
                writeln!(self.out, "{l_body}:").unwrap();
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "{l_cond}:").unwrap();
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\ttestl\t%eax, %eax").unwrap();
                writeln!(self.out, "\tjne\t{l_body}").unwrap();
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
                let l_cond = self.lab("for_cond");
                let l_step = self.lab("for_step");
                let l_end = self.lab("for_end");
                self.enter_scope();
                self.break_stack.push(l_end.clone());
                self.continue_stack.push(l_step.clone());
                if let Some(i) = init {
                    self.emit_stmt(i, typedefs)?;
                }
                writeln!(self.out, "{l_cond}:").unwrap();
                if let Some(c) = cond {
                    self.emit_expr_rval(c, 0, typedefs)?;
                    writeln!(self.out, "\ttestl\t%eax, %eax").unwrap();
                    writeln!(self.out, "\tje\t{l_end}").unwrap();
                }
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "{l_step}:").unwrap();
                if let Some(s) = step {
                    self.emit_expr_rval(s, 0, typedefs)?;
                }
                writeln!(self.out, "\tjmp\t{l_cond}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                self.exit_scope();
                Ok(())
            }
            Stmt::Break => {
                let lab = self
                    .break_stack
                    .last()
                    .ok_or_else(|| "i686: break outside loop".to_string())?;
                writeln!(self.out, "\tjmp\t{lab}").unwrap();
                Ok(())
            }
            Stmt::Continue => {
                let lab = self
                    .continue_stack
                    .last()
                    .ok_or_else(|| "i686: continue outside loop".to_string())?;
                writeln!(self.out, "\tjmp\t{lab}").unwrap();
                Ok(())
            }
            Stmt::Goto(name) => {
                writeln!(self.out, "\tjmp\tL_{}_goto_{name}", self.func_name).unwrap();
                Ok(())
            }
            Stmt::GotoIndirect(e) => {
                self.emit_expr_rval(e, 0, typedefs)?;
                writeln!(self.out, "\tjmp\t*%eax").unwrap();
                Ok(())
            }
            Stmt::Label(name, body) => {
                let lab = format!("L_{}_goto_{name}", self.func_name);
                if self.goto_labels_defined.insert(lab.clone()) {
                    writeln!(self.out, "{lab}:").unwrap();
                }
                self.emit_stmt(body, typedefs)
            }
            Stmt::Switch { cond, body } => {
                let l_end = self.lab("swend");
                let l_default = self.lab("swdef");
                self.break_stack.push(l_end.clone());
                let saved_cases = std::mem::take(&mut self.pending_case_labs);
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\tpushl\t%eax").unwrap();
                let mut cases: Vec<(Option<i64>, String)> = Vec::new();
                self.collect_switch_cases(body, &mut cases);
                self.pending_case_labs.clear();
                let mut has_default = false;
                let mut default_lab = l_default.clone();
                for (val, lab) in &cases {
                    if let Some(v) = val {
                        self.pending_case_labs.push_back(lab.clone());
                        writeln!(self.out, "\tmovl\t(%esp), %eax").unwrap();
                        writeln!(self.out, "\tcmpl\t${v}, %eax").unwrap();
                        writeln!(self.out, "\tje\t{lab}").unwrap();
                    } else {
                        has_default = true;
                        default_lab = lab.clone();
                    }
                }
                if has_default {
                    writeln!(self.out, "\tjmp\t{default_lab}").unwrap();
                } else {
                    writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                }
                self.emit_switch_body(body, &default_lab, typedefs)?;
                while let Some(lab) = self.pending_case_labs.pop_front() {
                    if !self.out.contains(&format!("{lab}:")) {
                        writeln!(self.out, "{lab}:").unwrap();
                    }
                }
                writeln!(self.out, "{l_end}:").unwrap();
                writeln!(self.out, "\taddl\t$4, %esp").unwrap();
                self.break_stack.pop();
                self.pending_case_labs = saved_cases;
                Ok(())
            }
            Stmt::Case { body, .. } => self.emit_stmt(body, typedefs),
            Stmt::Default(body) => self.emit_stmt(body, typedefs),
        }
    }

    fn const_i64_simple(&self, e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Binary { op, left, right } => {
                let l = self.const_i64_simple(left)?;
                let r = self.const_i64_simple(right)?;
                match op {
                    BinOp::Add => Some(l + r),
                    BinOp::Sub => Some(l - r),
                    BinOp::Mul => Some(l * r),
                    BinOp::BitOr => Some(l | r),
                    BinOp::BitAnd => Some(l & r),
                    BinOp::BitXor => Some(l ^ r),
                    BinOp::Shl => Some(l << r),
                    BinOp::Shr => Some(l >> r),
                    _ => None,
                }
            }
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(-self.const_i64_simple(expr)?),
            _ => None,
        }
    }

    fn collect_switch_cases(&mut self, st: &Stmt, out: &mut Vec<(Option<i64>, String)>) {
        match st {
            Stmt::Block(ss) => {
                for s in ss {
                    self.collect_switch_cases(s, out);
                }
            }
            Stmt::DeclGroup(_) => {}
            Stmt::Case { value, body } => {
                let lab = self.lab("case");
                out.push((self.const_i64_simple(value), lab));
                self.collect_switch_cases(body, out);
            }
            Stmt::Default(body) => {
                let lab = self.lab("swdef");
                out.push((None, lab));
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
            Stmt::Case { body, .. } => {
                let lab = self
                    .pending_case_labs
                    .pop_front()
                    .unwrap_or_else(|| self.lab("case"));
                writeln!(self.out, "{lab}:").unwrap();
                self.emit_switch_body(body, default_lab, typedefs)
            }
            Stmt::Default(body) => {
                writeln!(self.out, "{default_lab}:").unwrap();
                self.emit_switch_body(body, default_lab, typedefs)
            }
            other => self.emit_stmt(other, typedefs),
        }
    }

    fn emit_imm(&mut self, n: i64, dest: u8) {
        writeln!(self.out, "\tmovl\t${n}, {}", reg(dest)).unwrap();
    }

    fn store_at_offset(&mut self, offset: i64, ty: &Type, val_reg: u8) {
        match self.type_size(ty) {
            1 => writeln!(
                self.out,
                "\tmovb\t{}, {}(%ebp)",
                reg_b(val_reg),
                offset
            )
            .unwrap(),
            2 => writeln!(
                self.out,
                "\tmovw\t{}, {}(%ebp)",
                // low 16 of eax/ecx/...
                match val_reg {
                    0 => "%ax",
                    1 => "%cx",
                    2 => "%dx",
                    3 | 19 => "%bx",
                    _ => "%ax",
                },
                offset
            )
            .unwrap(),
            _ => writeln!(self.out, "\tmovl\t{}, {}(%ebp)", reg(val_reg), offset).unwrap(),
        }
    }

    fn load_from_offset(&mut self, offset: i64, ty: &Type, dest: u8) {
        match self.type_size(ty) {
            1 => {
                if matches!(ty, Type::SChar) {
                    writeln!(self.out, "\tmovsbl\t{}(%ebp), {}", offset, reg(dest)).unwrap();
                } else {
                    writeln!(self.out, "\tmovzbl\t{}(%ebp), {}", offset, reg(dest)).unwrap();
                }
            }
            2 => {
                if matches!(ty, Type::Short) {
                    writeln!(self.out, "\tmovswl\t{}(%ebp), {}", offset, reg(dest)).unwrap();
                } else {
                    writeln!(self.out, "\tmovzwl\t{}(%ebp), {}", offset, reg(dest)).unwrap();
                }
            }
            _ => writeln!(self.out, "\tmovl\t{}(%ebp), {}", offset, reg(dest)).unwrap(),
        }
    }

    fn emit_expr_rval(
        &mut self,
        e: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match e {
            Expr::Int(n) | Expr::Char(n) => {
                self.emit_imm(*n, dest);
                Ok(Type::Int)
            }
            Expr::Float(f) => {
                self.emit_imm(f.to_bits() as i32 as i64, dest);
                Ok(Type::Float)
            }
            Expr::String(s) => {
                let id = self.intern_str(s);
                // Absolute address — requires non-PIC link (`gcc -m32 -no-pie`).
                writeln!(self.out, "\tmovl\t$l_str_{id}, {}", reg(dest)).unwrap();
                Ok(Type::Ptr(Box::new(Type::Char)))
            }
            Expr::Var(name) => {
                let sy = match self.lookup(name) {
                    Ok(s) => s,
                    Err(_) => {
                        if self.funcs.contains_key(name) {
                            let s = sym(name);
                            writeln!(self.out, "\tmovl\t${s}, {}", reg(dest)).unwrap();
                            return Ok(Type::Ptr(Box::new(Type::Void)));
                        }
                        self.emit_imm(0, dest);
                        return Ok(Type::Int);
                    }
                };
                match &sy.ty {
                    Type::Array(elem, _) => {
                        match &sy.storage {
                            Storage::Local { offset } => {
                                writeln!(self.out, "\tleal\t{}(%ebp), {}", offset, reg(dest))
                                    .unwrap();
                            }
                            Storage::Global { name } => {
                                let s = sym(name);
                                writeln!(self.out, "\tmovl\t${s}, {}", reg(dest)).unwrap();
                            }
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
                                writeln!(self.out, "\tmovl\t{s}, {}", reg(dest)).unwrap();
                            }
                        }
                        Ok(ty.clone())
                    }
                }
            }
            Expr::Unary { op, expr } => match op {
                UnaryOp::Neg => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tnegl\t{}", reg(dest)).unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::Not => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\ttestl\t{}, {}", reg(dest), reg(dest)).unwrap();
                    writeln!(self.out, "\tsetz\t%al").unwrap();
                    writeln!(self.out, "\tmovzbl\t%al, {}", reg(dest)).unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::BitNot => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tnotl\t{}", reg(dest)).unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::Addr => {
                    self.emit_lvalue_addr(expr, dest, typedefs)?;
                    Ok(Type::Ptr(Box::new(Type::Void)))
                }
                UnaryOp::Deref => {
                    self.emit_expr_rval(expr, 9, typedefs)?;
                    let pty = self.typeof_expr(expr, typedefs);
                    match &pty {
                        Type::Ptr(inner) => match inner.as_ref() {
                            Type::Char | Type::SChar => {
                                writeln!(self.out, "\tmovzbl\t(%ecx), {}", reg(dest)).unwrap();
                            }
                            Type::Short => {
                                writeln!(self.out, "\tmovswl\t(%ecx), {}", reg(dest)).unwrap();
                            }
                            Type::UShort => {
                                writeln!(self.out, "\tmovzwl\t(%ecx), {}", reg(dest)).unwrap();
                            }
                            _ => {
                                writeln!(self.out, "\tmovl\t(%ecx), {}", reg(dest)).unwrap();
                            }
                        },
                        _ => writeln!(self.out, "\tmovl\t(%ecx), {}", reg(dest)).unwrap(),
                    }
                    Ok(match pty {
                        Type::Ptr(inner) => *inner,
                        _ => Type::Int,
                    })
                }
            },
            Expr::Binary { op, left, right } => {
                if matches!(op, BinOp::Comma) {
                    self.emit_expr_rval(left, 0, typedefs)?;
                    return self.emit_expr_rval(right, dest, typedefs);
                }
                if matches!(op, BinOp::And | BinOp::Or) {
                    let short = self.lab("short");
                    let end = self.lab("logic_end");
                    self.emit_expr_rval(left, 0, typedefs)?;
                    writeln!(self.out, "\ttestl\t%eax, %eax").unwrap();
                    if *op == BinOp::And {
                        writeln!(self.out, "\tje\t{short}").unwrap();
                    } else {
                        writeln!(self.out, "\tjne\t{short}").unwrap();
                    }
                    self.emit_expr_rval(right, 0, typedefs)?;
                    writeln!(self.out, "\ttestl\t%eax, %eax").unwrap();
                    writeln!(self.out, "\tsetne\t%al").unwrap();
                    writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
                    writeln!(self.out, "\tjmp\t{end}").unwrap();
                    writeln!(self.out, "{short}:").unwrap();
                    if *op == BinOp::And {
                        writeln!(self.out, "\txorl\t%eax, %eax").unwrap();
                    } else {
                        writeln!(self.out, "\tmovl\t$1, %eax").unwrap();
                    }
                    writeln!(self.out, "{end}:").unwrap();
                    if dest != 0 {
                        writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                self.emit_expr_rval(left, 0, typedefs)?;
                writeln!(self.out, "\tpushl\t%eax").unwrap();
                self.emit_expr_rval(right, 0, typedefs)?;
                writeln!(self.out, "\tmovl\t%eax, %ecx").unwrap();
                writeln!(self.out, "\tpopl\t%eax").unwrap();
                let lty = self.typeof_expr(left, typedefs);
                let rty = self.typeof_expr(right, typedefs);
                match op {
                    BinOp::Add => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            if esz != 1 {
                                writeln!(self.out, "\timull\t${esz}, %ecx").unwrap();
                            }
                            writeln!(self.out, "\taddl\t%ecx, %eax").unwrap();
                        } else if let Type::Ptr(inner) = rty {
                            let esz = self.type_size(&inner).max(1);
                            if esz != 1 {
                                writeln!(self.out, "\timull\t${esz}, %eax").unwrap();
                            }
                            writeln!(self.out, "\taddl\t%eax, %ecx").unwrap();
                            writeln!(self.out, "\tmovl\t%ecx, %eax").unwrap();
                        } else {
                            writeln!(self.out, "\taddl\t%ecx, %eax").unwrap();
                        }
                    }
                    BinOp::Sub => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            if matches!(rty, Type::Ptr(_)) {
                                writeln!(self.out, "\tsubl\t%ecx, %eax").unwrap();
                                if esz != 1 {
                                    writeln!(self.out, "\tmovl\t${esz}, %ecx").unwrap();
                                    writeln!(self.out, "\tcltd").unwrap();
                                    writeln!(self.out, "\tidivl\t%ecx").unwrap();
                                }
                            } else {
                                if esz != 1 {
                                    writeln!(self.out, "\timull\t${esz}, %ecx").unwrap();
                                }
                                writeln!(self.out, "\tsubl\t%ecx, %eax").unwrap();
                            }
                        } else {
                            writeln!(self.out, "\tsubl\t%ecx, %eax").unwrap();
                        }
                    }
                    BinOp::Mul => writeln!(self.out, "\timull\t%ecx, %eax").unwrap(),
                    BinOp::Div | BinOp::Mod => {
                        writeln!(self.out, "\tcltd").unwrap();
                        writeln!(self.out, "\tidivl\t%ecx").unwrap();
                        if *op == BinOp::Mod {
                            writeln!(self.out, "\tmovl\t%edx, %eax").unwrap();
                        }
                    }
                    BinOp::BitAnd => writeln!(self.out, "\tandl\t%ecx, %eax").unwrap(),
                    BinOp::BitOr => writeln!(self.out, "\torl\t%ecx, %eax").unwrap(),
                    BinOp::BitXor => writeln!(self.out, "\txorl\t%ecx, %eax").unwrap(),
                    BinOp::Shl => {
                        writeln!(self.out, "\tsall\t%cl, %eax").unwrap();
                    }
                    BinOp::Shr => {
                        writeln!(self.out, "\tsarl\t%cl, %eax").unwrap();
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        writeln!(self.out, "\tcmpl\t%ecx, %eax").unwrap();
                        let set = match op {
                            BinOp::Eq => "sete",
                            BinOp::Ne => "setne",
                            BinOp::Lt => "setl",
                            BinOp::Gt => "setg",
                            BinOp::Le => "setle",
                            BinOp::Ge => "setge",
                            _ => unreachable!(),
                        };
                        writeln!(self.out, "\t{set}\t%al").unwrap();
                        writeln!(self.out, "\tmovzbl\t%al, %eax").unwrap();
                    }
                    BinOp::And | BinOp::Or | BinOp::Comma => unreachable!(),
                }
                if dest != 0 {
                    writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                }
                Ok(Type::Int)
            }
            Expr::Assign { left, right } => {
                self.emit_expr_rval(right, 0, typedefs)?;
                writeln!(self.out, "\tpushl\t%eax").unwrap();
                self.emit_lvalue_addr(left, 19, typedefs)?;
                writeln!(self.out, "\tpopl\t%eax").unwrap();
                writeln!(self.out, "\tmovl\t%eax, (%ebx)").unwrap();
                if dest != 0 {
                    writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                }
                Ok(Type::Int)
            }
            Expr::CompoundAssign { op, left, right } => {
                let lty = self.emit_lvalue_addr(left, 19, typedefs)?;
                writeln!(self.out, "\tmovl\t(%ebx), %eax").unwrap();
                writeln!(self.out, "\tpushl\t%ebx").unwrap();
                writeln!(self.out, "\tpushl\t%eax").unwrap();
                self.emit_expr_rval(right, 0, typedefs)?;
                writeln!(self.out, "\tmovl\t%eax, %ecx").unwrap();
                writeln!(self.out, "\tpopl\t%eax").unwrap();
                match op {
                    BinOp::Add => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            if esz != 1 {
                                writeln!(self.out, "\timull\t${esz}, %ecx").unwrap();
                            }
                        }
                        writeln!(self.out, "\taddl\t%ecx, %eax").unwrap();
                    }
                    BinOp::Sub => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            if esz != 1 {
                                writeln!(self.out, "\timull\t${esz}, %ecx").unwrap();
                            }
                        }
                        writeln!(self.out, "\tsubl\t%ecx, %eax").unwrap();
                    }
                    BinOp::Mul => writeln!(self.out, "\timull\t%ecx, %eax").unwrap(),
                    BinOp::BitAnd => writeln!(self.out, "\tandl\t%ecx, %eax").unwrap(),
                    BinOp::BitOr => writeln!(self.out, "\torl\t%ecx, %eax").unwrap(),
                    BinOp::BitXor => writeln!(self.out, "\txorl\t%ecx, %eax").unwrap(),
                    _ => {
                        return Err(format!(
                            "i686: unsupported compound-assign {:?}",
                            op
                        ));
                    }
                }
                writeln!(self.out, "\tpopl\t%ebx").unwrap();
                writeln!(self.out, "\tmovl\t%eax, (%ebx)").unwrap();
                if dest != 0 {
                    writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                }
                Ok(Type::Int)
            }
            Expr::Call { name, args } => {
                if name == "__indirect__" {
                    if args.is_empty() {
                        return Err("i686: indirect call missing callee".into());
                    }
                    let (callee, real_args) = args.split_first().unwrap();
                    let callee = match callee {
                        Expr::Unary {
                            op: UnaryOp::Deref,
                            expr,
                        } => expr.as_ref(),
                        other => other,
                    };
                    let n = real_args.len() as i64;
                    let arg_bytes = n * 4;
                    let pad = (12 - (arg_bytes % 16) + 16) % 16;
                    if pad > 0 {
                        writeln!(self.out, "\tsubl\t${pad}, %esp").unwrap();
                    }
                    for a in real_args.iter().rev() {
                        self.emit_expr_rval(a, 0, typedefs)?;
                        writeln!(self.out, "\tpushl\t%eax").unwrap();
                    }
                    self.emit_expr_rval(callee, 0, typedefs)?;
                    writeln!(self.out, "\tcall\t*%eax").unwrap();
                    let total = arg_bytes + pad;
                    if total > 0 {
                        writeln!(self.out, "\taddl\t${total}, %esp").unwrap();
                    }
                    if dest != 0 {
                        writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Int);
                }
                // cdecl: push args right-to-left.
                // After our prologue (push %ebp + 16-byte frame), %esp ≡ 12 (mod 16).
                // SysV wants %esp ≡ 0 (mod 16) at `call`, so total pushed ≡ 12 (mod 16).
                let n = args.len() as i64;
                let arg_bytes = n * 4;
                let pad = (12 - (arg_bytes % 16) + 16) % 16;
                if pad > 0 {
                    writeln!(self.out, "\tsubl\t${pad}, %esp").unwrap();
                }
                for a in args.iter().rev() {
                    self.emit_expr_rval(a, 0, typedefs)?;
                    writeln!(self.out, "\tpushl\t%eax").unwrap();
                }
                let s = sym(name);
                writeln!(self.out, "\tcall\t{s}").unwrap();
                let total = arg_bytes + pad;
                if total > 0 {
                    writeln!(self.out, "\taddl\t${total}, %esp").unwrap();
                }
                if dest != 0 {
                    writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                }
                Ok(Type::Int)
            }
            Expr::Cast { ty, expr } => {
                self.emit_expr_rval(expr, dest, typedefs)?;
                Ok(ty.clone())
            }
            Expr::SizeofType(ty) => {
                self.emit_imm(self.type_size(ty), dest);
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
                writeln!(self.out, "\ttestl\t%eax, %eax").unwrap();
                writeln!(self.out, "\tje\t{l_else}").unwrap();
                self.emit_expr_rval(then_e, dest, typedefs)?;
                writeln!(self.out, "\tjmp\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                self.emit_expr_rval(else_e, dest, typedefs)?;
                writeln!(self.out, "{l_end}:").unwrap();
                Ok(Type::Int)
            }
            Expr::Index { .. } | Expr::Member { .. } => {
                let ty = self.emit_lvalue_addr(e, 9, typedefs)?;
                // Array lvalues decay to pointer (address), not a loaded value.
                // Needed for nested array members: a[0].sub[i] where .sub is T[N].
                if let Type::Array(elem, _) = ty {
                    if dest != 9 {
                        writeln!(self.out, "\tmovl\t%ecx, {}", reg(dest)).unwrap();
                    }
                    return Ok(Type::Ptr(elem));
                }
                match self.type_size(&ty) {
                    1 => writeln!(self.out, "\tmovzbl\t(%ecx), {}", reg(dest)).unwrap(),
                    2 => writeln!(self.out, "\tmovzwl\t(%ecx), {}", reg(dest)).unwrap(),
                    _ => writeln!(self.out, "\tmovl\t(%ecx), {}", reg(dest)).unwrap(),
                }
                Ok(ty)
            }
            Expr::PreInc(ex) | Expr::PreDec(ex) => {
                let is_inc = matches!(e, Expr::PreInc(_));
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                writeln!(self.out, "\tmovl\t(%ebx), %eax").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                if is_inc {
                    writeln!(self.out, "\taddl\t${step}, %eax").unwrap();
                } else {
                    writeln!(self.out, "\tsubl\t${step}, %eax").unwrap();
                }
                writeln!(self.out, "\tmovl\t%eax, (%ebx)").unwrap();
                if dest != 0 {
                    writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                }
                Ok(ty)
            }
            Expr::PostInc(ex) | Expr::PostDec(ex) => {
                let is_inc = matches!(e, Expr::PostInc(_));
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                writeln!(self.out, "\tmovl\t(%ebx), %eax").unwrap();
                writeln!(self.out, "\tpushl\t%eax").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                if is_inc {
                    writeln!(self.out, "\taddl\t${step}, %eax").unwrap();
                } else {
                    writeln!(self.out, "\tsubl\t${step}, %eax").unwrap();
                }
                writeln!(self.out, "\tmovl\t%eax, (%ebx)").unwrap();
                writeln!(self.out, "\tpopl\t%eax").unwrap();
                if dest != 0 {
                    writeln!(self.out, "\tmovl\t%eax, {}", reg(dest)).unwrap();
                }
                Ok(ty)
            }
            Expr::StmtExpr(stmts, final_expr) => {
                self.enter_scope();
                for s in stmts {
                    self.emit_stmt(s, typedefs)?;
                }
                let res = self.emit_expr_rval(final_expr, dest, typedefs);
                self.exit_scope();
                res
            }
            Expr::AddrOfLabel(label) => {
                writeln!(
                    self.out,
                    "\tmovl\t$L_{}_goto_{}, {}",
                    self.func_name, label, reg(dest)
                )
                .unwrap();
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
            Expr::InitList { .. } => {
                self.emit_imm(0, dest);
                Ok(Type::Int)
            }
        }
    }

    fn emit_lvalue_addr(
        &mut self,
        e: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match e {
            Expr::Var(name) => {
                if let Ok(sy) = self.lookup(name) {
                    match &sy.storage {
                        Storage::Local { offset } => {
                            writeln!(self.out, "\tleal\t{}(%ebp), {}", offset, reg(dest)).unwrap();
                        }
                        Storage::Global { name } => {
                            let s = sym(name);
                            writeln!(self.out, "\tmovl\t${s}, {}", reg(dest)).unwrap();
                        }
                    }
                    return Ok(sy.ty);
                }
                if self.funcs.contains_key(name) {
                    let s = sym(name);
                    writeln!(self.out, "\tmovl\t${s}, {}", reg(dest)).unwrap();
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                Err(format!("i686: unknown symbol '{name}'"))
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self.emit_expr_rval(expr, dest, typedefs),
            Expr::Index { base, index } => {
                let bty = self.emit_expr_rval(base, 0, typedefs)?;
                let elem = match &bty {
                    Type::Ptr(e) | Type::Array(e, _) => e.as_ref().clone(),
                    _ => Type::Int,
                };
                let esz = self.type_size(&elem).max(1);
                writeln!(self.out, "\tpushl\t%eax").unwrap();
                self.emit_expr_rval(index, 0, typedefs)?;
                if esz != 1 {
                    writeln!(self.out, "\timull\t${esz}, %eax").unwrap();
                }
                writeln!(self.out, "\tpopl\t%ecx").unwrap();
                writeln!(self.out, "\taddl\t%eax, %ecx").unwrap();
                if dest != 9 {
                    writeln!(self.out, "\tmovl\t%ecx, {}", reg(dest)).unwrap();
                }
                Ok(elem)
            }
            Expr::Member { base, field, arrow } => {
                if *arrow {
                    self.emit_expr_rval(base, dest, typedefs)?;
                } else {
                    self.emit_lvalue_addr(base, dest, typedefs)?;
                }
                let sty = self.typeof_expr(base, typedefs);
                let (off, fty) = match &sty {
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Struct(n) | Type::Union(n) => self
                            .layouts
                            .get(n)
                            .and_then(|l| l.fields.get(field).cloned())
                            .unwrap_or((0, Type::Int)),
                        Type::AnonStruct(fs) => {
                            let l = self.layout_fields(fs, false, false);
                            l.fields.get(field).cloned().unwrap_or((0, Type::Int))
                        }
                        _ => (0, Type::Int),
                    },
                    Type::Struct(n) | Type::Union(n) => self
                        .layouts
                        .get(n)
                        .and_then(|l| l.fields.get(field).cloned())
                        .unwrap_or((0, Type::Int)),
                    Type::AnonStruct(fs) => {
                        let l = self.layout_fields(fs, false, false);
                        l.fields.get(field).cloned().unwrap_or((0, Type::Int))
                    }
                    _ => (0, Type::Int),
                };
                if off != 0 {
                    writeln!(self.out, "\taddl\t${off}, {}", reg(dest)).unwrap();
                }
                Ok(fty)
            }
            Expr::Cast { expr, ty } => {
                self.emit_lvalue_addr(expr, dest, typedefs)?;
                Ok(ty.clone())
            }
            _ => Err(format!("i686: not an lvalue: {e:?}")),
        }
    }

    fn typeof_expr(&self, e: &Expr, typedefs: &HashMap<String, Type>) -> Type {
        match e {
            Expr::Int(_) | Expr::Char(_) => Type::Int,
            Expr::Float(_) => Type::Double,
            Expr::String(_) => Type::Ptr(Box::new(Type::Char)),
            Expr::Var(name) => self
                .lookup(name)
                .map(|s| s.ty)
                .unwrap_or(Type::Int),
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => Type::Ptr(Box::new(self.typeof_expr(expr, typedefs))),
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => match self.typeof_expr(expr, typedefs) {
                Type::Ptr(t) => *t,
                _ => Type::Int,
            },
            Expr::Cast { ty, .. } => ty.clone(),
            Expr::Index { base, .. } => match self.typeof_expr(base, typedefs) {
                Type::Ptr(e) | Type::Array(e, _) => *e,
                _ => Type::Int,
            },
            Expr::Member { base, field, arrow } => {
                let sty = if *arrow {
                    match self.typeof_expr(base, typedefs) {
                        Type::Ptr(t) => *t,
                        other => other,
                    }
                } else {
                    self.typeof_expr(base, typedefs)
                };
                match sty {
                    Type::Struct(n) | Type::Union(n) => self
                        .layouts
                        .get(&n)
                        .and_then(|l| l.fields.get(field).map(|(_, t)| t.clone()))
                        .unwrap_or(Type::Int),
                    Type::AnonStruct(fs) => {
                        let l = self.layout_fields(&fs, false, false);
                        l.fields
                            .get(field)
                            .map(|(_, t)| t.clone())
                            .unwrap_or(Type::Int)
                    }
                    _ => Type::Int,
                }
            }
            _ => Type::Int,
        }
    }
}

/// Emit i686 AT&T assembly for `prog` (Linux ELF / qemu-i386).
///
/// Signature matches `codegen_x86_64::emit_assembly` for drop-in dispatch.
pub fn emit_assembly(prog: &Program) -> Result<String, String> {
    let mut cg = Codegen::new();
    cg.compile(prog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn abi_constants() {
        assert_eq!(PTR_SIZE, 4);
        assert_eq!(STACK_ALIGN, 16);
        assert_eq!(RET_REG, "%eax");
    }

    #[test]
    fn nested_array_member_index_decays_to_addr() {
        // Regression: Stage A 00091 — `a[0].sub[1]` must not load `.sub` as a
        // pointer (array decay). Expect scaled index off the member address.
        let src = r#"
        typedef struct { int v; int sub[2]; } S;
        S a[1] = {{1, {2, 3}}};
        int main(void) { return a[0].sub[1]; }
        "#;
        let p = parser::parse(src).unwrap();
        let asm = emit_assembly(&p).expect("i686 emit");
        // After getting &a[0].sub, should add scaled index then load once —
        // not `movl (%ecx), %eax` then use eax as a base pointer.
        assert!(asm.contains("imull\t$4"), "{asm}");
        // Data must be present
        assert!(asm.contains(".long\t3"), "{asm}");
    }

    #[test]
    fn indirect_call_emits_call_star_eax() {
        // Regression: Stage A 00087 — must not emit `call __indirect__`.
        let src = r#"
        struct S { int (*fptr)(); };
        int foo(void) { return 0; }
        int main(void) {
            struct S v;
            v.fptr = foo;
            return v.fptr();
        }
        "#;
        let p = parser::parse(src).unwrap();
        let asm = emit_assembly(&p).expect("i686 emit");
        assert!(asm.contains("call\t*%eax"), "{asm}");
        assert!(!asm.contains("call\t__indirect__"), "{asm}");
    }

    #[test]
    fn hello_emits_printf_and_string() {
        let src = r#"
        int printf(const char*, ...);
        int main(void) {
            printf("Hello, world!\n");
            return 0;
        }
        "#;
        let p = parser::parse(src).unwrap();
        let asm = emit_assembly(&p).expect("i686 emit");
        assert!(asm.contains("Hello, world!"), "{asm}");
        assert!(asm.contains("call\tprintf"), "{asm}");
        assert!(asm.contains("pushl\t%ebp"), "{asm}");
        assert!(asm.contains(".globl\tmain"), "{asm}");
    }

    #[test]
    fn return_constant() {
        let src = "int main(void) { return 42; }";
        let p = parser::parse(src).unwrap();
        let asm = emit_assembly(&p).expect("emit");
        assert!(asm.contains("$42") || asm.contains("movl\t$42"), "{asm}");
    }

    /// Writes `scratch/hello_i686.s` from `oracles/hello/main.c` for Docker qemu-i386 verify.
    #[test]
    fn write_hello_oracle_asm() {
        let src = std::fs::read_to_string("oracles/hello/main.c")
            .expect("oracles/hello/main.c (run from crate root)");
        let pp = crate::preprocess::preprocess_with_options_arch(
            &src,
            None,
            &[],
            true, // Linux soft headers
            "main.c",
            "i686",
        )
        .expect("preprocess");
        let p = parser::parse(&pp).expect("parse");
        let asm = emit_assembly(&p).expect("i686 emit hello");
        std::fs::create_dir_all("scratch").ok();
        std::fs::write("scratch/hello_i686.s", &asm).expect("write scratch/hello_i686.s");
        assert!(asm.contains("Hello, world!"), "{asm}");
        assert!(asm.contains("call\tprintf"), "{asm}");
    }
}
