//! RISC-V 64 (LP64 / rv64gc) code generator — Stage A subset.
//!
//! Linux ELF only. See `docs/notes/riscv64_backend.md`.

#![allow(dead_code)]

use crate::ast::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

fn sym(name: &str) -> String {
    name.to_string()
}

const ARG_REGS: [&str; 8] = ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"];
const TEMP_REGS: [&str; 7] = ["t0", "t1", "t2", "t3", "t4", "t5", "t6"];

mod lp64 {
    pub const CHAR: i64 = 1;
    pub const SHORT: i64 = 2;
    pub const INT: i64 = 4;
    pub const LONG: i64 = 8;
    pub const PTR: i64 = 8;
    pub const FLOAT: i64 = 4;
    pub const DOUBLE: i64 = 8;
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
    scopes: Vec<HashMap<String, Sym>>,
    stack_size: i64,
    label_id: usize,
    func_name: String,
    funcs: HashMap<String, Function>,
    break_stack: Vec<String>,
    continue_stack: Vec<String>,
    pending_case_labs: VecDeque<String>,
    goto_labels_defined: HashSet<String>,
    ptr_relocs: Vec<(usize, String)>,
}

impl Codegen {
    fn new() -> Self {
        Self {
            out: String::new(),
            strings: Vec::new(),
            layouts: HashMap::new(),
            globals: HashMap::new(),
            scopes: vec![HashMap::new()],
            stack_size: 0,
            label_id: 0,
            func_name: String::new(),
            funcs: HashMap::new(),
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            pending_case_labs: VecDeque::new(),
            goto_labels_defined: HashSet::new(),
            ptr_relocs: Vec::new(),
        }
    }

    fn clear_locals(&mut self) {
        self.scopes = vec![HashMap::new()];
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

    fn lab(&mut self, p: &str) -> String {
        let id = self.label_id;
        self.label_id += 1;
        format!(".L_{}_{}_{}", self.func_name, p, id)
    }

    fn goto_lab(&self, name: &str) -> String {
        format!(".L_{}_goto_{name}", self.func_name)
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
            Type::Char | Type::SChar => lp64::CHAR,
            Type::Short | Type::UShort => lp64::SHORT,
            Type::Int | Type::UInt => lp64::INT,
            Type::Long | Type::ULong => lp64::LONG,
            Type::Float => lp64::FLOAT,
            Type::Double => lp64::DOUBLE,
            Type::Ptr(_) => lp64::PTR,
            Type::Array(e, n) => self.type_size(e) * n,
            Type::Struct(n) | Type::Union(n) => self.layouts.get(n).map(|l| l.size).unwrap_or(8),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).size,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).size,
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
            Type::Struct(n) | Type::Union(n) => self.layouts.get(n).map(|l| l.align).unwrap_or(8),
            Type::AnonStruct(fs) => self.layout_fields(fs, false, false).align,
            Type::AnonUnion(fs) => self.layout_fields(fs, true, false).align,
        }
    }

    fn layout_fields(&self, fields: &[Field], is_union: bool, packed: bool) -> Layout {
        let mut map = HashMap::new();
        let mut off = 0i64;
        let mut max_align = 1i64;
        let mut max_size = 0i64;
        for f in fields {
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

    fn stack_slot_size(&self, ty: &Type) -> i64 {
        match ty {
            Type::Array(e, n) => self.type_size(e) * n,
            other => self.type_size(other).max(8),
        }
    }

    fn alloc_local(&mut self, name: &str, ty: &Type) -> i64 {
        let sz = self.stack_slot_size(ty);
        self.stack_size = Self::align_up(self.stack_size + sz, 8);
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
        Err(format!("riscv64: unknown symbol '{name}'"))
    }

    fn compile(&mut self, prog: &Program) -> Result<String, String> {
        self.out.clear();
        self.strings.clear();
        self.funcs.clear();
        self.layouts.clear();
        self.globals.clear();
        self.collect_layouts(prog);

        let typedefs: HashMap<String, Type> = prog
            .items
            .iter()
            .filter_map(|item| {
                if let Item::Typedef { name, ty } = item {
                    Some((name.clone(), ty.clone()))
                } else {
                    None
                }
            })
            .collect();

        for item in &prog.items {
            if let Item::Func(f) = item {
                if f.body.is_some() {
                    self.funcs.insert(f.name.clone(), f.clone());
                }
            }
            if let Item::Global(g) = item {
                self.globals.insert(g.name.clone(), g.ty.clone());
            }
        }

        writeln!(self.out, "\t.text").unwrap();
        writeln!(self.out, "\t.align\t1").unwrap();

        let mut emitted = HashSet::new();
        for item in &prog.items {
            if let Item::Func(f) = item {
                if f.body.is_none() {
                    continue;
                }
                if !emitted.insert(f.name.clone()) {
                    continue;
                }
                self.emit_function(f, &typedefs)?;
            }
        }

        for item in &prog.items {
            if let Item::Global(g) = item {
                if g.is_extern && g.init.is_none() {
                    continue;
                }
                if !emitted.insert(g.name.clone()) {
                    continue;
                }
                self.emit_global(g)?;
            }
        }

        if !self.strings.is_empty() {
            writeln!(self.out, "\n\t.section\t.rodata").unwrap();
            for (i, s) in self.strings.iter().enumerate() {
                writeln!(self.out, ".Lstr_{i}:").unwrap();
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
        match &g.init {
            Some(Expr::Int(n) | Expr::Char(n)) => {
                writeln!(self.out, "\t.data").unwrap();
                writeln!(self.out, "\t.align\t3").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                if size <= 4 {
                    writeln!(self.out, "\t.word\t{n}").unwrap();
                } else {
                    writeln!(self.out, "\t.dword\t{n}").unwrap();
                }
            }
            Some(Expr::InitList { fields }) => {
                writeln!(self.out, "\t.data").unwrap();
                writeln!(self.out, "\t.align\t3").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                self.emit_init_list_data(&g.ty, fields)?;
            }
            Some(Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            }) if matches!(expr.as_ref(), Expr::Var(_)) => {
                if let Expr::Var(v) = expr.as_ref() {
                    writeln!(self.out, "\t.data").unwrap();
                    writeln!(self.out, "\t.align\t3").unwrap();
                    writeln!(self.out, "{s}:").unwrap();
                    writeln!(self.out, "\t.dword\t{}", sym(v)).unwrap();
                } else {
                    unreachable!()
                }
            }
            _ => {
                writeln!(self.out, "\t.bss").unwrap();
                writeln!(self.out, "\t.align\t3").unwrap();
                writeln!(self.out, "{s}:").unwrap();
                writeln!(self.out, "\t.zero\t{size}").unwrap();
            }
        }
        Ok(())
    }

    fn emit_scalar_data(&mut self, ty: &Type, e: &Expr) -> Result<(), String> {
        match e {
            Expr::Int(n) | Expr::Char(n) => {
                let sz = self.type_size(ty);
                if sz <= 4 {
                    writeln!(self.out, "\t.word\t{n}").unwrap();
                } else {
                    writeln!(self.out, "\t.dword\t{n}").unwrap();
                }
            }
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } if matches!(expr.as_ref(), Expr::Var(_)) => {
                if let Expr::Var(v) = expr.as_ref() {
                    writeln!(self.out, "\t.dword\t{}", sym(v)).unwrap();
                }
            }
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
                    writeln!(self.out, "\t.zero\t64").unwrap();
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
        // Positional init uses one slot per distinct offset (union members share one).
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
                if rel_off > i {
                    writeln!(self.out, "\t.zero\t{}", rel_off - i).unwrap();
                    i = rel_off;
                }
                writeln!(self.out, "\t.dword\t{name}").unwrap();
                i += lp64::PTR as usize;
                continue;
            }
            if i + 4 <= blob.len() {
                let w = u32::from_le_bytes(blob[i..i + 4].try_into().unwrap());
                writeln!(self.out, "\t.word\t{w}").unwrap();
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
                // Nested array initializers: `int sub[2] = {2, 3}` inside a struct.
                if let Type::Array(elem, n) = fty {
                    let esz = self.type_size(elem).max(1) as usize;
                    let mut cur = 0i64;
                    for (des, ex) in fields {
                        if let Some(d) = des {
                            if let Ok(i) = d.parse::<i64>() {
                                cur = i;
                            }
                        }
                        if cur >= 0 && cur < *n {
                            let eoff = off + (cur as usize) * esz;
                            self.write_init_expr_to_blob(blob, eoff, elem, ex)?;
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
                            align: 8,
                            fields: HashMap::new(),
                        }),
                    Type::AnonStruct(fs) => self.layout_fields(fs, false, false),
                    Type::AnonUnion(fs) => self.layout_fields(fs, true, false),
                    _ => Layout {
                        size: self.type_size(fty),
                        align: 8,
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
        self.goto_labels_defined.clear();
        self.stack_size = 16;

        let body = f.body.as_ref().ok_or_else(|| "no body".to_string())?;

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
        self.measure_stmts(body);
        let frame = Self::align_up(self.stack_size, 16);

        self.clear_locals();
        self.stack_size = 16;

        let s = sym(&f.name);
        if !f.is_static {
            writeln!(self.out, "\n\t.globl\t{s}").unwrap();
        } else {
            writeln!(self.out, "").unwrap();
        }
        writeln!(self.out, "{s}:").unwrap();
        self.emit_sp_add(-frame);
        self.emit_sd_sp("ra", frame - 8);
        self.emit_sd_sp("s0", frame - 16);
        self.emit_set_s0_from_sp(frame);

        for (i, (pname, pty)) in f.params.iter().enumerate() {
            if pname.is_empty() {
                continue;
            }
            let pty = match pty {
                Type::Array(e, _) => Type::Ptr(e.clone()),
                other => other.clone(),
            };
            let off = self.alloc_local(pname, &pty);
            if i < 8 {
                self.emit_sd_s0(ARG_REGS[i], off);
            } else {
                return Err(format!(
                    "riscv64: >8 args not supported yet (fn {})",
                    f.name
                ));
            }
        }

        for st in body {
            self.emit_stmt(st, typedefs)?;
        }

        writeln!(self.out, "\tli\ta0, 0").unwrap();
        let end = format!(".L_{}_epilogue", f.name);
        writeln!(self.out, "{end}:").unwrap();
        // FP-based epilogue: discard mid-function SP adjusts (switch/binop spills)
        // so goto/return out of nested switches cannot leave SP unbalanced.
        self.emit_ld_s0("ra", -8);
        self.emit_ld_s0("t6", -16);
        writeln!(self.out, "\tmv\tsp, s0").unwrap();
        writeln!(self.out, "\tmv\ts0, t6").unwrap();
        writeln!(self.out, "\tret").unwrap();
        Ok(())
    }

    fn measure_stmts(&mut self, stmts: &[Stmt]) {
        for st in stmts {
            self.measure_stmt(st);
        }
    }

    fn measure_stmt(&mut self, st: &Stmt) {
        match st {
            Stmt::Decl(d) => {
                let _ = self.alloc_local(&d.name, &d.ty);
            }
            Stmt::DeclGroup(decls) => {
                for d in decls {
                    let _ = self.alloc_local(&d.name, &d.ty);
                }
            }
            Stmt::Block(ss) => {
                self.enter_scope();
                self.measure_stmts(ss);
                self.exit_scope();
            }
            Stmt::If {
                then_b, else_b, ..
            } => {
                self.measure_stmt(then_b);
                if let Some(e) = else_b {
                    self.measure_stmt(e);
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::Label(_, body)
            | Stmt::Case { body, .. }
            | Stmt::Default(body) => {
                self.measure_stmt(body);
            }
            Stmt::Switch { body, .. } => self.measure_stmt(body),
            Stmt::For { init, body, .. } => {
                self.enter_scope();
                if let Some(i) = init {
                    self.measure_stmt(i);
                }
                self.measure_stmt(body);
                self.exit_scope();
            }
            _ => {}
        }
    }

    fn emit_stmt(
        &mut self,
        st: &Stmt,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        match st {
            Stmt::Empty => Ok(()),
            Stmt::Block(ss) => {
                self.enter_scope();
                for s in ss {
                    self.emit_stmt(s, typedefs)?;
                }
                self.exit_scope();
                Ok(())
            }
            Stmt::Decl(d) => self.emit_decl(d),
            Stmt::DeclGroup(decls) => {
                for d in decls {
                    self.emit_decl(d)?;
                }
                Ok(())
            }
            Stmt::Expr(e) => {
                self.emit_expr_rval(e, 0, typedefs)?;
                Ok(())
            }
            Stmt::Return(None) => {
                writeln!(self.out, "\tli\ta0, 0").unwrap();
                writeln!(self.out, "\tj\t.L_{}_epilogue", self.func_name).unwrap();
                Ok(())
            }
            Stmt::Return(Some(e)) => {
                self.emit_expr_rval(e, 0, typedefs)?;
                writeln!(self.out, "\tj\t.L_{}_epilogue", self.func_name).unwrap();
                Ok(())
            }
            Stmt::If {
                cond,
                then_b,
                else_b,
            } => {
                let else_l = self.lab("else");
                let end_l = self.lab("endif");
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\tbeqz\ta0, {else_l}").unwrap();
                self.emit_stmt(then_b, typedefs)?;
                writeln!(self.out, "\tj\t{end_l}").unwrap();
                writeln!(self.out, "{else_l}:").unwrap();
                if let Some(e) = else_b {
                    self.emit_stmt(e, typedefs)?;
                }
                writeln!(self.out, "{end_l}:").unwrap();
                Ok(())
            }
            Stmt::While { cond, body } => {
                let top = self.lab("while");
                let end = self.lab("endwhile");
                self.break_stack.push(end.clone());
                self.continue_stack.push(top.clone());
                writeln!(self.out, "{top}:").unwrap();
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\tbeqz\ta0, {end}").unwrap();
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "\tj\t{top}").unwrap();
                writeln!(self.out, "{end}:").unwrap();
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
                writeln!(self.out, "\tbnez\ta0, {l_body}").unwrap();
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
                if let Some(i) = init {
                    self.emit_stmt(i, typedefs)?;
                }
                let top = self.lab("for");
                let step_l = self.lab("for_step");
                let end = self.lab("endfor");
                self.break_stack.push(end.clone());
                self.continue_stack.push(step_l.clone());
                writeln!(self.out, "{top}:").unwrap();
                if let Some(c) = cond {
                    self.emit_expr_rval(c, 0, typedefs)?;
                    writeln!(self.out, "\tbeqz\ta0, {end}").unwrap();
                }
                self.emit_stmt(body, typedefs)?;
                writeln!(self.out, "{step_l}:").unwrap();
                if let Some(s) = step {
                    self.emit_expr_rval(s, 0, typedefs)?;
                }
                writeln!(self.out, "\tj\t{top}").unwrap();
                writeln!(self.out, "{end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                self.exit_scope();
                Ok(())
            }
            Stmt::Break => {
                let lab = self
                    .break_stack
                    .last()
                    .ok_or_else(|| "riscv64: break outside loop".to_string())?;
                writeln!(self.out, "\tj\t{lab}").unwrap();
                Ok(())
            }
            Stmt::Continue => {
                let lab = self
                    .continue_stack
                    .last()
                    .ok_or_else(|| "riscv64: continue outside loop".to_string())?;
                writeln!(self.out, "\tj\t{lab}").unwrap();
                Ok(())
            }
            Stmt::Goto(name) => {
                writeln!(self.out, "\tj\t{}", self.goto_lab(name)).unwrap();
                Ok(())
            }
            Stmt::GotoIndirect(e) => {
                self.emit_expr_rval(e, 0, typedefs)?;
                // a0 holds address; jalr zero, 0(a0)
                writeln!(self.out, "\tjr\ta0").unwrap();
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
                let saved_cases = std::mem::take(&mut self.pending_case_labs);
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\taddi\tsp, sp, -16").unwrap();
                writeln!(self.out, "\tsd\ta0, 0(sp)").unwrap();
                let mut cases: Vec<(Option<i64>, String)> = Vec::new();
                self.collect_switch_cases(body, &mut cases);
                self.pending_case_labs.clear();
                let mut has_default = false;
                let mut default_lab = l_default.clone();
                for (val, lab) in &cases {
                    if let Some(v) = val {
                        self.pending_case_labs.push_back(lab.clone());
                        writeln!(self.out, "\tld\ta0, 0(sp)").unwrap();
                        writeln!(self.out, "\tli\ta1, {v}").unwrap();
                        writeln!(self.out, "\tbeq\ta0, a1, {lab}").unwrap();
                    } else {
                        has_default = true;
                        default_lab = lab.clone();
                    }
                }
                if has_default {
                    writeln!(self.out, "\tj\t{default_lab}").unwrap();
                } else {
                    writeln!(self.out, "\tj\t{l_end}").unwrap();
                }
                self.emit_switch_body(body, &default_lab, typedefs)?;
                while let Some(lab) = self.pending_case_labs.pop_front() {
                    if !self.out.contains(&format!("{lab}:")) {
                        writeln!(self.out, "{lab}:").unwrap();
                    }
                }
                writeln!(self.out, "{l_end}:").unwrap();
                writeln!(self.out, "\taddi\tsp, sp, 16").unwrap();
                self.break_stack.pop();
                self.pending_case_labs = saved_cases;
                Ok(())
            }
            Stmt::Case { body, .. } => self.emit_stmt(body, typedefs),
            Stmt::Default(body) => self.emit_stmt(body, typedefs),
            other => Err(format!(
                "riscv64: unsupported stmt variant (minimal backend): {other:?}"
            )),
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
                let v = self.const_i64_simple(value);
                out.push((v, lab));
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
            Expr::Unary { op: UnaryOp::Neg, expr } => Some(-self.const_i64_simple(expr)?),
            _ => None,
        }
    }

    fn emit_decl(&mut self, d: &VarDecl) -> Result<(), String> {
        let off = self.alloc_local(&d.name, &d.ty);
        if let Some(init) = &d.init {
            self.emit_expr_rval(init, 0, &HashMap::new())?;
            self.store_local(off, &d.ty, 0)?;
        }
        Ok(())
    }

    /// RISC-V `addi` immediate is 12-bit signed; adjust SP in steps.
    fn emit_sp_add(&mut self, delta: i64) {
        if delta == 0 {
            return;
        }
        let mut rem = delta;
        while rem <= -2048 {
            writeln!(self.out, "\taddi\tsp, sp, -2048").unwrap();
            rem += 2048;
        }
        while rem >= 2048 {
            writeln!(self.out, "\taddi\tsp, sp, 2047").unwrap();
            rem -= 2047;
        }
        if rem != 0 {
            writeln!(self.out, "\taddi\tsp, sp, {rem}").unwrap();
        }
    }

    fn emit_sd_sp(&mut self, reg: &str, off: i64) {
        if (-2048..=2047).contains(&off) {
            writeln!(self.out, "\tsd\t{reg}, {off}(sp)").unwrap();
        } else {
            writeln!(self.out, "\tli\tt6, {off}").unwrap();
            writeln!(self.out, "\tadd\tt6, sp, t6").unwrap();
            writeln!(self.out, "\tsd\t{reg}, 0(t6)").unwrap();
        }
    }

    fn emit_ld_sp(&mut self, reg: &str, off: i64) {
        if (-2048..=2047).contains(&off) {
            writeln!(self.out, "\tld\t{reg}, {off}(sp)").unwrap();
        } else {
            writeln!(self.out, "\tli\tt6, {off}").unwrap();
            writeln!(self.out, "\tadd\tt6, sp, t6").unwrap();
            writeln!(self.out, "\tld\t{reg}, 0(t6)").unwrap();
        }
    }

    fn emit_set_s0_from_sp(&mut self, frame: i64) {
        if (-2048..=2047).contains(&frame) {
            writeln!(self.out, "\taddi\ts0, sp, {frame}").unwrap();
        } else {
            writeln!(self.out, "\tli\tt6, {frame}").unwrap();
            writeln!(self.out, "\tadd\ts0, sp, t6").unwrap();
        }
    }

    /// `ld/sd/addi` vs `s0` with out-of-range (>12-bit) offset expansion.
    fn emit_addi_s0(&mut self, rd: &str, off: i64) {
        if (-2048..=2047).contains(&off) {
            writeln!(self.out, "\taddi\t{rd}, s0, {off}").unwrap();
        } else {
            writeln!(self.out, "\tli\tt6, {off}").unwrap();
            writeln!(self.out, "\tadd\t{rd}, s0, t6").unwrap();
        }
    }

    fn emit_sd_s0(&mut self, reg: &str, off: i64) {
        if (-2048..=2047).contains(&off) {
            writeln!(self.out, "\tsd\t{reg}, {off}(s0)").unwrap();
        } else {
            writeln!(self.out, "\tli\tt6, {off}").unwrap();
            writeln!(self.out, "\tadd\tt6, s0, t6").unwrap();
            writeln!(self.out, "\tsd\t{reg}, 0(t6)").unwrap();
        }
    }

    fn emit_ld_s0(&mut self, reg: &str, off: i64) {
        if (-2048..=2047).contains(&off) {
            writeln!(self.out, "\tld\t{reg}, {off}(s0)").unwrap();
        } else {
            writeln!(self.out, "\tli\tt6, {off}").unwrap();
            writeln!(self.out, "\tadd\tt6, s0, t6").unwrap();
            writeln!(self.out, "\tld\t{reg}, 0(t6)").unwrap();
        }
    }

    fn emit_store_s0(&mut self, reg: &str, off: i64, sz: i64) {
        let opc = match sz {
            1 => "sb",
            2 => "sh",
            4 => "sw",
            _ => "sd",
        };
        if (-2048..=2047).contains(&off) {
            writeln!(self.out, "\t{opc}\t{reg}, {off}(s0)").unwrap();
        } else {
            // Avoid clobbering `reg` if it is t6; use t5 as address scratch.
            let scratch = if reg == "t6" { "t5" } else { "t6" };
            writeln!(self.out, "\tli\t{scratch}, {off}").unwrap();
            writeln!(self.out, "\tadd\t{scratch}, s0, {scratch}").unwrap();
            writeln!(self.out, "\t{opc}\t{reg}, 0({scratch})").unwrap();
        }
    }

    fn emit_load_s0(&mut self, reg: &str, off: i64, opc: &str) {
        if (-2048..=2047).contains(&off) {
            writeln!(self.out, "\t{opc}\t{reg}, {off}(s0)").unwrap();
        } else {
            let scratch = if reg == "t6" { "t5" } else { "t6" };
            writeln!(self.out, "\tli\t{scratch}, {off}").unwrap();
            writeln!(self.out, "\tadd\t{scratch}, s0, {scratch}").unwrap();
            writeln!(self.out, "\t{opc}\t{reg}, 0({scratch})").unwrap();
        }
    }

    fn treg(n: u8) -> &'static str {
        match n {
            0 => "a0",
            1 => TEMP_REGS[0],
            2 => TEMP_REGS[1],
            3 => TEMP_REGS[2],
            4 => TEMP_REGS[3],
            5 => TEMP_REGS[4],
            6 => TEMP_REGS[5],
            _ => TEMP_REGS[6],
        }
    }

    fn store_local(&mut self, off: i64, ty: &Type, src: u8) -> Result<(), String> {
        let r = Self::treg(src);
        let sz = self.type_size(ty);
        self.emit_store_s0(r, off, sz);
        Ok(())
    }

    fn load_local(&mut self, off: i64, ty: &Type, dest: u8) -> Result<(), String> {
        let r = Self::treg(dest);
        let opc = match ty {
            Type::Char => "lbu",
            Type::SChar => "lb",
            Type::Short => "lh",
            Type::UShort => "lhu",
            Type::Int | Type::UInt => "lw",
            _ => "ld",
        };
        self.emit_load_s0(r, off, opc);
        Ok(())
    }

    fn store_at_mem(&mut self, addr: u8, ty: &Type, val: u8) -> Result<(), String> {
        let base = Self::treg(addr);
        let r = Self::treg(val);
        match self.type_size(ty) {
            1 => writeln!(self.out, "\tsb\t{r}, 0({base})").unwrap(),
            2 => writeln!(self.out, "\tsh\t{r}, 0({base})").unwrap(),
            4 => writeln!(self.out, "\tsw\t{r}, 0({base})").unwrap(),
            _ => writeln!(self.out, "\tsd\t{r}, 0({base})").unwrap(),
        }
        Ok(())
    }

    fn load_from_mem(&mut self, addr: u8, ty: &Type, dest: u8) -> Result<(), String> {
        let base = Self::treg(addr);
        let r = Self::treg(dest);
        match ty {
            Type::Char => writeln!(self.out, "\tlbu\t{r}, 0({base})").unwrap(),
            Type::SChar => writeln!(self.out, "\tlb\t{r}, 0({base})").unwrap(),
            Type::Short => writeln!(self.out, "\tlh\t{r}, 0({base})").unwrap(),
            Type::UShort => writeln!(self.out, "\tlhu\t{r}, 0({base})").unwrap(),
            Type::Int | Type::UInt => writeln!(self.out, "\tlw\t{r}, 0({base})").unwrap(),
            _ => writeln!(self.out, "\tld\t{r}, 0({base})").unwrap(),
        }
        Ok(())
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

    fn emit_lvalue_addr(
        &mut self,
        e: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        let rd = Self::treg(dest);
        match e {
            Expr::Var(name) => {
                if let Ok(sy) = self.lookup(name) {
                    match &sy.storage {
                        Storage::Local { offset } => {
                            self.emit_addi_s0(rd, *offset);
                        }
                        Storage::Global { name } => {
                            writeln!(self.out, "\tlla\t{rd}, {}", sym(name)).unwrap();
                        }
                    }
                    return Ok(sy.ty);
                }
                if self.funcs.contains_key(name) {
                    writeln!(self.out, "\tlla\t{rd}, {}", sym(name)).unwrap();
                    return Ok(Type::Ptr(Box::new(Type::Void)));
                }
                Err(format!("riscv64: unknown symbol '{name}'"))
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self.emit_expr_rval(expr, dest, typedefs),
            Expr::Index { base, index } => {
                // Spill base: index eval reuses t0/t1 and would clobber the pointer
                // (n-queens 00040: `t[x+8*y]++` lost the calloc'd base).
                let bty = self.emit_expr_rval(base, 1, typedefs)?;
                writeln!(self.out, "\taddi\tsp, sp, -16").unwrap();
                writeln!(self.out, "\tsd\t{}, 0(sp)", Self::treg(1)).unwrap();
                let elem = match &bty {
                    Type::Ptr(e) | Type::Array(e, _) => e.as_ref().clone(),
                    _ => Type::Int,
                };
                let esz = self.type_size(&elem).max(1);
                self.emit_expr_rval(index, 2, typedefs)?;
                if esz != 1 {
                    if esz > 0 && (esz & (esz - 1)) == 0 {
                        let sh = esz.trailing_zeros();
                        writeln!(
                            self.out,
                            "\tslli\t{}, {}, {sh}",
                            Self::treg(2),
                            Self::treg(2)
                        )
                        .unwrap();
                    } else {
                        writeln!(self.out, "\tli\tt6, {esz}").unwrap();
                        writeln!(
                            self.out,
                            "\tmul\t{}, {}, t6",
                            Self::treg(2),
                            Self::treg(2)
                        )
                        .unwrap();
                    }
                }
                writeln!(self.out, "\tld\t{}, 0(sp)", Self::treg(1)).unwrap();
                writeln!(self.out, "\taddi\tsp, sp, 16").unwrap();
                writeln!(
                    self.out,
                    "\tadd\t{rd}, {}, {}",
                    Self::treg(1),
                    Self::treg(2)
                )
                .unwrap();
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
                            let l = self.layout_fields(&fs, false, false);
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
                        let l = self.layout_fields(&fs, false, false);
                        l.fields.get(field).cloned().unwrap_or((0, Type::Int))
                    }
                    _ => (0, Type::Int),
                };
                if off != 0 {
                    if (-2048..=2047).contains(&off) {
                        writeln!(self.out, "\taddi\t{rd}, {rd}, {off}").unwrap();
                    } else {
                        let scratch = if rd == "t6" { "t5" } else { "t6" };
                        writeln!(self.out, "\tli\t{scratch}, {off}").unwrap();
                        writeln!(self.out, "\tadd\t{rd}, {rd}, {scratch}").unwrap();
                    }
                }
                Ok(fty)
            }
            _ => Err(format!("riscv64: not an lvalue: {e:?}")),
        }
    }

    fn emit_expr_rval(
        &mut self,
        e: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        let rd = Self::treg(dest);
        match e {
            Expr::Int(n) | Expr::Char(n) => {
                self.emit_li(dest, *n);
                Ok(Type::Int)
            }
            Expr::Float(_) => Err("riscv64: float immediates not supported yet".into()),
            Expr::String(s) => {
                let id = self.intern_str(s);
                writeln!(self.out, "\tlla\t{rd}, .Lstr_{id}").unwrap();
                Ok(Type::Ptr(Box::new(Type::Char)))
            }
            Expr::Var(name) => {
                if let Ok(sy) = self.lookup(name) {
                    match &sy.ty {
                        Type::Array(elem, _) => match &sy.storage {
                            Storage::Local { offset } => {
                                self.emit_addi_s0(rd, *offset);
                                Ok(Type::Ptr(elem.clone()))
                            }
                            Storage::Global { name } => {
                                writeln!(self.out, "\tlla\t{rd}, {}", sym(name)).unwrap();
                                Ok(Type::Ptr(elem.clone()))
                            }
                        },
                        ty => match &sy.storage {
                            Storage::Local { offset } => {
                                self.load_local(*offset, ty, dest)?;
                                Ok(ty.clone())
                            }
                            Storage::Global { name } => {
                                writeln!(self.out, "\tlla\tt6, {}", sym(name)).unwrap();
                                self.load_from_mem(7, ty, dest)?;
                                Ok(ty.clone())
                            }
                        },
                    }
                } else if self.funcs.contains_key(name) {
                    writeln!(self.out, "\tlla\t{rd}, {}", sym(name)).unwrap();
                    Ok(Type::Ptr(Box::new(Type::Void)))
                } else {
                    writeln!(self.out, "\tlla\tt6, {}", sym(name)).unwrap();
                    writeln!(self.out, "\tld\t{rd}, 0(t6)").unwrap();
                    Ok(Type::Int)
                }
            }
            Expr::Unary { op, expr } => match op {
                UnaryOp::Neg => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tneg\t{rd}, {rd}").unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::Not => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tseqz\t{rd}, {rd}").unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::BitNot => {
                    self.emit_expr_rval(expr, dest, typedefs)?;
                    writeln!(self.out, "\tnot\t{rd}, {rd}").unwrap();
                    Ok(Type::Int)
                }
                UnaryOp::Addr => {
                    self.emit_lvalue_addr(expr, dest, typedefs)?;
                    Ok(Type::Ptr(Box::new(Type::Void)))
                }
                UnaryOp::Deref => {
                    self.emit_expr_rval(expr, 1, typedefs)?;
                    let ty = match self.typeof_expr(expr, typedefs) {
                        Type::Ptr(t) => *t,
                        _ => Type::Int,
                    };
                    self.load_from_mem(1, &ty, dest)?;
                    Ok(ty)
                }
            },
            Expr::Binary { op, left, right } => {
                if matches!(op, BinOp::Comma) {
                    self.emit_expr_rval(left, 0, typedefs)?;
                    return self.emit_expr_rval(right, dest, typedefs);
                }
                self.emit_binop(*op, left, right, dest, typedefs)?;
                Ok(Type::Int)
            }
            Expr::Assign { left, right } => {
                self.emit_expr_rval(right, 0, typedefs)?;
                self.emit_store(left, 0, typedefs)?;
                if dest != 0 {
                    writeln!(self.out, "\tmv\t{rd}, a0").unwrap();
                }
                Ok(self.typeof_expr(left, typedefs))
            }
            Expr::CompoundAssign { op, left, right } => {
                let lty = self.emit_lvalue_addr(left, 1, typedefs)?;
                self.load_from_mem(1, &lty, 0)?;
                writeln!(self.out, "\tmv\tt6, a0").unwrap();
                self.emit_expr_rval(right, 2, typedefs)?;
                match op {
                    BinOp::Add if matches!(&lty, Type::Ptr(_)) => {
                        if let Type::Ptr(inner) = &lty {
                            self.emit_scale_reg(2, self.type_size(inner).max(1));
                        }
                        writeln!(self.out, "\tadd\ta0, t6, t1").unwrap();
                    }
                    BinOp::Sub if matches!(&lty, Type::Ptr(_)) => {
                        if let Type::Ptr(inner) = &lty {
                            self.emit_scale_reg(2, self.type_size(inner).max(1));
                        }
                        writeln!(self.out, "\tsub\ta0, t6, t1").unwrap();
                    }
                    _ => {
                        writeln!(self.out, "\tmv\ta0, t6").unwrap();
                        self.emit_binop_regs(*op, 0, 2, 0)?;
                    }
                }
                self.store_at_mem(1, &lty, 0)?;
                if dest != 0 {
                    writeln!(self.out, "\tmv\t{rd}, a0").unwrap();
                }
                Ok(lty)
            }
            Expr::Call { name, args } => {
                self.emit_call(name, args, dest, typedefs)?;
                Ok(Type::Int)
            }
            Expr::Cast { expr, ty } => {
                self.emit_expr_rval(expr, dest, typedefs)?;
                Ok(ty.clone())
            }
            Expr::SizeofType(ty) => {
                self.emit_li(dest, self.type_size(ty));
                Ok(Type::ULong)
            }
            Expr::SizeofExpr(expr) => {
                let ty = self.typeof_expr(expr, typedefs);
                self.emit_li(dest, self.type_size(&ty));
                Ok(Type::ULong)
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let else_l = self.lab("cond_else");
                let end_l = self.lab("cond_end");
                self.emit_expr_rval(cond, 0, typedefs)?;
                writeln!(self.out, "\tbeqz\ta0, {else_l}").unwrap();
                self.emit_expr_rval(then_e, dest, typedefs)?;
                writeln!(self.out, "\tj\t{end_l}").unwrap();
                writeln!(self.out, "{else_l}:").unwrap();
                self.emit_expr_rval(else_e, dest, typedefs)?;
                writeln!(self.out, "{end_l}:").unwrap();
                Ok(Type::Int)
            }
            Expr::PreInc(ex) | Expr::PreDec(ex) => {
                let is_inc = matches!(e, Expr::PreInc(_));
                let ty = self.emit_lvalue_addr(ex, 1, typedefs)?;
                self.load_from_mem(1, &ty, 0)?;
                let step = match &ty {
                    Type::Ptr(inner) => self.type_size(inner).max(1),
                    _ => 1,
                };
                if is_inc {
                    writeln!(self.out, "\taddi\ta0, a0, {step}").unwrap();
                } else {
                    writeln!(self.out, "\taddi\ta0, a0, -{step}").unwrap();
                }
                self.store_at_mem(1, &ty, 0)?;
                if dest != 0 {
                    writeln!(self.out, "\tmv\t{rd}, a0").unwrap();
                }
                Ok(ty)
            }
            Expr::PostInc(ex) | Expr::PostDec(ex) => {
                let is_inc = matches!(e, Expr::PostInc(_));
                let ty = self.emit_lvalue_addr(ex, 1, typedefs)?;
                self.load_from_mem(1, &ty, 0)?;
                writeln!(self.out, "\tmv\tt6, a0").unwrap();
                let step = match &ty {
                    Type::Ptr(inner) => self.type_size(inner).max(1),
                    _ => 1,
                };
                if is_inc {
                    writeln!(self.out, "\taddi\ta0, a0, {step}").unwrap();
                } else {
                    writeln!(self.out, "\taddi\ta0, a0, -{step}").unwrap();
                }
                self.store_at_mem(1, &ty, 0)?;
                if dest != 0 {
                    writeln!(self.out, "\tmv\t{rd}, t6").unwrap();
                } else {
                    writeln!(self.out, "\tmv\ta0, t6").unwrap();
                }
                Ok(ty)
            }
            Expr::Index { .. } | Expr::Member { .. } => {
                let ty = self.emit_lvalue_addr(e, 1, typedefs)?;
                // Array lvalues decay to pointer (address), not a loaded value.
                // Needed for `a[0].sub[i]` where `.sub` has type int[2].
                if let Type::Array(elem, _) = ty {
                    if dest != 1 {
                        writeln!(self.out, "\tmv\t{rd}, {}", Self::treg(1)).unwrap();
                    }
                    return Ok(Type::Ptr(elem));
                }
                self.load_from_mem(1, &ty, dest)?;
                Ok(ty)
            }
            Expr::InitList { .. } => {
                self.emit_li(dest, 0);
                Ok(Type::Int)
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
                let rd = Self::treg(dest);
                writeln!(
                    self.out,
                    "\tlla\t{rd}, L_{}_goto_{}",
                    self.func_name, label
                )
                .unwrap();
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
        }
    }

    fn emit_li(&mut self, dest: u8, n: i64) {
        let rd = Self::treg(dest);
        writeln!(self.out, "\tli\t{rd}, {n}").unwrap();
    }

    fn emit_store(
        &mut self,
        left: &Expr,
        src: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        match left {
            Expr::Var(name) => {
                if let Ok(local) = self.lookup(name) {
                    match local.storage {
                        Storage::Local { offset } => self.store_local(offset, &local.ty, src),
                        Storage::Global { name } => {
                            let r = Self::treg(src);
                            writeln!(self.out, "\tlla\tt6, {}", sym(&name)).unwrap();
                            writeln!(self.out, "\tsd\t{r}, 0(t6)").unwrap();
                            Ok(())
                        }
                    }
                } else {
                    let r = Self::treg(src);
                    writeln!(self.out, "\tlla\tt6, {}", sym(name)).unwrap();
                    writeln!(self.out, "\tsd\t{r}, 0(t6)").unwrap();
                    Ok(())
                }
            }
            _ => {
                let ty = self.emit_lvalue_addr(left, 1, typedefs)?;
                self.store_at_mem(1, &ty, src)
            }
        }
    }

    fn emit_binop(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        match op {
            BinOp::And => {
                let false_l = self.lab("and_false");
                let end_l = self.lab("and_end");
                self.emit_expr_rval(left, dest, typedefs)?;
                writeln!(self.out, "\tbeqz\t{}, {false_l}", Self::treg(dest)).unwrap();
                self.emit_expr_rval(right, dest, typedefs)?;
                writeln!(self.out, "\tbeqz\t{}, {false_l}", Self::treg(dest)).unwrap();
                self.emit_li(dest, 1);
                writeln!(self.out, "\tj\t{end_l}").unwrap();
                writeln!(self.out, "{false_l}:").unwrap();
                self.emit_li(dest, 0);
                writeln!(self.out, "{end_l}:").unwrap();
                return Ok(());
            }
            BinOp::Or => {
                let true_l = self.lab("or_true");
                let end_l = self.lab("or_end");
                self.emit_expr_rval(left, dest, typedefs)?;
                writeln!(self.out, "\tbnez\t{}, {true_l}", Self::treg(dest)).unwrap();
                self.emit_expr_rval(right, dest, typedefs)?;
                writeln!(self.out, "\tbnez\t{}, {true_l}", Self::treg(dest)).unwrap();
                self.emit_li(dest, 0);
                writeln!(self.out, "\tj\t{end_l}").unwrap();
                writeln!(self.out, "{true_l}:").unwrap();
                self.emit_li(dest, 1);
                writeln!(self.out, "{end_l}:").unwrap();
                return Ok(());
            }
            BinOp::Comma => {
                self.emit_expr_rval(left, 1, typedefs)?;
                return self.emit_expr_rval(right, dest, typedefs).map(|_| ());
            }
            _ => {}
        }
        let lty = self.typeof_expr(left, typedefs);
        let rty = self.typeof_expr(right, typedefs);
        self.emit_expr_rval(left, 1, typedefs)?;
        writeln!(self.out, "\taddi\tsp, sp, -16").unwrap();
        writeln!(self.out, "\tsd\tt0, 0(sp)").unwrap();
        self.emit_expr_rval(right, 2, typedefs)?;
        writeln!(self.out, "\tld\tt6, 0(sp)").unwrap();
        let rd = Self::treg(dest);
        match op {
            BinOp::Add => {
                if let Type::Ptr(inner) = &lty {
                    self.emit_scale_reg(2, self.type_size(inner).max(1));
                    writeln!(self.out, "\tadd\t{rd}, t6, t1").unwrap();
                    writeln!(self.out, "\taddi\tsp, sp, 16").unwrap();
                    return Ok(());
                }
                if let Type::Ptr(inner) = rty {
                    writeln!(self.out, "\tmv\tt0, t6").unwrap();
                    self.emit_scale_reg(1, self.type_size(&inner).max(1));
                    writeln!(self.out, "\tadd\t{rd}, t1, t0").unwrap();
                    writeln!(self.out, "\taddi\tsp, sp, 16").unwrap();
                    return Ok(());
                }
            }
            BinOp::Sub => {
                if let Type::Ptr(inner) = &lty {
                    let esz = self.type_size(inner).max(1);
                    if matches!(rty, Type::Ptr(_)) {
                        writeln!(self.out, "\tsub\t{rd}, t6, t1").unwrap();
                        if esz != 1 {
                            writeln!(self.out, "\tli\tt6, {esz}").unwrap();
                            writeln!(self.out, "\tdiv\t{rd}, {rd}, t6").unwrap();
                        }
                        writeln!(self.out, "\taddi\tsp, sp, 16").unwrap();
                        return Ok(());
                    }
                    self.emit_scale_reg(2, esz);
                    writeln!(self.out, "\tsub\t{rd}, t6, t1").unwrap();
                    writeln!(self.out, "\taddi\tsp, sp, 16").unwrap();
                    return Ok(());
                }
            }
            _ => {}
        }
        writeln!(self.out, "\tmv\tt0, t6").unwrap();
        self.emit_binop_regs(op, 1, 2, dest)?;
        writeln!(self.out, "\taddi\tsp, sp, 16").unwrap();
        Ok(())
    }

    fn emit_scale_reg(&mut self, reg: u8, esz: i64) {
        if esz == 1 {
            return;
        }
        let r = Self::treg(reg);
        if esz > 0 && (esz & (esz - 1)) == 0 {
            let sh = esz.trailing_zeros();
            writeln!(self.out, "\tslli\t{r}, {r}, {sh}").unwrap();
        } else {
            writeln!(self.out, "\tli\tt6, {esz}").unwrap();
            writeln!(self.out, "\tmul\t{r}, {r}, t6").unwrap();
        }
    }

    fn emit_binop_regs(&mut self, op: BinOp, lhs: u8, rhs: u8, dest: u8) -> Result<(), String> {
        let rd = Self::treg(dest);
        let rs1 = Self::treg(lhs);
        let rs2 = Self::treg(rhs);
        match op {
            BinOp::Add => writeln!(self.out, "\tadd\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Sub => writeln!(self.out, "\tsub\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Mul => writeln!(self.out, "\tmul\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Div => writeln!(self.out, "\tdiv\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Mod => writeln!(self.out, "\trem\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::BitAnd => writeln!(self.out, "\tand\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::BitOr => writeln!(self.out, "\tor\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::BitXor => writeln!(self.out, "\txor\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Shl => writeln!(self.out, "\tsll\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Shr => writeln!(self.out, "\tsra\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Eq => {
                writeln!(self.out, "\txor\t{rd}, {rs1}, {rs2}").unwrap();
                writeln!(self.out, "\tseqz\t{rd}, {rd}").unwrap();
            }
            BinOp::Ne => {
                writeln!(self.out, "\txor\t{rd}, {rs1}, {rs2}").unwrap();
                writeln!(self.out, "\tsnez\t{rd}, {rd}").unwrap();
            }
            BinOp::Lt => writeln!(self.out, "\tslt\t{rd}, {rs1}, {rs2}").unwrap(),
            BinOp::Gt => writeln!(self.out, "\tslt\t{rd}, {rs2}, {rs1}").unwrap(),
            BinOp::Le => {
                writeln!(self.out, "\tslt\t{rd}, {rs2}, {rs1}").unwrap();
                writeln!(self.out, "\txori\t{rd}, {rd}, 1").unwrap();
            }
            BinOp::Ge => {
                writeln!(self.out, "\tslt\t{rd}, {rs1}, {rs2}").unwrap();
                writeln!(self.out, "\txori\t{rd}, {rd}, 1").unwrap();
            }
            BinOp::And | BinOp::Or | BinOp::Comma => unreachable!(),
        }
        Ok(())
    }

    fn emit_call(
        &mut self,
        name: &str,
        args: &[Expr],
        dest: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        if name == "__indirect__" {
            if args.is_empty() {
                return Err("riscv64: indirect call missing callee".into());
            }
            let (callee, real_args) = args.split_first().unwrap();
            let callee = match callee {
                Expr::Unary {
                    op: UnaryOp::Deref,
                    expr,
                } => expr.as_ref(),
                other => other,
            };
            if real_args.len() > 8 {
                return Err("riscv64: >8 indirect call args".into());
            }
            for (i, a) in real_args.iter().enumerate() {
                let tmp = (i as u8) + 1;
                self.emit_expr_rval(a, tmp, typedefs)?;
                if Self::treg(tmp) != ARG_REGS[i] {
                    writeln!(
                        self.out,
                        "\tmv\t{}, {}",
                        ARG_REGS[i],
                        Self::treg(tmp)
                    )
                    .unwrap();
                }
            }
            self.emit_expr_rval(callee, 7, typedefs)?;
            writeln!(self.out, "\tjalr\tra, t6, 0").unwrap();
            if dest != 0 {
                writeln!(self.out, "\tmv\t{}, a0", Self::treg(dest)).unwrap();
            }
            return Ok(());
        }
        if args.len() > 8 {
            return Err(format!("riscv64: >8 call args not supported ({name})"));
        }
        let mut staged: Vec<(usize, u8)> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let tmp = (i as u8) + 1;
            if tmp > 6 {
                self.emit_expr_rval(a, 0, typedefs)?;
                let spill = -(self.stack_size + 8 + (i as i64) * 8);
                self.emit_sd_s0("a0", spill);
                staged.push((i, 255));
            } else {
                self.emit_expr_rval(a, tmp, typedefs)?;
                staged.push((i, tmp));
            }
        }
        for (i, tmp) in staged {
            if tmp == 255 {
                let spill = -(self.stack_size + 8 + (i as i64) * 8);
                self.emit_ld_s0(ARG_REGS[i], spill);
            } else if Self::treg(tmp) != ARG_REGS[i] {
                writeln!(
                    self.out,
                    "\tmv\t{}, {}",
                    ARG_REGS[i],
                    Self::treg(tmp)
                )
                .unwrap();
            }
        }
        writeln!(self.out, "\tcall\t{}", sym(name)).unwrap();
        if dest != 0 {
            writeln!(self.out, "\tmv\t{}, a0", Self::treg(dest)).unwrap();
        }
        Ok(())
    }
}

pub fn emit_assembly(prog: &Program) -> Result<String, String> {
    let mut cg = Codegen::new();
    cg.compile(prog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog_main_return(n: i64) -> Program {
        Program {
            items: vec![Item::Func(Function {
                name: "main".into(),
                ret: Type::Int,
                params: vec![],
                variadic: false,
                body: Some(vec![Stmt::Return(Some(Expr::Int(n)))]),
                is_static: false,
                is_weak: false,
                section: None,
            })],
            type_layouts: vec![],
        }
    }

    #[test]
    fn return_code_emits_li_and_ret() {
        let asm = emit_assembly(&prog_main_return(7)).expect("emit");
        assert!(asm.contains(".globl\tmain"), "{asm}");
        assert!(asm.contains("li\ta0, 7"), "{asm}");
        assert!(asm.contains("\tret"), "{asm}");
    }

    #[test]
    fn hello_emits_printf_call_and_rodata() {
        let prog = Program {
            items: vec![
                Item::Func(Function {
                    name: "printf".into(),
                    ret: Type::Int,
                    params: vec![("fmt".into(), Type::Ptr(Box::new(Type::Char)))],
                    variadic: true,
                    body: None,
                    is_static: false,
                    is_weak: false,
                    section: None,
                }),
                Item::Func(Function {
                    name: "main".into(),
                    ret: Type::Int,
                    params: vec![],
                    variadic: false,
                    body: Some(vec![
                        Stmt::Expr(Expr::Call {
                            name: "printf".into(),
                            args: vec![Expr::String("Hello, world!\n".into())],
                        }),
                        Stmt::Return(Some(Expr::Int(0))),
                    ]),
                    is_static: false,
                    is_weak: false,
                    section: None,
                }),
            ],
            type_layouts: vec![],
        };
        let asm = emit_assembly(&prog).expect("emit");
        assert!(asm.contains("call\tprintf"), "{asm}");
        assert!(asm.contains(".Lstr_0:"), "{asm}");
        assert!(asm.contains("Hello, world!"), "{asm}");
        assert!(asm.contains("lla\t"), "{asm}");
    }

    #[test]
    fn arith_add_chain() {
        let prog = Program {
            items: vec![Item::Func(Function {
                name: "main".into(),
                ret: Type::Int,
                params: vec![],
                variadic: false,
                body: Some(vec![Stmt::Return(Some(Expr::Binary {
                    op: BinOp::Add,
                    left: Box::new(Expr::Binary {
                        op: BinOp::Add,
                        left: Box::new(Expr::Int(10)),
                        right: Box::new(Expr::Int(20)),
                    }),
                    right: Box::new(Expr::Int(12)),
                }))]),
                is_static: false,
                is_weak: false,
                section: None,
            })],
            type_layouts: vec![],
        };
        let asm = emit_assembly(&prog).expect("emit");
        assert!(asm.contains("add\t"), "{asm}");
    }

    #[test]
    fn lp64_constants() {
        assert_eq!(lp64::INT, 4);
        assert_eq!(lp64::LONG, 8);
        assert_eq!(lp64::PTR, 8);
        assert_eq!(ARG_REGS.len(), 8);
    }

    #[test]
    fn large_frame_prologue_avoids_12bit_imm() {
        // int v[1000] forces frame > 2047; addi/ld/sd immediates must be expanded.
        let prog = Program {
            items: vec![Item::Func(Function {
                name: "main".into(),
                ret: Type::Int,
                params: vec![],
                variadic: false,
                body: Some(vec![
                    Stmt::Decl(VarDecl {
                        name: "v".into(),
                        ty: Type::Array(Box::new(Type::Int), 1000),
                        init: None,
                        is_static: false,
                        is_extern: false,
                        is_weak: false,
                        section: None,
                    }),
                    Stmt::Return(Some(Expr::Int(0))),
                ]),
                is_static: false,
                is_weak: false,
                section: None,
            })],
            type_layouts: vec![],
        };
        let asm = emit_assembly(&prog).expect("emit");
        assert!(
            !asm.contains("addi\tsp, sp, -4032")
                && !asm.contains("addi\tsp, sp, 4032")
                && !asm.contains("addi\ts0, sp, 4032"),
            "raw out-of-range addi still present:\n{asm}"
        );
        assert!(
            asm.contains("li\tt6,") && asm.contains("add\ts0, sp, t6"),
            "expected li/add expansion for FP setup:\n{asm}"
        );
        assert!(
            asm.contains("mv\tsp, s0"),
            "expected FP-based epilogue:\n{asm}"
        );
    }

    #[test]
    fn array_member_decay_does_not_load() {
        // struct { int v; int sub[2]; } — `.sub` must yield address, not ld.
        let lay = Layout {
            size: 12,
            align: 4,
            fields: {
                let mut m = HashMap::new();
                m.insert("v".into(), (0, Type::Int));
                m.insert("sub".into(), (4, Type::Array(Box::new(Type::Int), 2)));
                m
            },
        };
        let mut cg = Codegen::new();
        cg.layouts.insert("S".into(), lay);
        cg.globals.insert("a".into(), Type::Struct("S".into()));
        cg.scopes[0].insert(
            "a".into(),
            Sym {
                ty: Type::Struct("S".into()),
                storage: Storage::Global {
                    name: "a".into(),
                },
            },
        );
        let e = Expr::Member {
            base: Box::new(Expr::Var("a".into())),
            field: "sub".into(),
            arrow: false,
        };
        cg.emit_expr_rval(&e, 0, &HashMap::new()).expect("emit");
        let asm = cg.out;
        assert!(
            !asm.contains("ld\ta0,") && !asm.contains("lw\ta0,"),
            "array member must decay without load:\n{asm}"
        );
        assert!(
            asm.contains("addi\t") || asm.contains("add\t"),
            "expected address arithmetic:\n{asm}"
        );
    }
}
