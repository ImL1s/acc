//! ELF64 little-endian relocatable object writer (aarch64 Linux M2 subset).

use std::collections::HashMap;

pub const EM_AARCH64: u16 = 183;
pub const ET_REL: u16 = 1;
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;

pub const SHT_NULL: u32 = 0;
pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_NOBITS: u32 = 8;

pub const SHF_WRITE: u64 = 0x1;
pub const SHF_ALLOC: u64 = 0x2;
pub const SHF_EXECINSTR: u64 = 0x4;

pub const SHN_UNDEF: u16 = 0;

pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;

pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;

pub const R_AARCH64_NONE: u32 = 0;
pub const R_AARCH64_CALL26: u32 = 283;
pub const R_AARCH64_JUMP26: u32 = 282;
pub const R_AARCH64_ADR_PREL_LO21: u32 = 274;
pub const R_AARCH64_ADR_PREL_HI21: u32 = 275;
pub const R_AARCH64_ADD_ABS_LO12_NC: u32 = 277;

#[derive(Clone, Debug)]
pub struct Reloc {
    pub offset: u64,
    pub sym_name: String,
    pub r_type: u32,
    pub addend: i64,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub data: Vec<u8>,
    pub align: u64,
    pub relocs: Vec<Reloc>,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub section: Option<String>,
    pub value: u64,
    pub size: u64,
    pub binding: u8,
    pub sym_type: u8,
}

pub struct ElfObject {
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
}

impl ElfObject {
    pub fn section_mut(&mut self, name: &str, sh_type: u32, sh_flags: u64, align: u64) {
        if let Some(i) = self.sections.iter().position(|s| s.name == name) {
            self.sections[i].align = self.sections[i].align.max(align);
            return;
        }
        self.sections.push(Section {
            name: name.to_string(),
            sh_type,
            sh_flags,
            data: Vec::new(),
            align: align.max(1),
            relocs: Vec::new(),
        });
    }

    pub fn write(&self) -> Vec<u8> {
        let mut shstr = StringTable::new();
        shstr.add("");
        for sec in &self.sections {
            shstr.add(&sec.name);
        }
        let rela_names: Vec<String> = self
            .sections
            .iter()
            .filter(|s| !s.relocs.is_empty())
            .map(|s| format!(".rela.{}", s.name))
            .collect();
        for name in &rela_names {
            shstr.add(name);
        }
        shstr.add(".symtab");
        shstr.add(".strtab");
        shstr.add(".shstrtab");

        let mut strtab = StringTable::new();
        strtab.add("");

        let mut sym_entries: Vec<(u32, Symbol)> = Vec::new();
        sym_entries.push((0, Symbol {
            name: String::new(),
            section: None,
            value: 0,
            size: 0,
            binding: STB_LOCAL,
            sym_type: STT_NOTYPE,
        }));

        let mut locals: Vec<Symbol> = Vec::new();
        let mut globals: Vec<Symbol> = Vec::new();
        for sym in &self.symbols {
            if sym.section.is_none() {
                globals.push(sym.clone());
            } else if sym.binding == STB_LOCAL {
                locals.push(sym.clone());
            } else {
                globals.push(sym.clone());
            }
        }
        locals.sort_by(|a, b| a.name.cmp(&b.name));
        globals.sort_by(|a, b| a.name.cmp(&b.name));

        let mut sym_idx: HashMap<String, u32> = HashMap::new();
        for sym in locals.into_iter().chain(globals) {
            let name_idx = strtab.add(&sym.name);
            let idx = sym_entries.len() as u32;
            if !sym.name.is_empty() {
                sym_idx.insert(sym.name.clone(), idx);
            }
            sym_entries.push((name_idx, sym));
        }

        let mut file = Vec::new();
        file.resize(64, 0); // ELF header placeholder

        let mut shdrs: Vec<Shdr> = Vec::new();
        shdrs.push(Shdr::null());
        let mut sec_shndx: HashMap<String, usize> = HashMap::new();

        for sec in &self.sections {
            align_bytes(&mut file, sec.align, sec.sh_flags & SHF_EXECINSTR != 0);
            let offset = file.len() as u64;
            if sec.sh_type != SHT_NOBITS {
                file.extend_from_slice(&sec.data);
            }
            let idx = shdrs.len();
            sec_shndx.insert(sec.name.clone(), idx);
            shdrs.push(Shdr {
                sh_name: shstr.offset(&sec.name),
                sh_type: sec.sh_type,
                sh_flags: sec.sh_flags,
                sh_addr: 0,
                sh_offset: offset,
                sh_size: sec.data.len() as u64,
            sh_link: 0,
            sh_info: 0,
            sh_align: sec.align,
            sh_entsize: 0,
            });
        }

        let symtab_idx = shdrs.len();
        let symtab_off = align_to(&mut file, 8);
        let mut sym_bytes = Vec::new();
        let local_count = sym_entries
            .iter()
            .skip(1)
            .take_while(|(_, s)| s.binding == STB_LOCAL)
            .count()
            + 1;
        for (name_idx, sym) in &sym_entries {
            let shndx = match &sym.section {
                None => SHN_UNDEF,
                Some(sec) => sec_shndx.get(sec).copied().unwrap_or(0) as u16,
            };
            write_sym(&mut sym_bytes, *name_idx, sym, shndx);
        }
        file.extend_from_slice(&sym_bytes);
        shdrs.push(Shdr {
            sh_name: shstr.offset(".symtab"),
            sh_type: SHT_SYMTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: symtab_off,
            sh_size: sym_bytes.len() as u64,
            sh_link: 0,
            sh_info: local_count as u32,
            sh_align: 8,
            sh_entsize: 24,
        });

        let strtab_idx = shdrs.len();
        let strtab_off = align_to(&mut file, 1);
        let strtab_bytes = strtab.bytes.clone();
        file.extend_from_slice(&strtab_bytes);
        shdrs.push(Shdr {
            sh_name: shstr.offset(".strtab"),
            sh_type: SHT_STRTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: strtab_off,
            sh_size: strtab_bytes.len() as u64,
            sh_link: 0,
            sh_info: 0,
            sh_align: 1,
            sh_entsize: 0,
        });
        shdrs[symtab_idx].sh_link = strtab_idx as u32;

        for rela_name in &rela_names {
            let sec_name = rela_name.strip_prefix(".rela.").unwrap();
            let sec = self.sections.iter().find(|s| s.name == sec_name).unwrap();
            let rela_off = align_to(&mut file, 8);
            let mut rela_bytes = Vec::new();
            for reloc in &sec.relocs {
                let sym_i = *sym_idx
                    .get(&reloc.sym_name)
                    .unwrap_or_else(|| panic!("reloc symbol {} missing", reloc.sym_name));
                write_rela(&mut rela_bytes, reloc.offset, sym_i, reloc.r_type, reloc.addend);
            }
            file.extend_from_slice(&rela_bytes);
            shdrs.push(Shdr {
                sh_name: shstr.offset(rela_name),
                sh_type: SHT_RELA,
                sh_flags: 0,
                sh_addr: 0,
                sh_offset: rela_off,
                sh_size: rela_bytes.len() as u64,
                sh_link: symtab_idx as u32,
                sh_info: sec_shndx.get(sec_name).copied().unwrap_or(0) as u32,
                sh_align: 8,
                sh_entsize: 24,
            });
        }

        let shstrtab_idx = shdrs.len();
        let shstrtab_off = align_to(&mut file, 1);
        let shstrtab_bytes = shstr.bytes.clone();
        file.extend_from_slice(&shstrtab_bytes);
        shdrs.push(Shdr {
            sh_name: shstr.offset(".shstrtab"),
            sh_type: SHT_STRTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: shstrtab_off,
            sh_size: shstrtab_bytes.len() as u64,
            sh_link: 0,
            sh_info: 0,
            sh_align: 1,
            sh_entsize: 0,
        });

        let e_shoff = align_to(&mut file, 8);
        for sh in &shdrs {
            write_shdr(&mut file, sh);
        }

        let mut ehdr = Vec::new();
        write_ehdr(
            &mut ehdr,
            e_shoff,
            shdrs.len() as u16,
            shstrtab_idx as u16,
        );
        file[..64].copy_from_slice(&ehdr);
        file
    }
}

struct StringTable {
    bytes: Vec<u8>,
    offsets: HashMap<String, u32>,
}

impl StringTable {
    fn new() -> Self {
        let mut s = Self {
            bytes: vec![0],
            offsets: HashMap::new(),
        };
        s.offsets.insert(String::new(), 0);
        s
    }

    fn add(&mut self, s: &str) -> u32 {
        if let Some(&off) = self.offsets.get(s) {
            return off;
        }
        let off = self.bytes.len() as u32;
        self.offsets.insert(s.to_string(), off);
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        off
    }

    fn offset(&self, s: &str) -> u32 {
        *self.offsets.get(s).unwrap_or_else(|| panic!("shstr missing {s}"))
    }
}

struct Shdr {
    sh_name: u32,
    sh_type: u32,
    sh_flags: u64,
    sh_addr: u64,
    sh_offset: u64,
    sh_size: u64,
    sh_link: u32,
    sh_info: u32,
    sh_align: u64,
    sh_entsize: u64,
}

impl Shdr {
    fn null() -> Self {
        Self {
            sh_name: 0,
            sh_type: 0,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: 0,
            sh_size: 0,
            sh_link: 0,
            sh_info: 0,
            sh_align: 0,
            sh_entsize: 0,
        }
    }
}

fn align_to(buf: &mut Vec<u8>, align: u64) -> u64 {
    let mask = align.saturating_sub(1);
    while (buf.len() as u64) & mask != 0 {
        buf.push(0);
    }
    buf.len() as u64
}

fn align_bytes(buf: &mut Vec<u8>, align: u64, is_text: bool) {
    let mask = align.saturating_sub(1);
    while (buf.len() as u64) & mask != 0 {
        if is_text {
            buf.extend_from_slice(&encode_nop_le());
        } else {
            buf.push(0);
        }
    }
}

fn encode_nop_le() -> [u8; 4] {
    0xD503_201Fu32.to_le_bytes()
}

fn write_ehdr(out: &mut Vec<u8>, shoff: u64, shnum: u16, shstrndx: u16) {
    out.extend_from_slice(&[
        0x7F, b'E', b'L', b'F', ELFCLASS64, ELFDATA2LSB, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    write_u16(out, ET_REL);
    write_u16(out, EM_AARCH64);
    write_u32(out, 1);
    write_u64(out, 0);
    write_u64(out, 0);
    write_u64(out, shoff);
    write_u32(out, 0);
    write_u16(out, 64);
    write_u16(out, 0);
    write_u16(out, 0);
    write_u16(out, 64);
    write_u16(out, shnum);
    write_u16(out, shstrndx);
}

fn write_shdr(out: &mut Vec<u8>, sh: &Shdr) {
    write_u32(out, sh.sh_name);
    write_u32(out, sh.sh_type);
    write_u64(out, sh.sh_flags);
    write_u64(out, sh.sh_addr);
    write_u64(out, sh.sh_offset);
    write_u64(out, sh.sh_size);
    write_u32(out, sh.sh_link);
    write_u32(out, sh.sh_info);
    write_u64(out, sh.sh_align);
    write_u64(out, sh.sh_entsize);
}

fn write_sym(out: &mut Vec<u8>, name: u32, sym: &Symbol, shndx: u16) {
    write_u32(out, name);
    let info = (sym.binding << 4) | (sym.sym_type & 0xF);
    out.push(info);
    out.push(0);
    write_u16(out, shndx);
    write_u64(out, sym.value);
    write_u64(out, sym.size);
}

fn write_rela(out: &mut Vec<u8>, offset: u64, sym: u32, r_type: u32, addend: i64) {
    write_u64(out, offset);
    write_u64(out, ((sym as u64) << 32) | (r_type as u64));
    write_u64(out, addend as u64);
}

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
