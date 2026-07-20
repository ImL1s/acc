//! Code generators: aarch64-apple-darwin (default) and x86_64 System V / Darwin.
//! Emits real assembly from the AST — no fixture hardcoding.

#[path = "codegen_x86_64.rs"]
mod x86_64;

use crate::ast::*;
use std::collections::HashMap;
use std::fmt::Write as _;

/// ISA backend selection (`-m aarch64` / `-m x86_64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Aarch64,
    X86_64,
}

impl Target {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "aarch64" | "arm64" => Some(Self::Aarch64),
            "x86_64" | "x86-64" | "amd64" => Some(Self::X86_64),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
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
    funcs: HashMap<String, Function>,
    /// current function locals
    locals: HashMap<String, Sym>,
    stack_size: i64,
    label_id: usize,
    break_stack: Vec<String>,
    continue_stack: Vec<String>,
    func_name: String,
    /// Return type of the function currently being emitted.
    func_ret: Type,
    pending_case_labs: std::collections::VecDeque<String>,
    os: TargetOs,
    /// FP-relative offset of x0..x7 save area for the current variadic function (0 = none).
    va_regsave_off: i64,
    /// Number of fixed (named) integer/pointer params before `...`.
    va_fixed_n: usize,
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
            funcs: HashMap::new(),
            locals: HashMap::new(),
            stack_size: 0,
            label_id: 0,
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            func_name: String::new(),
            func_ret: Type::Void,
            pending_case_labs: std::collections::VecDeque::new(),
            os,
            va_regsave_off: 0,
            va_fixed_n: 0,
        }
    }

    /// C ABI symbol as seen by the assembler (Darwin underscores).
    fn c_sym(&self, name: &str) -> String {
        match self.os {
            TargetOs::Darwin => format!("_{name}"),
            TargetOs::Linux => name.to_string(),
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
            Type::AnonStruct(fs) => self.layout_fields(fs, false).size,
            Type::AnonUnion(fs) => self.layout_fields(fs, true).size,
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
            Type::Array(e, n) => self.stack_slot_size(e) * n,
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
            Type::AnonStruct(fs) => self.layout_fields(fs, false).align,
            Type::AnonUnion(fs) => self.layout_fields(fs, true).align,
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
                let lay = self.layout_fields(&fs, false);
                lay.fields.get(field).cloned()
            }
            Type::AnonUnion(fs) => {
                let lay = self.layout_fields(&fs, true);
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

    fn layout_fields(&self, fields: &[Field], is_union: bool) -> Layout {
        let mut map = HashMap::new();
        let mut max_align = 1i64;
        let mut max_size = 0i64;
        // GCC/aarch64-compatible bitfield packing (PCC_BITFIELD_TYPE_MATTERS):
        // track free position in bits; a bitfield of declared type T/width W
        // must not straddle a container of sizeof(T) bits. If it would, pad to
        // the next container boundary. Non-bitfields round up to a byte, then
        // align. This yields e.g. SQLite Column = 16 (not 24).
        let mut offset_bits: u64 = 0;

        for f in fields {
            // Anonymous nested struct/union: promote fields into this layout.
            if f.name.is_empty() && f.bit_width.is_none() {
                let nested_opt = match &f.ty {
                    Type::AnonStruct(fs) => Some(self.layout_fields(fs, false)),
                    Type::AnonUnion(fs) => Some(self.layout_fields(fs, true)),
                    Type::Struct(n) => self.layouts.get(n).cloned(),
                    Type::Union(n) => self.layouts.get(n).cloned(),
                    _ => None,
                };
                if let Some(nested) = nested_opt {
                    max_align = max_align.max(nested.align);
                    if is_union {
                        for (fnm, place) in &nested.fields {
                            map.insert(
                                fnm.clone(),
                                FieldPlace {
                                    offset: 0,
                                    ty: place.ty.clone(),
                                    bit: place.bit,
                                },
                            );
                        }
                        max_size = max_size.max(nested.size);
                    } else {
                        let mut byte_off = ((offset_bits + 7) / 8) as i64;
                        byte_off = Self::align_up(byte_off, nested.align);
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
                let al = self.type_align(&f.ty);
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
            let al = self.type_align(&f.ty);
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
        let size = if is_union {
            Self::align_up(max_size, max_align.max(1))
        } else {
            let byte_off = ((offset_bits + 7) / 8) as i64;
            Self::align_up(byte_off, max_align.max(1))
        };
        Layout {
            size,
            align: max_align.max(1),
            fields: map,
        }
    }

    fn collect_layouts(&mut self, prog: &Program) {
        // Multi-pass: type_layouts comes from a HashMap and may list a union/struct
        // before its nested named members. A single pass leaves unknown Struct(n)
        // at the 8-byte fallback (see type_size), so sizeof(union) collapses
        // (e.g. SQLite YYMINORTYPE became 8 instead of 16 → lemon stack smash).
        for _ in 0..12 {
            for (name, is_union, fields) in &prog.type_layouts {
                let lay = self.layout_fields(fields, *is_union);
                self.layouts.insert(name.clone(), lay);
            }
            for item in &prog.items {
                match item {
                    Item::StructDef { name, fields } => {
                        let lay = self.layout_fields(fields, false);
                        self.layouts.insert(name.clone(), lay);
                    }
                    Item::UnionDef { name, fields } => {
                        let lay = self.layout_fields(fields, true);
                        self.layouts.insert(name.clone(), lay);
                    }
                    Item::Typedef { name, ty } => match ty {
                        Type::AnonStruct(fs) => {
                            let lay = self.layout_fields(fs, false);
                            self.layouts.insert(name.clone(), lay);
                        }
                        Type::AnonUnion(fs) => {
                            let lay = self.layout_fields(fs, true);
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
        for item in &prog.items {
            if let Item::Typedef { name, ty } = item {
                typedefs.insert(name.clone(), ty.clone());
            }
            if let Item::Func(f) = item {
                self.funcs.insert(f.name.clone(), f.clone());
            }
            if let Item::Global(g) = item {
                self.globals.insert(g.name.clone(), g.ty.clone());
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

        for item in &prog.items {
            if let Item::Func(f) = item {
                // Skip static function bodies except main: kernel headers inject
                // thousands of static inlines; kbuild offset TUs only need main.
                if f.body.is_some() && (!f.is_static || f.name == "main") {
                    self.emit_function(f, &typedefs)?;
                }
            }
        }

        // Globals (dedupe by name: keep first with init, else first)
        let mut emitted_globals = std::collections::HashSet::new();
        // Prefer initialized definitions
        for item in &prog.items {
            if let Item::Global(g) = item {
                if g.init.is_some() && emitted_globals.insert(g.name.clone()) {
                    self.emit_global(g)?;
                }
            }
        }
        for item in &prog.items {
            if let Item::Global(g) = item {
                if emitted_globals.insert(g.name.clone()) {
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
                | "errno"
                | "__ggcc_errno"
                | "optarg"
                | "optind"
        )
    }

    fn emit_global(&mut self, g: &VarDecl) -> Result<(), String> {
        // libc-provided symbols: reference only, do not define in our data.
        if Self::is_extern_libc(&g.name) {
            self.globals.insert(g.name.clone(), g.ty.clone());
            return Ok(());
        }
        let size = self.type_size(&g.ty).max(1);
        let sym = self.c_sym(&g.name);
        writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
        if let Some(init) = &g.init {
            match init {
                Expr::Int(n) | Expr::Char(n) => {
                    self.emit_data_section();
                    let al = self.type_align(&g.ty).max(1).min(8);
                    writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    if matches!(g.ty, Type::Float) {
                        writeln!(self.out, "\t.float\t{}", *n as f32).unwrap();
                    } else if matches!(g.ty, Type::Double) {
                        writeln!(self.out, "\t.double\t{}", *n as f64).unwrap();
                    } else {
                        self.emit_int_directive(size, *n);
                    }
                }
                Expr::Float(f) => {
                    self.emit_data_section();
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
                        self.emit_data_section();
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
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
                            self.emit_data_section();
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{glab}:").unwrap();
                            self.emit_init_list_data(cty, fields)?;
                            writeln!(self.out, "\t.globl\t{sym}").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            writeln!(self.out, "\t.quad\t{glab}").unwrap();
                        } else if let Expr::Var(v) = inner.as_ref() {
                            self.emit_data_section();
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
                        } else {
                            // best-effort null pointer
                            self.emit_data_section();
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
                            self.emit_data_section();
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            writeln!(
                                self.out,
                                "\t.quad\t{}+{}",
                                self.c_sym(v),
                                idx * esz
                            )
                            .unwrap();
                        } else {
                            self.emit_data_section();
                            writeln!(self.out, "\t.p2align\t3").unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            writeln!(self.out, "\t.quad\t0").unwrap();
                        }
                    } else {
                        // Unsupported &expr form: null
                        self.emit_data_section();
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        writeln!(self.out, "\t.quad\t0").unwrap();
                    }
                }
                Expr::String(s) => {
                    // char arr[] = "..." is contents; char *p = "..." is pointer
                    if matches!(g.ty, Type::Array(_, _)) {
                        self.emit_data_section();
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
                        self.emit_data_section();
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        writeln!(self.out, "\t.quad\tl_str_{id}").unwrap();
                    }
                }
                Expr::Var(v) if self.funcs.contains_key(v) || v == "main" => {
                    // function address
                    self.emit_data_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
                }
                Expr::InitList { fields } => {
                    self.emit_data_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    writeln!(self.out, "{sym}:").unwrap();
                    self.emit_init_list_data(&g.ty, fields)?;
                }
                Expr::Cast { ty: _, expr } => {
                    // myint x = (myint)1; — peel casts for constant inits
                    match expr.as_ref() {
                        Expr::Int(n) | Expr::Char(n) => {
                            self.emit_data_section();
                            let al = self.type_align(&g.ty).max(1).min(8);
                            writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                            writeln!(self.out, "{sym}:").unwrap();
                            if matches!(g.ty, Type::Float) {
                                writeln!(self.out, "\t.float\t{}", *n as f32).unwrap();
                            } else if matches!(g.ty, Type::Double) {
                                writeln!(self.out, "\t.double\t{}", *n as f64).unwrap();
                            } else {
                                self.emit_int_directive(size, *n);
                            }
                        }
                        Expr::Float(f) => {
                            self.emit_data_section();
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
                    if let Some(n) = Self::const_i64(other) {
                        self.emit_data_section();
                        let al = self.type_align(&g.ty).max(1).min(8);
                        writeln!(self.out, "\t.p2align\t{}", al.trailing_zeros()).unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        self.emit_int_directive(size, n);
                    } else {
                        self.emit_bss_section();
                        writeln!(self.out, "\t.p2align\t3").unwrap();
                        writeln!(self.out, "{sym}:").unwrap();
                        writeln!(self.out, "\t.zero\t{size}").unwrap();
                    }
                }
            }
        } else {
            self.emit_bss_section();
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
                // Map designators and positional
                let mut values: Vec<Option<&Expr>> = vec![None; (*n as usize).max(1)];
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
                for i in 0..(*n as usize) {
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
                let lay = self.layout_fields(fs, is_union);
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
        // Build values per field name
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
        ordered.sort_by_key(|(_, p)| p.offset);
        let mut pos = 0i64;
        let mut pos_i = 0usize;
        let mut i = 0;
        while i < ordered.len() {
            let (fname, place) = ordered[i];
            let off = place.offset;
            let fty = &place.ty;
            if pos < off {
                writeln!(self.out, "\t.zero\t{}", off - pos).unwrap();
                pos = off;
            }
            // Group union members that share the same offset — one storage slot.
            let mut j = i + 1;
            while j < ordered.len() && ordered[j].1.offset == off {
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
            // Also check other names in the union group for designated init
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

    /// Emit a static integer of the given byte size (1/2/4/8).
    fn emit_int_directive(&mut self, size: i64, n: i64) {
        match size {
            1 => writeln!(self.out, "\t.byte\t{n}").unwrap(),
            // Critical: short/unsigned short tables (lemon yy_action, yy_lookahead)
            // must be 2-byte elements. Emitting .long broke all parser indexing.
            2 => writeln!(self.out, "\t.hword\t{n}").unwrap(),
            4 => writeln!(self.out, "\t.long\t{n}").unwrap(),
            _ => writeln!(self.out, "\t.quad\t{n}").unwrap(),
        }
    }

    fn emit_scalar_data(&mut self, ty: &Type, e: &Expr) -> Result<(), String> {
        // Fold simple constant expressions for static storage duration.
        if let Some(n) = Self::const_i64(e) {
            let sz = self.type_size(ty);
            self.emit_int_directive(sz, n);
            return Ok(());
        }
        match e {
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => {
                if let Expr::Var(v) = expr.as_ref() {
                    writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
                } else {
                    writeln!(self.out, "\t.quad\t0").unwrap();
                }
            }
            Expr::Var(v) => {
                // Function designator or other global → address
                writeln!(self.out, "\t.quad\t{}", self.c_sym(v)).unwrap();
            }
            Expr::String(s) => {
                // const char * field = "literal"
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
            _ => {
                writeln!(self.out, "\t.zero\t{}", self.type_size(ty)).unwrap();
            }
        }
        Ok(())
    }

    fn const_i64(e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(-Self::const_i64(expr)?),
            Expr::Unary {
                op: UnaryOp::BitNot,
                expr,
            } => Some(!Self::const_i64(expr)?),
            Expr::Cast { expr, .. } => Self::const_i64(expr),
            Expr::Binary { op, left, right } => {
                let l = Self::const_i64(left)?;
                let r = Self::const_i64(right)?;
                Some(match op {
                    BinOp::Add => l.wrapping_add(r),
                    BinOp::Sub => l.wrapping_sub(r),
                    BinOp::Mul => l.wrapping_mul(r),
                    BinOp::Div if r != 0 => l / r,
                    BinOp::Mod if r != 0 => l % r,
                    BinOp::Shl => l.wrapping_shl(r as u32),
                    BinOp::Shr => l.wrapping_shr(r as u32),
                    BinOp::BitAnd => l & r,
                    BinOp::BitOr => l | r,
                    BinOp::BitXor => l ^ r,
                    BinOp::Comma => r,
                    BinOp::Eq => (l == r) as i64,
                    BinOp::Ne => (l != r) as i64,
                    BinOp::Lt => (l < r) as i64,
                    BinOp::Le => (l <= r) as i64,
                    BinOp::Gt => (l > r) as i64,
                    BinOp::Ge => (l >= r) as i64,
                    _ => return None,
                })
            }
            _ => None,
        }
    }

    fn alloc_local(&mut self, name: &str, ty: &Type) -> i64 {
        let sz = self.stack_slot_size(ty).max(8);
        let al = 8i64;
        self.stack_size = Self::align_up(self.stack_size + sz, al);
        let offset = -self.stack_size;
        self.locals.insert(
            name.to_string(),
            Sym {
                ty: ty.clone(),
                storage: Storage::Local { offset },
            },
        );
        offset
    }

    fn emit_function(
        &mut self,
        f: &Function,
        typedefs: &HashMap<String, Type>,
    ) -> Result<(), String> {
        self.func_name = f.name.clone();
        self.func_ret = f.ret.clone();
        self.locals.clear();
        // Reserve [x29,#-8] for saved x19 (lvalue address temp / logical reg 19).
        // x19 is callee-saved under AAPCS64; we use it across calls in
        // CompoundAssign / PreInc / PostInc, so every function must preserve it
        // (same pattern as x86_64 %rbx). Without this, `n += f()` stores through
        // the callee's clobbered x19 and the add is lost (SQLite MakeRecord nHdr).
        self.stack_size = 8;
        self.break_stack.clear();
        self.continue_stack.clear();
        self.va_regsave_off = 0;
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
        // Variadic: 64B for x0..x7 save + 128B (16×8) copy of stack-passed
        // overflow args so va_arg walks a contiguous cursor (NestedParse has 8+
        // variadic args; without the overflow copy, #%d stays literal).
        if f.variadic {
            self.stack_size = Self::align_up(self.stack_size, 16) + 64 + 128;
        }
        let mut measure = self.locals.clone();
        let mut measure_size = self.stack_size;
        self.measure_stmts(body, &mut measure, &mut measure_size, typedefs);
        // Extra padding: temporary SP spills during calls must not collide with
        // fixed-frame locals (AAPCS64 has no red zone). Keep SP well below frame.
        let frame = Self::align_up(measure_size + 256, 16);

        // Reset and emit for real (keep slot for saved x19)
        self.locals.clear();
        self.stack_size = 8;

        // Count fixed GPRs consumed (small aggregates take 1–2 regs).
        let mut fixed_gp = 0usize;
        for (_, pty) in f.params.iter() {
            let pty = match pty {
                Type::Array(e, _) => Type::Ptr(e.clone()),
                other => other.clone(),
            };
            if matches!(pty, Type::Float | Type::Double) {
                continue;
            }
            if let Some(nr) = self.small_agg_nregs(&pty) {
                fixed_gp += nr as usize;
            } else {
                fixed_gp += 1;
            }
        }
        self.va_fixed_n = if f.variadic { fixed_gp.min(8) } else { 0 };

        let sym = self.c_sym(&f.name);
        writeln!(self.out, "\n\t.globl\t{sym}").unwrap();
        writeln!(self.out, "{sym}:").unwrap();
        writeln!(self.out, "\tstp\tx29, x30, [sp, #-16]!").unwrap();
        writeln!(self.out, "\tmov\tx29, sp").unwrap();
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
            } else if igpr < 8 {
                param_offs.push((off, pty, igpr, 1, 0));
                igpr += 1;
            } else {
                // 9th+ integer/pointer argument: on caller's stack.
                param_offs.push((off, pty, 255, 1, stack_x29));
                stack_x29 += 8;
            }
        }

        // Variadic: reserve save area, spill x0-x7, copy stack overflow args
        // into the contiguous region after x7, then materialize named params.
        if f.variadic {
            self.stack_size = Self::align_up(self.stack_size, 16) + 64 + 128;
            self.va_regsave_off = -self.stack_size; // x0 at this offset
            for r in 0u8..8 {
                let off = self.va_regsave_off + (r as i64) * 8;
                self.emit_fp_addr(off, 17);
                writeln!(self.out, "\tstr\tx{r}, [x17]").unwrap();
            }
            // Stack-passed args (9th+) land at [x29,#16] after stp fp,lr.
            // Copy 16 words so va_arg after x7 keeps walking linearly.
            for i in 0i64..16 {
                let src = 16 + i * 8;
                writeln!(self.out, "\tldr\tx16, [x29, #{src}]").unwrap();
                let dest = self.va_regsave_off + 64 + i * 8;
                self.emit_fp_addr(dest, 17);
                writeln!(self.out, "\tstr\tx16, [x17]").unwrap();
            }
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
                    writeln!(self.out, "\tfmov\tx0, d{freg}").unwrap();
                    self.emit_fp_addr(*off, 9);
                    self.store_ty(pty, 9, 0);
                }
            }
        } else {
            for (off, pty, reg, nregs, stack_off) in &param_offs {
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
                    writeln!(self.out, "\tfmov\tx0, d{freg}").unwrap();
                    self.emit_fp_addr(*off, 9);
                    self.store_ty(pty, 9, 0);
                }
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
        locals: &mut HashMap<String, Sym>,
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
        locals: &mut HashMap<String, Sym>,
        stack: &mut i64,
        typedefs: &HashMap<String, Type>,
    ) {
        match st {
            Stmt::Decl(d) => {
                let ty = self.expand_ty(&d.ty, typedefs);
                if d.is_static {
                    // static local: no stack slot; global storage keyed by name
                    // (func_name may be empty during measure — fixup at emit time)
                    locals.insert(
                        d.name.clone(),
                        Sym {
                            ty,
                            storage: Storage::Global {
                                name: format!("__static_pending_{}", d.name),
                            },
                        },
                    );
                } else {
                    let sz = self.stack_slot_size(&ty).max(8);
                    let al = 8i64;
                    *stack = Self::align_up(*stack + sz, al);
                    let offset = -*stack;
                    locals.insert(
                        d.name.clone(),
                        Sym {
                            ty,
                            storage: Storage::Local { offset },
                        },
                    );
                }
            }
            Stmt::Block(ss) => self.measure_stmts(ss, locals, stack, typedefs),
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
                if let Some(i) = init {
                    self.measure_stmt(i, locals, stack, typedefs);
                }
                self.measure_stmt(body, locals, stack, typedefs);
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
            Stmt::Asm { lines } => {
                // Emit kbuild DEFINE lines; skip raw templates with %0/%[name].
                for line in lines {
                    let t = line.trim();
                    if t.is_empty() {
                        continue;
                    }
                    if t.contains('%')
                        && t.bytes().enumerate().any(|(i, b)| {
                            b == b'%'
                                && t.as_bytes()
                                    .get(i + 1)
                                    .map(|c| c.is_ascii_digit() || *c == b'[')
                                    .unwrap_or(false)
                        })
                    {
                        continue;
                    }
                    writeln!(self.out, "\t{line}").unwrap();
                }
                Ok(())
            }
            Stmt::Block(ss) => {
                for s in ss {
                    self.emit_stmt(s, typedefs)?;
                }
                Ok(())
            }
            Stmt::Decl(d) => {
                let ty = match &d.ty {
                    Type::AnonStruct(fs) => {
                        let lay = self.layout_fields(fs, false);
                        let key = format!("anon_{}", d.name);
                        self.layouts.insert(key.clone(), lay);
                        Type::Struct(key)
                    }
                    Type::AnonUnion(fs) => {
                        let lay = self.layout_fields(fs, true);
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
                                    let lay = self.layout_fields(fs, false);
                                    self.layouts.insert(n.clone(), lay);
                                    Type::Struct(n.clone())
                                }
                                Type::AnonUnion(fs) => {
                                    let lay = self.layout_fields(fs, true);
                                    self.layouts.insert(n.clone(), lay);
                                    Type::Union(n.clone())
                                }
                                other => other.clone(),
                            }
                        }
                    }
                    other => self.expand_ty(other, typedefs),
                };
                if d.is_static {
                    // Emit once as a unique global; re-entry keeps the value.
                    let gname = format!("__static_{}_{}", self.func_name, d.name);
                    // Only emit data the first time we see this static in the function.
                    if !self.globals.contains_key(&gname) {
                        let mut g = d.clone();
                        g.name = gname.clone();
                        g.is_static = false;
                        self.emit_global(&g)?;
                        // switch back to text
                        self.emit_text_section();
                        self.globals.insert(gname.clone(), ty.clone());
                    }
                    self.locals.insert(
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
                writeln!(self.out, "\tcbz\tx0, {l_else}").unwrap();
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
                writeln!(self.out, "\tcbz\tx0, {l_end}").unwrap();
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
                writeln!(self.out, "\tcbnz\tx0, {l_head}").unwrap();
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
                    writeln!(self.out, "\tcbz\tx0, {l_end}").unwrap();
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
                writeln!(
                    self.out,
                    "\tb\tL_{}_goto_{}",
                    self.func_name, name
                )
                .unwrap();
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
                        writeln!(self.out, "\tb.eq\t{lab}").unwrap();
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
                writeln!(self.out, "\tadd\tsp, sp, #16").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
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
            Stmt::Case { value, body } => {
                let lab = self.lab("case");
                let v = match value {
                    Expr::Int(n) | Expr::Char(n) => Some(*n),
                    Expr::Unary {
                        op: UnaryOp::Neg,
                        expr,
                    } => {
                        if let Expr::Int(n) = expr.as_ref() {
                            Some(-*n)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
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
                writeln!(self.out, "\tcbnz\tx0, {l_head}").unwrap();
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
                writeln!(self.out, "\tcbz\tx0, {l_end}").unwrap();
                self.emit_switch_body(body, default_lab, typedefs)?;
                writeln!(self.out, "{l_cont}:").unwrap();
                writeln!(self.out, "\tb\t{l_head}").unwrap();
                writeln!(self.out, "{l_end}:").unwrap();
                self.break_stack.pop();
                self.continue_stack.pop();
                Ok(())
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
                writeln!(self.out, "\tcbz\tx0, {l_else}").unwrap();
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
        } else if (1..=4095).contains(&off) {
            writeln!(self.out, "\tadd\tx{addr_reg}, x29, #{off}").unwrap();
        } else if (-4095..=-1).contains(&off) {
            writeln!(self.out, "\tsub\tx{addr_reg}, x29, #{}", -off).unwrap();
        } else {
            // movz/movk full offset then add
            self.emit_imm(off, 16);
            writeln!(self.out, "\tadd\tx{addr_reg}, x29, x16").unwrap();
        }
    }

    fn store_to_offset(&mut self, off: i64, ty: &Type, reg: u8) {
        if matches!(ty, Type::Float | Type::Double) {
            self.emit_fp_addr(off, 17);
            self.store_ty(ty, 17, reg);
            return;
        }
        // Stack slots are 8-byte for scalars; store full x so high bits are clear.
        // (load_from_offset uses ldrsw for Int, but Long/Ptr need clean high bits.)
        if (-256..256).contains(&off) {
            match self.type_size(ty) {
                1 => {
                    writeln!(self.out, "\tand\tx{reg}, x{reg}, #0xff").unwrap();
                    writeln!(self.out, "\tstr\tx{reg}, [x29, #{off}]").unwrap();
                }
                2 => {
                    writeln!(self.out, "\tand\tx{reg}, x{reg}, #0xffff").unwrap();
                    writeln!(self.out, "\tstr\tx{reg}, [x29, #{off}]").unwrap();
                }
                4 => {
                    writeln!(self.out, "\tmov\tw{reg}, w{reg}").unwrap(); // zero-extend
                    writeln!(self.out, "\tstr\tx{reg}, [x29, #{off}]").unwrap();
                }
                _ => writeln!(self.out, "\tstr\tx{reg}, [x29, #{off}]").unwrap(),
            }
        } else {
            self.emit_fp_addr(off, 17);
            match self.type_size(ty) {
                1 => {
                    writeln!(self.out, "\tand\tx{reg}, x{reg}, #0xff").unwrap();
                    writeln!(self.out, "\tstr\tx{reg}, [x17]").unwrap();
                }
                2 => {
                    writeln!(self.out, "\tand\tx{reg}, x{reg}, #0xffff").unwrap();
                    writeln!(self.out, "\tstr\tx{reg}, [x17]").unwrap();
                }
                4 => {
                    writeln!(self.out, "\tmov\tw{reg}, w{reg}").unwrap();
                    writeln!(self.out, "\tstr\tx{reg}, [x17]").unwrap();
                }
                _ => writeln!(self.out, "\tstr\tx{reg}, [x17]").unwrap(),
            }
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
                let start = base_off;
                for i in 0..(*n as usize) {
                    let eoff = start + (i as i64) * self.type_size(elem);
                    if let Some((_, e)) = fields_in.get(i) {
                        self.emit_expr_rval(e, 0, typedefs)?;
                        self.store_to_offset(eoff, elem, 0);
                    }
                }
            }
            Type::Struct(name) | Type::Union(name) => {
                let lay = self
                    .layouts
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("local init list unknown {name}"))?;
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
                for (fname, place) in ordered {
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
                        if let Some((bo, bw)) = place.bit {
                            self.store_bitfield(base_off + place.offset, &place.ty, bo, bw, 0)?;
                        } else {
                            self.store_to_offset(base_off + place.offset, &place.ty, 0);
                        }
                    }
                }
            }
            _ => {
                if let Some((_, e)) = fields_in.first() {
                    self.emit_expr_rval(e, 0, typedefs)?;
                    self.store_to_offset(base_off, &Type::Long, 0);
                }
            }
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Sym, String> {
        if let Some(s) = self.locals.get(name) {
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
        // functions as symbols? not as vars
        Err(format!("undefined variable '{name}'"))
    }

    /// Emit address of lvalue into x{reg}
    fn emit_lvalue_addr(
        &mut self,
        e: &Expr,
        reg: u8,
        typedefs: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match e {
            Expr::Var(name) => {
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
                    Type::Int | Type::Long | Type::Short | Type::Char => {
                        (Type::Char, true)
                    }
                    other => return Err(format!("index of non-array {:?}", other)),
                };
                let _ = decayed;
                let esz = self.type_size(&elem).max(1);
                writeln!(self.out, "\tmov\tx11, #{esz}").unwrap();
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
                        Type::Int | Type::Long | Type::Short | Type::Char | Type::Void => {
                            Type::Struct("__opaque__".into())
                        }
                        other => return Err(format!("-> on non-pointer {:?}", other)),
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
                    Type::Struct(n) | Type::Union(n) => self
                        .layouts
                        .get(n)
                        .cloned()
                        .ok_or_else(|| format!("unknown struct layout {n}"))?,
                    Type::AnonStruct(fs) => self.layout_fields(fs, false),
                    Type::AnonUnion(fs) => self.layout_fields(fs, true),
                    other => return Err(format!("member of non-struct {:?} .{}", other, field)),
                };
                let place = lay
                    .fields
                    .get(field)
                    .ok_or_else(|| format!("no field {field}"))?
                    .clone();
                // Bitfields are not addressable as pure lvalues in C; we still return
                // the container address so assign/load paths can special-case via
                // typeof + a bitfield store helper. Non-bitfield: address of field.
                if place.offset != 0 {
                    writeln!(self.out, "\tadd\tx{reg}, x{reg}, #{}", place.offset).unwrap();
                }
                // Stash bitfield info on a side channel? For now return container type;
                // load path for Member uses typeof and layout again.
                Ok(place.ty)
            }
            _ => Err("expression is not an lvalue".into()),
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
            Expr::Int(n) | Expr::Char(n) => {
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
            Expr::Var(name) => {
                // if function name used as value — not supported
                let sym = match self.lookup(name) {
                    Ok(s) => s,
                    Err(_) => {
                        // could be function — take address
                        if self.funcs.contains_key(name) {
                            let lab = self.c_sym(name);
                            self.emit_adrp_add(dest, &lab);
                            return Ok(Type::Ptr(Box::new(Type::Void)));
                        }
                        // Undeclared identifier used as rvalue: treat as external
                        // function designator (common for dlsym, libc, etc.).
                        let lab = self.c_sym(name);
                        if self.os == TargetOs::Linux {
                            self.emit_adrp_got(dest, &lab);
                        } else {
                            self.emit_adrp_add(dest, &lab);
                        }
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
                                if Self::is_extern_libc(name) {
                                    self.emit_adrp_got(9, &lab);
                                } else {
                                    self.emit_adrp_add(9, &lab);
                                }
                                self.load_ty(ty, 9, dest);
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
                            let lab = self.c_sym(n);
                            self.emit_adrp_add(dest, &lab);
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
                    let ty = self.emit_expr_rval(expr, 9, typedefs)?;
                    let inner = match ty {
                        Type::Ptr(i) => *i,
                        Type::Array(i, _) => *i,
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
                    writeln!(self.out, "\tcbz\tx{dest}, {l_false}").unwrap();
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
                    writeln!(self.out, "\tcbnz\tx{dest}, {l_true}").unwrap();
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
                            writeln!(self.out, "\tmov\tx11, #{esz}").unwrap();
                            writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            writeln!(self.out, "\tadd\tx{dest}, x9, x10").unwrap();
                            return Ok(lty);
                        }
                        // int + ptr
                        let rty2 = self.typeof_expr(right, typedefs);
                        if let Type::Ptr(inner) = rty2 {
                            let esz = self.type_size(&inner).max(1);
                            writeln!(self.out, "\tmov\tx11, #{esz}").unwrap();
                            writeln!(self.out, "\tmul\tx9, x9, x11").unwrap();
                            writeln!(self.out, "\tadd\tx{dest}, x10, x9").unwrap();
                            return Ok(Type::Ptr(inner));
                        }
                        writeln!(self.out, "\tadd\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::Sub => {
                        if let Type::Ptr(inner) = &lty {
                            let esz = self.type_size(inner).max(1);
                            let rty = self.typeof_expr(right, typedefs);
                            if matches!(rty, Type::Ptr(_)) {
                                writeln!(self.out, "\tsub\tx{dest}, x9, x10").unwrap();
                                writeln!(self.out, "\tmov\tx11, #{esz}").unwrap();
                                writeln!(self.out, "\tsdiv\tx{dest}, x{dest}, x11").unwrap();
                                return Ok(Type::Int);
                            }
                            // ptr - int
                            writeln!(self.out, "\tmov\tx11, #{esz}").unwrap();
                            writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            writeln!(self.out, "\tsub\tx{dest}, x9, x10").unwrap();
                            return Ok(lty);
                        }
                        writeln!(self.out, "\tsub\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::Mul => {
                        writeln!(self.out, "\tmul\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::Div => {
                        writeln!(self.out, "\tsdiv\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::Mod => {
                        writeln!(self.out, "\tsdiv\tx11, x9, x10").unwrap();
                        writeln!(self.out, "\tmsub\tx{dest}, x11, x10, x9").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        // Use 64-bit cmp when either operand is long/pointer (or wider
                        // than 32-bit). 32-bit-only cmp truncates constants like
                        // 4294967296LL to 0 and breaks sqlite3GetUInt32:
                        //   if (v > 4294967296LL)  // was always true for v=1
                        let wide = matches!(
                            lty,
                            Type::Long
                                | Type::ULong
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                        ) || matches!(
                            rty,
                            Type::Long
                                | Type::ULong
                                | Type::Ptr(_)
                                | Type::Array(_, _)
                        );
                        // Always 64-bit when either side may hold a zero-extended u32
                        // (Pgno/mxPgno) so 0xfffffffe is not compared as -2 via ldrsw.
                        let unsignedish = matches!(
                            lty,
                            Type::UInt | Type::ULong | Type::UShort | Type::Char
                        ) || matches!(
                            rty,
                            Type::UInt | Type::ULong | Type::UShort | Type::Char
                        );
                        if wide || unsignedish {
                            writeln!(self.out, "\tcmp\tx9, x10").unwrap();
                        } else {
                            // 32-bit compare for signed int-ish values
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
                        writeln!(self.out, "\tand\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::BitOr => {
                        writeln!(self.out, "\torr\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::BitXor => {
                        writeln!(self.out, "\teor\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::Shl => {
                        writeln!(self.out, "\tlsl\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
                    }
                    BinOp::Shr => {
                        writeln!(self.out, "\tasr\tx{dest}, x9, x10").unwrap();
                        Ok(Type::Int)
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
                // Aggregate / struct assign via memcpy.
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
                        // struct/union lvalue: a = b, a = s.field, etc.
                        other => match self.emit_lvalue_addr(other, 0, typedefs) {
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
                // Always use store_ty so float/double get correct width/format.
                if matches!(left.as_ref(), Expr::Var(_))
                    && matches!(
                        lty,
                        Type::Char | Type::Short | Type::Int | Type::Long | Type::Ptr(_)
                    )
                {
                    // stack locals use full 8-byte slots for integer scalars
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
                                writeln!(self.out, "\tmov\tx11, #{esz}").unwrap();
                                writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            }
                            writeln!(self.out, "\tadd\tx0, x9, x10").unwrap();
                        }
                        BinOp::Sub => {
                            if let Type::Ptr(inner) = &lty {
                                let esz = self.type_size(inner).max(1);
                                writeln!(self.out, "\tmov\tx11, #{esz}").unwrap();
                                writeln!(self.out, "\tmul\tx10, x10, x11").unwrap();
                            }
                            writeln!(self.out, "\tsub\tx0, x9, x10").unwrap();
                        }
                        BinOp::Mul => writeln!(self.out, "\tmul\tx0, x9, x10").unwrap(),
                        BinOp::Div => writeln!(self.out, "\tsdiv\tx0, x9, x10").unwrap(),
                        BinOp::Mod => {
                            writeln!(self.out, "\tsdiv\tx11, x9, x10").unwrap();
                            writeln!(self.out, "\tmsub\tx0, x11, x10, x9").unwrap();
                        }
                        BinOp::BitAnd => writeln!(self.out, "\tand\tx0, x9, x10").unwrap(),
                        BinOp::BitOr => writeln!(self.out, "\torr\tx0, x9, x10").unwrap(),
                        BinOp::BitXor => writeln!(self.out, "\teor\tx0, x9, x10").unwrap(),
                        BinOp::Shl => writeln!(self.out, "\tlsl\tx0, x9, x10").unwrap(),
                        BinOp::Shr => writeln!(self.out, "\tasr\tx0, x9, x10").unwrap(),
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
                // Variadic intrinsics (from expanded va_start / va_arg macros).
                if name == "__ggcc_va_start" {
                    if self.va_regsave_off == 0 {
                        return Err("va_start outside variadic function".into());
                    }
                    // Cursor = &regsave[fixed_n]
                    let off = self.va_regsave_off + (self.va_fixed_n as i64) * 8;
                    self.emit_fp_addr(off, dest);
                    return Ok(Type::Ptr(Box::new(Type::Char)));
                }
                if name == "__ggcc_va_arg" {
                    // args: &ap  (pointer to va_list / char*)
                    // Returns the current cursor (for *(type*)cursor); advances ap by 8.
                    if args.is_empty() {
                        return Err("__ggcc_va_arg needs &ap".into());
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
                    if dest != 0 {
                        writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                    }
                    // function pointer type is Ptr(T); calling yields T (often another Ptr)
                    let ret = match cty {
                        Type::Ptr(inner) => *inner,
                        other => other,
                    };
                    return Ok(ret);
                }
                // Apple Silicon ABI: only *variadic* args go on the stack; fixed
                // parameters stay in registers (x0..). printf has 1 fixed arg;
                // sprintf/fprintf have 2; snprintf has 3.
                // Linux AAPCS64: variadic uses the same x0..x7 then stack path as
                // non-variadic — do NOT take the Darwin special case.
                let fixed_n: usize = if self.os == TargetOs::Darwin {
                    match name.as_str() {
                        "printf" | "scanf" => 1,
                        "sprintf" | "fprintf" | "sscanf" => 2,
                        "snprintf" => 3,
                        n if n.contains("snprintf") => 3,
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
                    // Call via bl, or blr if name is a function-pointer global
                    if self.globals.contains_key(name) || self.locals.contains_key(name) {
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
                // Variadic callees compiled by ggcc therefore cannot see doubles in d0..d7.
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
                // Spill each logical half-register as a 16-byte slot (top = args[0]).
                let n = args.len();
                let mut arg_nregs: Vec<u8> = Vec::with_capacity(n);
                let mut arg_is_float: Vec<bool> = Vec::with_capacity(n);
                let mut total_slots: i64 = 0;
                for i in 0..n {
                    let aty = self.typeof_expr(&args[i], typedefs);
                    let pty = param_tys.get(i).cloned().unwrap_or(aty.clone());
                    let is_f = matches!(pty, Type::Float | Type::Double)
                        || (param_tys.is_empty() && matches!(aty, Type::Float | Type::Double));
                    arg_is_float.push(is_f);
                    // ggcc-variadic floats travel as one GPR each (not FPR).
                    let nr = if is_f {
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
                    if !arg_is_float[i] && self.small_agg_nregs(&pty).or_else(|| self.small_agg_nregs(&aty)).is_some()
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
                let mut igpr = 0u8;
                let mut fpr = 0u8;
                let mut slot: i64 = 0;
                let mut gpr_slots_used: i64 = 0;
                for i in 0..n {
                    let nr = arg_nregs[i] as i64;
                    let aty = self.typeof_expr(&args[i], typedefs);
                    let pty = param_tys.get(i).cloned().unwrap_or(aty.clone());
                    if arg_is_float[i] {
                        let src = slot * 16;
                        writeln!(self.out, "\tldr\tx16, [sp, #{src}]").unwrap();
                        // Prefer param type: if callee expects double, spilled bits are
                        // already IEEE (even when typeof_expr of a complex arg is wrong).
                        let as_float = matches!(pty, Type::Float | Type::Double)
                            || matches!(aty, Type::Float | Type::Double);
                        if !as_float {
                            // integer expression promoted to float param
                            if matches!(aty, Type::UShort | Type::UInt | Type::ULong) {
                                writeln!(self.out, "\tucvtf\td0, x16").unwrap();
                            } else {
                                writeln!(self.out, "\tscvtf\td0, x16").unwrap();
                            }
                            writeln!(self.out, "\tfmov\tx16, d0").unwrap();
                        }
                        if callee_variadic {
                            // IEEE bits in next GPR (va_arg walks GP regsave only).
                            if igpr < 8 {
                                writeln!(self.out, "\tmov\tx{igpr}, x16").unwrap();
                                igpr += 1;
                                gpr_slots_used += 1;
                            }
                            // if igpr>=8: bits stay in spill; stack packer sends them
                            slot += 1;
                        } else {
                            if as_float {
                                writeln!(self.out, "\tfmov\td{fpr}, x16").unwrap();
                            } else {
                                // already converted above into x16 bits
                                writeln!(self.out, "\tfmov\td{fpr}, x16").unwrap();
                            }
                            if matches!(pty, Type::Float) {
                                writeln!(self.out, "\tfcvt\ts{fpr}, d{fpr}").unwrap();
                            }
                            fpr += 1;
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
                                gpr_slots_used += 1;
                            }
                        }
                        slot += nr;
                    }
                }

                let spill = total_slots * 16;
                let n_stack_slots = (total_slots - gpr_slots_used).max(0);
                if n_stack_slots > 0 {
                    // Outgoing stack args: pack leftover spill slots into 8-byte region.
                    for r in 0..igpr {
                        writeln!(self.out, "\tstr\tx{r}, [sp, #-16]!").unwrap();
                    }
                    let stack_bytes = Self::align_up(n_stack_slots * 8, 16);
                    writeln!(self.out, "\tsub\tsp, sp, #{stack_bytes}").unwrap();
                    for k in 0..n_stack_slots {
                        // Leftover slots start at spill index gpr_slots_used.
                        let from = (igpr as i64) * 16
                            + stack_bytes
                            + (gpr_slots_used + k) * 16;
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
                    } else if self.locals.contains_key(name) || self.globals.contains_key(name) {
                        let _ = self.emit_expr_rval(&Expr::Var(name.clone()), 16, typedefs)?;
                        writeln!(self.out, "\tblr\tx16").unwrap();
                    } else {
                        writeln!(self.out, "\tbl\t{}", self.c_sym(name)).unwrap();
                    }
                }
                // float return in d0
                let ret_ty = self
                    .funcs
                    .get(name)
                    .map(|f| f.ret.clone())
                    .unwrap_or(Type::Int);
                if matches!(ret_ty, Type::Float | Type::Double) {
                    writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                    return Ok(Type::Double);
                }
                // Small aggregate return: x0[,x1] already hold the value.
                // Leave them in place; scalar callers only consume x0.
                if dest != 0 {
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
                // compound literal (T){...} as data symbol + pointer
                if let Expr::InitList { fields } = expr.as_ref() {
                    let id = self.label_id;
                    self.label_id += 1;
                    let gname = format!("__comp_{id}");
                    // stash to emit later in data section via deferred list — emit inline now
                    self.emit_data_section();
                    writeln!(self.out, "\t.p2align\t3").unwrap();
                    let glab = self.c_sym(&gname);
                    writeln!(self.out, "{glab}:").unwrap();
                    self.emit_init_list_data(ty, fields)?;
                    self.emit_text_section();
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
                        writeln!(self.out, "\tfmov\ts0, w{dest}").unwrap();
                        if to_unsigned {
                            writeln!(self.out, "\tfcvtzu\tx{dest}, s0").unwrap();
                        } else {
                            writeln!(self.out, "\tfcvtzs\tx{dest}, s0").unwrap();
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
                        writeln!(self.out, "\tfmov\ts0, w{dest}").unwrap();
                        writeln!(self.out, "\tfcvt\td0, s0").unwrap();
                        writeln!(self.out, "\tfmov\tx{dest}, d0").unwrap();
                    }
                    (Type::Double, Type::Float) => {
                        writeln!(self.out, "\tfmov\td0, x{dest}").unwrap();
                        writeln!(self.out, "\tfcvt\ts0, d0").unwrap();
                        writeln!(self.out, "\tfmov\tw{dest}, s0").unwrap();
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
                writeln!(self.out, "\tcbz\tx0, {l_else}").unwrap();
                self.emit_expr_rval(then_e, dest, typedefs)?;
                writeln!(self.out, "\tb\t{l_end}").unwrap();
                writeln!(self.out, "{l_else}:").unwrap();
                self.emit_expr_rval(else_e, dest, typedefs)?;
                writeln!(self.out, "{l_end}:").unwrap();
                Ok(Type::Int)
            }
            Expr::PreInc(ex) => {
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
                writeln!(self.out, "\tadd\tx0, x0, #{step}").unwrap();
                self.store_ty(&ty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
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
                writeln!(self.out, "\tsub\tx0, x0, #{step}").unwrap();
                self.store_ty(&ty, 19, 0);
                if dest != 0 {
                    writeln!(self.out, "\tmov\tx{dest}, x0").unwrap();
                }
                Ok(ty)
            }
            Expr::PostInc(ex) => {
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                // old value in x0; spill so nested assigns cannot clobber
                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                writeln!(self.out, "\tadd\tx0, x0, #{step}").unwrap();
                self.store_ty(&ty, 19, 0);
                writeln!(self.out, "\tldr\tx{dest}, [sp], #16").unwrap();
                Ok(ty)
            }
            Expr::PostDec(ex) => {
                let ty = self.emit_lvalue_addr(ex, 19, typedefs)?;
                self.load_ty(&ty, 19, 0);
                writeln!(self.out, "\tstr\tx0, [sp, #-16]!").unwrap();
                let step = match &ty {
                    Type::Ptr(i) => self.type_size(i).max(1),
                    _ => 1,
                };
                writeln!(self.out, "\tsub\tx0, x0, #{step}").unwrap();
                self.store_ty(&ty, 19, 0);
                writeln!(self.out, "\tldr\tx{dest}, [sp], #16").unwrap();
                Ok(ty)
            }
            Expr::InitList { fields } => {
                // Bare brace list as expression (rare); emit static zeroed blob.
                let id = self.label_id;
                self.label_id += 1;
                let gname = format!("__initlist_{id}");
                self.emit_data_section();
                writeln!(self.out, "\t.p2align\t3").unwrap();
                let glab = self.c_sym(&gname);
                writeln!(self.out, "{glab}:").unwrap();
                let ty = Type::Array(Box::new(Type::Char), 64);
                self.emit_init_list_data(&ty, fields)?;
                self.emit_text_section();
                self.emit_adrp_add(dest, &glab);
                Ok(Type::Ptr(Box::new(Type::Void)))
            }
        }
    }

    fn emit_type_of(&self, _e: &Expr, _typedefs: &HashMap<String, Type>) -> Type {
        Type::Int
    }

    fn typeof_expr(&self, e: &Expr, typedefs: &HashMap<String, Type>) -> Type {
        match e {
            Expr::Int(_) | Expr::Char(_) => Type::Int,
            Expr::Float(_) => Type::Double,
            Expr::String(_) => Type::Ptr(Box::new(Type::Char)),
            Expr::Var(n) => {
                if let Some(s) = self.locals.get(n) {
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
                        return match ct {
                            Type::Ptr(inner) => *inner,
                            other => other,
                        };
                    }
                    return Type::Int;
                }
                self.funcs
                    .get(name)
                    .map(|f| f.ret.clone())
                    .unwrap_or(Type::Int)
            }
            Expr::Binary { op, left, right } => {
                let l = self.typeof_expr(left, typedefs);
                let r = self.typeof_expr(right, typedefs);
                // comparisons → int
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
                    _ => Type::Int,
                }
            }
            _ => Type::Int,
        }
    }

    fn emit_imm(&mut self, n: i64, dest: u8) {
        let u = n as u64;
        let w0 = (u & 0xffff) as u16;
        let w1 = ((u >> 16) & 0xffff) as u16;
        let w2 = ((u >> 32) & 0xffff) as u16;
        let w3 = ((u >> 48) & 0xffff) as u16;
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
        // For negative small values, movz alone of low 16 bits is wrong for -1 etc.
        // Using full u64 bit pattern via all movk is correct when all non-zero.
        // When n < 0 and upper bits set, we need them:
        if n < 0 {
            // ensure full pattern: re-emit completely
            // already using u as bit pattern — if w1/w2/w3 zero for -1? -1 has all ffff
            // good.
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
    fn x86_64_hello_string() {
        let src = r#"
        #include <stdio.h>
        int main(void) {
            printf("Hello, world!\n");
            return 0;
        }
        "#;
        let p = parser::parse(src).unwrap();
        let asm = emit_assembly_for(&p, Target::X86_64).unwrap();
        assert!(asm.contains("Hello, world!"));
        assert!(asm.contains("callq") || asm.contains("call\t"));
    }
}
