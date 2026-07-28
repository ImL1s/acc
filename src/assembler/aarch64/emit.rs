//! Pass over parsed `AsmUnit` lines and emit a relocatable ELF object.

use super::elf::{
    ElfObject, Reloc, Section, Symbol, R_AARCH64_ADD_ABS_LO12_NC, R_AARCH64_ADR_PREL_HI21,
    R_AARCH64_CALL26, SHF_ALLOC, SHF_EXECINSTR, SHF_WRITE, SHT_NOBITS, SHT_PROGBITS, STB_GLOBAL,
    STB_LOCAL, STB_WEAK, STT_FUNC, STT_NOTYPE, STT_OBJECT, STT_SECTION,
};
use super::encode::{
    encode_add_imm, encode_add_lo12_placeholder, encode_adrp_placeholder, encode_and_imm,
    encode_b_imm26, encode_bl_placeholder, encode_ldp_post, encode_ldr_reg, encode_mov_reg,
    encode_movz, encode_nop, encode_ret, encode_rev, encode_rev16, encode_stp_pre, encode_str_pre,
    encode_str_reg, encode_sub_imm, parse_reg, write_u32_le,
};
use crate::assembler::{AsmLine, AsmUnit};

pub fn emit_elf_object(unit: &AsmUnit) -> Result<Vec<u8>, String> {
    if unit.target_os != crate::codegen::TargetOs::Linux {
        return Err(format!(
            "builtin assembler M2: ELF emit only supports linux (got {})",
            unit.target_os.as_str()
        ));
    }
    let mut obj = ElfObject {
        sections: Vec::new(),
        symbols: Vec::new(),
    };
    let mut st = Emitter {
        obj: &mut obj,
        cur: ".text".to_string(),
        globl: std::collections::HashSet::new(),
        weak: std::collections::HashSet::new(),
        labels: std::collections::HashMap::new(),
        undefined: std::collections::HashSet::new(),
        lineno: 0,
        section_sizes: std::collections::HashMap::new(),
    };
    st.ensure_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 4);

    // Pass 1: record label offsets (forward branches need this).
    st.pass_labels(unit)?;

    // Pass 2: encode.
    for line in &unit.lines {
        st.lineno += 1;
        match line {
            AsmLine::Empty => {}
            AsmLine::Label(_) => {}
            AsmLine::Directive { name, args } => st.directive(name, args)?,
            AsmLine::Instr { mnemonic, operands } => {
                st.encode(mnemonic, operands)?;
            }
        }
    }

    st.finalize_symbols();
    Ok(obj.write())
}

struct Emitter<'a> {
    obj: &'a mut ElfObject,
    cur: String,
    globl: std::collections::HashSet<String>,
    weak: std::collections::HashSet<String>,
    labels: std::collections::HashMap<String, (String, u64)>,
    undefined: std::collections::HashSet<String>,
    lineno: usize,
    section_sizes: std::collections::HashMap<String, u64>,
}

impl<'a> Emitter<'a> {
    fn err(&self, msg: impl Into<String>) -> String {
        format!("assembler aarch64 M2:{}: {}", self.lineno, msg.into())
    }

    fn ensure_section(&mut self, name: &str, sh_type: u32, sh_flags: u64, align: u64) {
        self.obj.section_mut(name, sh_type, sh_flags, align);
    }

    /// Walk lines once to bind labels before encoding branches.
    fn pass_labels(&mut self, unit: &AsmUnit) -> Result<(), String> {
        for line in &unit.lines {
            self.lineno += 1;
            match line {
                AsmLine::Empty => {}
                AsmLine::Label(name) => {
                    self.define_label_pass1(name)?;
                }
                AsmLine::Directive { name, args } => {
                    self.pass_directive(name, args)?;
                }
                AsmLine::Instr { mnemonic, operands } => {
                    self.pass_insn_size(mnemonic, operands)?;
                }
            }
        }
        self.lineno = 0;
        Ok(())
    }

    fn pass_directive(&mut self, name: &str, args: &str) -> Result<(), String> {
        match name {
            ".text" => {
                self.cur = ".text".to_string();
                self.ensure_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 4);
            }
            ".data" => {
                self.cur = ".data".to_string();
                self.ensure_section(".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE, 8);
            }
            ".rodata" => {
                self.cur = ".rodata".to_string();
                self.ensure_section(".rodata", SHT_PROGBITS, SHF_ALLOC, 8);
            }
            ".bss" => {
                self.cur = ".bss".to_string();
                self.ensure_section(".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE, 8);
            }
            ".section" => {
                let part = args.split(',').next().unwrap_or("").trim();
                let sec = part.trim_matches('"');
                self.cur = sec.to_string();
                let flags = if sec.contains("text") {
                    SHF_ALLOC | SHF_EXECINSTR
                } else if sec.contains("bss") {
                    SHF_ALLOC | SHF_WRITE
                } else if sec.contains("rodata") {
                    SHF_ALLOC
                } else {
                    SHF_ALLOC | SHF_WRITE
                };
                let ty = if sec.contains("bss") {
                    SHT_NOBITS
                } else {
                    SHT_PROGBITS
                };
                self.ensure_section(sec, ty, flags, 8);
            }
            ".p2align" | ".align" => {
                let n: u32 = args
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| self.err("bad align"))?;
                self.align_size(n)?;
            }
            ".asciz" | ".string" => {
                let s = parse_string_literal(args)?;
                self.bump_size(s.len() + 1);
            }
            ".ascii" => {
                let s = parse_string_literal(args)?;
                self.bump_size(s.len());
            }
            ".zero" | ".space" => {
                let n: usize = args
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| self.err("bad .zero"))?;
                self.bump_size(n);
            }
            ".byte" => self.bump_size(args.split(',').count()),
            ".short" => self.bump_size(2 * args.split(',').count()),
            ".word" | ".long" => self.bump_size(4 * args.split(',').count()),
            ".quad" => self.bump_size(8 * args.split(',').count()),
            _ => {}
        }
        Ok(())
    }

    fn pass_insn_size(&mut self, mnemonic: &str, _operands: &str) -> Result<(), String> {
        if matches!(
            mnemonic,
            "ret"
                | "nop"
                | "mov"
                | "movz"
                | "add"
                | "sub"
                | "and"
                | "rev"
                | "rev16"
                | "stp"
                | "ldp"
                | "str"
                | "ldr"
                | "adrp"
                | "bl"
                | "b"
        ) {
            self.bump_size(4);
        } else {
            return Err(self.err(format!("M2 size pass: unsupported '{mnemonic}'")));
        }
        Ok(())
    }

    fn align_size(&mut self, log2: u32) -> Result<(), String> {
        let align = 1u64 << log2;
        let cur = self.size_off();
        let aligned = (cur + align - 1) & !(align - 1);
        *self.section_sizes.entry(self.cur.clone()).or_insert(0) = aligned;
        Ok(())
    }

    fn cur_sec(&mut self) -> &mut Section {
        self.obj
            .sections
            .iter_mut()
            .find(|s| s.name == self.cur)
            .expect("current section")
    }

    fn cur_off(&mut self) -> u64 {
        self.cur_sec().data.len() as u64
    }

    fn size_off(&self) -> u64 {
        *self.section_sizes.get(&self.cur).unwrap_or(&0)
    }

    fn bump_size(&mut self, n: usize) {
        *self.section_sizes.entry(self.cur.clone()).or_insert(0) += n as u64;
    }

    fn align_cur(&mut self, log2: u32) -> Result<(), String> {
        let align = 1u64 << log2;
        let sec = self
            .obj
            .sections
            .iter_mut()
            .find(|s| s.name == self.cur)
            .unwrap();
        let is_text = sec.sh_flags & SHF_EXECINSTR != 0;
        while sec.data.len() as u64 & (align - 1) != 0 {
            if is_text {
                write_u32_le(&mut sec.data, encode_nop());
            } else {
                sec.data.push(0);
            }
        }
        Ok(())
    }

    fn define_label_pass1(&mut self, name: &str) -> Result<(), String> {
        let off = self.size_off();
        if self
            .labels
            .insert(name.to_string(), (self.cur.clone(), off))
            .is_some()
        {
            return Err(self.err(format!("duplicate label '{name}'")));
        }
        Ok(())
    }

    fn directive(&mut self, name: &str, args: &str) -> Result<(), String> {
        match name {
            ".text" => {
                self.cur = ".text".to_string();
                self.ensure_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 4);
            }
            ".data" => {
                self.cur = ".data".to_string();
                self.ensure_section(".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE, 8);
            }
            ".rodata" => {
                self.cur = ".rodata".to_string();
                self.ensure_section(".rodata", SHT_PROGBITS, SHF_ALLOC, 8);
            }
            ".bss" => {
                self.cur = ".bss".to_string();
                self.ensure_section(".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE, 8);
            }
            ".section" => {
                let part = args.split(',').next().unwrap_or("").trim();
                let sec = part.trim_matches('"');
                self.cur = sec.to_string();
                let flags = if sec.contains("text") {
                    SHF_ALLOC | SHF_EXECINSTR
                } else if sec.contains("bss") {
                    SHF_ALLOC | SHF_WRITE
                } else if sec.contains("rodata") {
                    SHF_ALLOC
                } else {
                    SHF_ALLOC | SHF_WRITE
                };
                let ty = if sec.contains("bss") {
                    SHT_NOBITS
                } else {
                    SHT_PROGBITS
                };
                self.ensure_section(sec, ty, flags, 8);
            }
            ".globl" | ".global" => {
                for sym in split_syms(args) {
                    self.globl.insert(sym);
                }
            }
            ".weak" => {
                for sym in split_syms(args) {
                    self.weak.insert(sym);
                }
            }
            ".p2align" | ".align" => {
                let n: u32 = args
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| self.err("bad align"))?;
                self.align_cur(n)?;
            }
            ".type" | ".size" | ".file" | ".loc" | ".set" | ".equ" | ".local" | ".comm"
            | ".lcomm" | ".cfi_startproc" | ".cfi_endproc" | ".cfi_def_cfa" | ".cfi_offset" => {}
            ".asciz" | ".string" => {
                let s = parse_string_literal(args)?;
                let sec = self.cur_sec();
                sec.data.extend_from_slice(s.as_bytes());
                sec.data.push(0);
            }
            ".ascii" => {
                let s = parse_string_literal(args)?;
                self.cur_sec().data.extend_from_slice(s.as_bytes());
            }
            ".zero" | ".space" => {
                let n: usize = args
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| self.err("bad .zero"))?;
                let sec = self.cur_sec();
                let new_len = sec.data.len() + n;
                sec.data.resize(new_len, 0);
            }
            ".byte" => {
                for tok in args.split(',') {
                    let v: u8 = tok.trim().parse().map_err(|_| self.err("bad .byte"))?;
                    self.cur_sec().data.push(v);
                }
            }
            ".short" | ".word" | ".long" => {
                let width = if name == ".short" { 2 } else { 4 };
                for tok in args.split(',') {
                    let v = parse_int(tok.trim())? as u32;
                    let sec = self.cur_sec();
                    sec.data.extend_from_slice(&v.to_le_bytes()[..width]);
                }
            }
            ".quad" => {
                for tok in args.split(',') {
                    let v = parse_int(tok.trim())? as u64;
                    self.cur_sec().data.extend_from_slice(&v.to_le_bytes());
                }
            }
            other => {
                if other.starts_with(".cfi_") || other.starts_with(".section") {
                    return Ok(());
                }
                return Err(self.err(format!("unsupported directive '{other}'")));
            }
        }
        Ok(())
    }

    fn encode(&mut self, mnemonic: &str, operands: &str) -> Result<(), String> {
        let pc = self.cur_off();
        let mut out = Vec::new();
        let mut relocs: Vec<Reloc> = Vec::new();

        match mnemonic {
            "ret" => write_u32_le(&mut out, encode_ret()),
            "nop" => write_u32_le(&mut out, encode_nop()),
            "mov" => {
                let (a, b) = split2(operands)?;
                let rd = parse_reg(a)?;
                if b.trim().starts_with('#') {
                    let imm: u16 = parse_imm16(b.trim())?;
                    write_u32_le(&mut out, encode_movz(rd, imm, 0)?);
                } else {
                    write_u32_le(&mut out, encode_mov_reg(rd, parse_reg(b)?)?);
                }
            }
            "movz" => {
                let (rd_s, imm_s) = split2(operands)?;
                let rd = parse_reg(rd_s)?;
                let imm = parse_mov_imm(imm_s)?;
                write_u32_le(&mut out, encode_movz(rd, imm.0, imm.1)?);
            }
            "add" => {
                let parts = split_ops(operands);
                if parts.len() == 3 && parts[2].contains(":lo12:") {
                    let rd = parse_reg(parts[0])?;
                    let rn = parse_reg(parts[1])?;
                    let sym = parse_lo12_sym(parts[2])?;
                    write_u32_le(&mut out, encode_add_lo12_placeholder(rd, rn)?);
                    let (rsym, addend) = self.reloc_symbol_for(&sym);
                    relocs.push(Reloc {
                        offset: pc + out.len() as u64 - 4,
                        sym_name: rsym,
                        r_type: R_AARCH64_ADD_ABS_LO12_NC,
                        addend,
                    });
                } else if parts.len() == 3 {
                    let rd = parse_reg(parts[0])?;
                    let rn = parse_reg(parts[1])?;
                    let imm: u32 = parse_uimm12(parts[2])?;
                    write_u32_le(&mut out, encode_add_imm(rd, rn, imm)?);
                } else {
                    return Err(self.err(format!("unsupported add '{operands}'")));
                }
            }
            "sub" => {
                let (rd, rn, imm) = split3(operands)?;
                write_u32_le(
                    &mut out,
                    encode_sub_imm(parse_reg(rd)?, parse_reg(rn)?, parse_uimm12(imm)?)?,
                );
            }
            "and" => {
                let parts = split_ops(operands);
                if parts.len() != 3 {
                    return Err(self.err(format!("unsupported and '{operands}'")));
                }
                let rd = parse_reg(parts[0])?;
                let rn = parse_reg(parts[1])?;
                let imm = parse_hash_u64(parts[2])?;
                write_u32_le(&mut out, encode_and_imm(rd, rn, imm)?);
            }
            "rev" => {
                let (rd, rn) = split2(operands)?;
                write_u32_le(&mut out, encode_rev(parse_reg(rd)?, parse_reg(rn)?)?);
            }
            "rev16" => {
                let (rd, rn) = split2(operands)?;
                write_u32_le(&mut out, encode_rev16(parse_reg(rd)?, parse_reg(rn)?)?);
            }
            "stp" => {
                let parts = split_mem_ops(operands)?;
                let (rn, off, pre) = parse_mem_pair(&parts[2])?;
                if !pre {
                    return Err(self.err("post-index stp not supported in M2"));
                }
                write_u32_le(
                    &mut out,
                    encode_stp_pre(
                        parse_reg(&parts[0])?,
                        parse_reg(&parts[1])?,
                        parse_reg(&rn)?,
                        off,
                    )?,
                );
            }
            "ldp" => {
                let parts = split_mem_ops(operands)?;
                let (rn, off, pre) = parse_mem_pair(&parts[2])?;
                if pre {
                    return Err(self.err("pre-index ldp not supported in M2"));
                }
                write_u32_le(
                    &mut out,
                    encode_ldp_post(
                        parse_reg(&parts[0])?,
                        parse_reg(&parts[1])?,
                        parse_reg(&rn)?,
                        off,
                    )?,
                );
            }
            "str" => {
                let (rt, mem) = split2(operands)?;
                let (rn, off, pre) = parse_mem(mem)?;
                if pre {
                    write_u32_le(
                        &mut out,
                        encode_str_pre(parse_reg(rt)?, parse_reg(&rn)?, off)?,
                    );
                } else {
                    write_u32_le(
                        &mut out,
                        encode_str_reg(parse_reg(rt)?, parse_reg(&rn)?, off)?,
                    );
                }
            }
            "ldr" => {
                let (rt, mem) = split2(operands)?;
                let (rn, off, pre) = parse_mem(mem)?;
                if pre {
                    return Err(self.err("pre-index ldr not supported in M2"));
                }
                write_u32_le(
                    &mut out,
                    encode_ldr_reg(parse_reg(rt)?, parse_reg(&rn)?, off)?,
                );
            }
            "adrp" => {
                let (rd, sym) = split2(operands)?;
                write_u32_le(&mut out, encode_adrp_placeholder(parse_reg(rd)?)?);
                let (rsym, addend) = self.reloc_symbol_for(sym.trim());
                relocs.push(Reloc {
                    offset: pc,
                    sym_name: rsym,
                    r_type: R_AARCH64_ADR_PREL_HI21,
                    addend,
                });
            }
            "bl" => {
                let sym = operands.trim();
                write_u32_le(&mut out, encode_bl_placeholder());
                relocs.push(Reloc {
                    offset: pc,
                    sym_name: sym.to_string(),
                    r_type: R_AARCH64_CALL26,
                    addend: 0,
                });
                self.undefined.insert(sym.to_string());
            }
            "b" => {
                let target = operands.trim();
                if is_label_name(target) {
                    let imm26 = self.branch_imm26(target, pc)?;
                    write_u32_le(&mut out, encode_b_imm26(imm26)?);
                } else {
                    return Err(self.err(format!("unsupported branch target '{target}'")));
                }
            }
            other => return Err(self.err(format!("M2 encoding not implemented for '{other}'"))),
        }

        let sec_name = self.cur.clone();
        let section_names: Vec<String> = self.obj.sections.iter().map(|s| s.name.clone()).collect();
        let sec = self
            .obj
            .sections
            .iter_mut()
            .find(|s| s.name == sec_name)
            .unwrap();
        sec.data.extend_from_slice(&out);
        for r in relocs {
            if !self.labels.contains_key(&r.sym_name)
                && !section_names.iter().any(|n| n == &r.sym_name)
            {
                self.undefined.insert(r.sym_name.clone());
            }
            sec.relocs.push(r);
        }
        Ok(())
    }

    fn reloc_symbol_for(&self, sym: &str) -> (String, i64) {
        if let Some((sec, off)) = self.labels.get(sym) {
            if sec != ".text" {
                return (sec.clone(), *off as i64);
            }
        }
        (sym.to_string(), 0)
    }

    fn branch_imm26(&self, target: &str, pc: u64) -> Result<i32, String> {
        let (sec, off) = self
            .labels
            .get(target)
            .ok_or_else(|| self.err(format!("undefined label '{target}'")))?;
        if sec != &self.cur {
            return Err(self.err(format!("branch to label in other section '{target}'")));
        }
        let delta = *off as i64 - pc as i64;
        if delta % 4 != 0 {
            return Err(self.err("misaligned branch target"));
        }
        Ok((delta / 4) as i32)
    }

    fn finalize_symbols(&mut self) {
        for sec in &self.obj.sections {
            self.obj.symbols.push(Symbol {
                name: sec.name.clone(),
                section: Some(sec.name.clone()),
                value: 0,
                size: 0,
                binding: STB_LOCAL,
                sym_type: STT_SECTION,
            });
        }
        for (name, (sec, off)) in &self.labels {
            let binding = if self.weak.contains(name) {
                STB_WEAK
            } else if self.globl.contains(name) || !name.starts_with('.') {
                STB_GLOBAL
            } else {
                STB_LOCAL
            };
            let sym_type = if sec.contains("rodata") || sec.contains("data") || sec.contains("bss")
            {
                STT_OBJECT
            } else {
                STT_FUNC
            };
            self.obj.symbols.push(Symbol {
                name: name.clone(),
                section: Some(sec.clone()),
                value: *off,
                size: 0,
                binding,
                sym_type,
            });
        }
        for name in &self.undefined {
            if self.labels.contains_key(name) {
                continue;
            }
            self.obj.symbols.push(Symbol {
                name: name.clone(),
                section: None,
                value: 0,
                size: 0,
                binding: STB_GLOBAL,
                sym_type: STT_NOTYPE,
            });
        }
    }
}

fn split_syms(args: &str) -> Vec<String> {
    args.split_whitespace()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn split_ops(s: &str) -> Vec<&str> {
    s.split(',').map(|p| p.trim()).collect()
}

fn split2(s: &str) -> Result<(&str, &str), String> {
    let parts: Vec<&str> = s.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err(format!("expected two operands: '{s}'"));
    }
    Ok((parts[0].trim(), parts[1].trim()))
}

fn split3(s: &str) -> Result<(&str, &str, &str), String> {
    let parts = split_ops(s);
    if parts.len() != 3 {
        return Err(format!("expected three operands: '{s}'"));
    }
    Ok((parts[0], parts[1], parts[2]))
}

fn split_mem_ops(s: &str) -> Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    let mut commas = 0;
    for ch in s.chars() {
        if ch == '[' {
            depth += 1;
        }
        if ch == ']' {
            depth -= 1;
        }
        if ch == ',' && depth == 0 {
            commas += 1;
            if commas <= 2 {
                parts.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
        }
        cur.push(ch);
    }
    parts.push(cur.trim().to_string());
    if parts.len() != 3 {
        return Err(format!("expected reg, reg, mem: '{s}'"));
    }
    Ok(parts)
}

fn parse_mem_pair(mem: &str) -> Result<(String, i32, bool), String> {
    let mem = mem.trim();
    if let Some(rest) = mem.strip_prefix('[') {
        if let Some((base_part, tail)) = rest.split_once("], ") {
            let off = parse_i32(tail.trim().trim_start_matches('#'))?;
            return Ok((base_part.trim().to_string(), off, false));
        }
    }
    let pre = mem.ends_with('!');
    let inner = mem
        .trim_end_matches('!')
        .trim_start_matches('[')
        .trim_end_matches(']');
    let (base, off_s) = inner.rsplit_once(',').unwrap_or((inner, "#0"));
    let off = parse_i32(off_s.trim().trim_start_matches('#'))?;
    Ok((base.trim().to_string(), off, pre))
}

fn parse_mem(mem: &str) -> Result<(String, i32, bool), String> {
    let mem = mem.trim();
    let pre = mem.ends_with('!');
    let inner = mem
        .trim_end_matches('!')
        .trim_start_matches('[')
        .trim_end_matches(']');
    let (base, off_s) = inner.rsplit_once(',').unwrap_or((inner, "#0"));
    let off = parse_i32(off_s.trim().trim_start_matches('#'))?;
    Ok((base.trim().to_string(), off, pre))
}

fn parse_string_literal(s: &str) -> Result<String, String> {
    let s = s.trim();
    if !(s.starts_with('"') && s.ends_with('"')) {
        return Err(format!("expected string literal, got '{s}'"));
    }
    let inner = &s[1..s.len() - 1];
    Ok(unescape_c_string(inner))
}

fn unescape_c_string(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('0') => out.push('\0'),
                Some(x) => {
                    out.push('\\');
                    out.push(x);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_i32(s: &str) -> Result<i32, String> {
    let v = parse_int(s)?;
    i32::try_from(v).map_err(|_| format!("integer out of range: {v}"))
}

fn parse_int(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(rest, 16)
            .map(|v| v as i64)
            .map_err(|e| e.to_string())
    } else {
        s.parse::<i64>().map_err(|e| e.to_string())
    }
}

fn parse_uimm12(s: &str) -> Result<u32, String> {
    let v = parse_int(s.trim().trim_start_matches('#'))?;
    if v < 0 || v > 0xFFF {
        return Err(format!("immediate out of range: {v}"));
    }
    Ok(v as u32)
}

fn parse_hash_u64(s: &str) -> Result<u64, String> {
    let t = s.trim().trim_start_matches('#');
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        t.parse::<u64>().map_err(|e| e.to_string())
    }
}

fn parse_imm16(s: &str) -> Result<u16, String> {
    let v = parse_int(s.trim_start_matches('#'))?;
    if v < 0 || v > 0xFFFF {
        return Err(format!("immediate out of range: {v}"));
    }
    Ok(v as u16)
}

fn parse_mov_imm(s: &str) -> Result<(u16, u8), String> {
    let s = s.trim().trim_start_matches('#');
    if let Some(rest) = s.strip_prefix("lsl") {
        return Err(format!("movz lsl not supported: {rest}"));
    }
    Ok((parse_imm16(&format!("#{s}"))?, 0))
}

fn parse_lo12_sym(s: &str) -> Result<String, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix(":lo12:") {
        return Ok(rest.trim().to_string());
    }
    Err(format!("expected :lo12:sym, got '{s}'"))
}

fn is_label_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c == '.' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{Target, TargetOs};

    #[test]
    fn emits_valid_elf_header_for_ret_main() {
        let src = "\t.text\n\t.globl\tmain\nmain:\n\tmov\tw0, #0\n\tret\n";
        let unit = crate::assembler::parse_assembly(src, Target::Aarch64, TargetOs::Linux).unwrap();
        let obj = emit_elf_object(&unit).expect("emit");
        assert_eq!(&obj[0..4], b"\x7fELF");
        assert_eq!(obj[4], 2); // ELFCLASS64
        assert_eq!(obj[5], 1); // LE
    }
}
