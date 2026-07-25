//! Minimal ELF64 little-endian `ET_REL` reader for the builtin linker.

use std::collections::HashMap;

pub const ET_REL: u16 = 1;
pub const EM_AARCH64: u16 = 183;
#[allow(dead_code)]
pub const SHT_NULL: u32 = 0;
pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
#[allow(dead_code)]
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_NOBITS: u32 = 8;

pub const SHF_WRITE: u64 = 0x1;
pub const SHF_ALLOC: u64 = 0x2;
pub const SHF_EXECINSTR: u64 = 0x4;

pub const SHN_UNDEF: u16 = 0;
#[allow(dead_code)]
pub const SHN_ABS: u16 = 0xfff1;
#[allow(dead_code)]
pub const SHN_COMMON: u16 = 0xfff2;

#[allow(dead_code)]
pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;

#[allow(dead_code)]
pub const STT_NOTYPE: u8 = 0;
#[allow(dead_code)]
pub const STT_OBJECT: u8 = 1;
#[allow(dead_code)]
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;

#[derive(Clone, Debug)]
pub struct ParsedReloc {
    pub offset: u64,
    pub sym_index: u32,
    pub r_type: u32,
    pub addend: i64,
}

#[derive(Clone, Debug)]
pub struct ParsedSection {
    pub name: String,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub data: Vec<u8>,
    pub align: u64,
    pub relocs: Vec<ParsedReloc>,
    /// Original section header index in the object (for symbol shndx mapping).
    pub shndx: u16,
}

#[derive(Clone, Debug)]
pub struct ParsedSymbol {
    pub name: String,
    pub value: u64,
    pub size: u64,
    pub binding: u8,
    pub sym_type: u8,
    pub shndx: u16,
}

#[derive(Clone, Debug)]
pub struct ObjectFile {
    pub sections: Vec<ParsedSection>,
    pub symbols: Vec<ParsedSymbol>,
}

pub fn parse_elf_rel(bytes: &[u8]) -> Result<ObjectFile, String> {
    if bytes.len() < 64 {
        return Err("ELF too short".into());
    }
    if &bytes[0..4] != b"\x7fELF" {
        return Err("not ELF".into());
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err("need ELF64 LE".into());
    }
    let e_type = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
    let e_machine = u16::from_le_bytes(bytes[18..20].try_into().unwrap());
    if e_type != ET_REL {
        return Err(format!("expected ET_REL, got {e_type}"));
    }
    if e_machine != EM_AARCH64 {
        return Err(format!("expected EM_AARCH64, got {e_machine}"));
    }
    let e_shoff = u64::from_le_bytes(bytes[40..48].try_into().unwrap()) as usize;
    let e_shentsize = u16::from_le_bytes(bytes[58..60].try_into().unwrap()) as usize;
    let e_shnum = u16::from_le_bytes(bytes[60..62].try_into().unwrap()) as usize;
    let e_shstrndx = u16::from_le_bytes(bytes[62..64].try_into().unwrap()) as usize;
    if e_shentsize < 64 || e_shnum == 0 {
        return Err("invalid section header table".into());
    }
    let sh_end = e_shoff
        .checked_add(e_shentsize.checked_mul(e_shnum).ok_or("shnum overflow")?)
        .ok_or("shoff overflow")?;
    if sh_end > bytes.len() {
        return Err("section headers past EOF".into());
    }

    let read_shdr = |idx: usize| -> Result<Shdr, String> {
        let off = e_shoff + idx * e_shentsize;
        Ok(Shdr {
            name_off: u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
            sh_type: u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()),
            sh_flags: u64::from_le_bytes(bytes[off + 8..off + 16].try_into().unwrap()),
            sh_addr: u64::from_le_bytes(bytes[off + 16..off + 24].try_into().unwrap()),
            sh_offset: u64::from_le_bytes(bytes[off + 24..off + 32].try_into().unwrap()) as usize,
            sh_size: u64::from_le_bytes(bytes[off + 32..off + 40].try_into().unwrap()) as usize,
            sh_link: u32::from_le_bytes(bytes[off + 40..off + 44].try_into().unwrap()),
            sh_info: u32::from_le_bytes(bytes[off + 44..off + 48].try_into().unwrap()),
            sh_addralign: u64::from_le_bytes(bytes[off + 48..off + 56].try_into().unwrap()),
            sh_entsize: u64::from_le_bytes(bytes[off + 56..off + 64].try_into().unwrap()) as usize,
        })
    };

    let shstr = read_shdr(e_shstrndx)?;
    let shstrtab = slice(bytes, shstr.sh_offset, shstr.sh_size)?;

    let mut raw_sections: Vec<(u16, Shdr, String)> = Vec::new();
    for i in 0..e_shnum {
        let sh = read_shdr(i)?;
        let name = cstr_at(shstrtab, sh.name_off as usize)?;
        raw_sections.push((i as u16, sh, name));
    }

    // First pass: alloc/progbits/nobits content sections (skip meta).
    let mut sections: Vec<ParsedSection> = Vec::new();
    let mut shndx_to_sec: HashMap<u16, usize> = HashMap::new();
    for (idx, sh, name) in &raw_sections {
        if *idx == 0 {
            continue;
        }
        if sh.sh_type != SHT_PROGBITS && sh.sh_type != SHT_NOBITS {
            continue;
        }
        if name.starts_with(".rela") || name == ".symtab" || name == ".strtab" || name == ".shstrtab"
        {
            continue;
        }
        let data = if sh.sh_type == SHT_NOBITS {
            vec![0u8; sh.sh_size]
        } else {
            slice(bytes, sh.sh_offset, sh.sh_size)?.to_vec()
        };
        let sec_i = sections.len();
        shndx_to_sec.insert(*idx, sec_i);
        sections.push(ParsedSection {
            name: name.clone(),
            sh_type: sh.sh_type,
            sh_flags: sh.sh_flags,
            data,
            align: sh.sh_addralign.max(1),
            relocs: Vec::new(),
            shndx: *idx,
        });
    }

    // Symbols.
    let symtab_hdr = raw_sections
        .iter()
        .find(|(_, sh, name)| sh.sh_type == SHT_SYMTAB || name == ".symtab")
        .ok_or("missing .symtab")?;
    let strtab_idx = symtab_hdr.1.sh_link as usize;
    let strtab_hdr = raw_sections
        .get(strtab_idx)
        .ok_or("symtab sh_link out of range")?;
    let strtab = slice(bytes, strtab_hdr.1.sh_offset, strtab_hdr.1.sh_size)?;
    let entsz = if symtab_hdr.1.sh_entsize == 0 {
        24
    } else {
        symtab_hdr.1.sh_entsize
    };
    if entsz < 24 {
        return Err("bad symtab entsize".into());
    }
    let sym_bytes = slice(bytes, symtab_hdr.1.sh_offset, symtab_hdr.1.sh_size)?;
    let mut symbols = Vec::new();
    let mut off = 0;
    while off + entsz <= sym_bytes.len() {
        let name_off = u32::from_le_bytes(sym_bytes[off..off + 4].try_into().unwrap()) as usize;
        let info = sym_bytes[off + 4];
        let _other = sym_bytes[off + 5];
        let shndx = u16::from_le_bytes(sym_bytes[off + 6..off + 8].try_into().unwrap());
        let value = u64::from_le_bytes(sym_bytes[off + 8..off + 16].try_into().unwrap());
        let size = u64::from_le_bytes(sym_bytes[off + 16..off + 24].try_into().unwrap());
        let name = cstr_at(strtab, name_off)?;
        symbols.push(ParsedSymbol {
            name,
            value,
            size,
            binding: info >> 4,
            sym_type: info & 0xf,
            shndx,
        });
        off += entsz;
    }

    // Relocations (.rela.*).
    for (_, sh, name) in &raw_sections {
        if sh.sh_type != SHT_RELA && !name.starts_with(".rela.") {
            continue;
        }
        let target_name = name.strip_prefix(".rela.").unwrap_or("");
        let target_idx = sections
            .iter()
            .position(|s| s.name == target_name)
            .or_else(|| {
                let ti = sh.sh_info as u16;
                shndx_to_sec.get(&ti).copied()
            });
        let Some(ti) = target_idx else {
            continue;
        };
        let sec = &mut sections[ti];
        let entsz = if sh.sh_entsize == 0 { 24 } else { sh.sh_entsize };
        if entsz < 24 {
            return Err("bad rela entsize".into());
        }
        let rela = slice(bytes, sh.sh_offset, sh.sh_size)?;
        let mut roff = 0;
        while roff + entsz <= rela.len() {
            let r_offset = u64::from_le_bytes(rela[roff..roff + 8].try_into().unwrap());
            let r_info = u64::from_le_bytes(rela[roff + 8..roff + 16].try_into().unwrap());
            let r_addend = i64::from_le_bytes(rela[roff + 16..roff + 24].try_into().unwrap());
            sec.relocs.push(ParsedReloc {
                offset: r_offset,
                sym_index: (r_info >> 32) as u32,
                r_type: (r_info & 0xffff_ffff) as u32,
                addend: r_addend,
            });
            roff += entsz;
        }
    }

    let _ = shndx_to_sec;
    Ok(ObjectFile { sections, symbols })
}

struct Shdr {
    name_off: u32,
    sh_type: u32,
    sh_flags: u64,
    #[allow(dead_code)]
    sh_addr: u64,
    sh_offset: usize,
    sh_size: usize,
    sh_link: u32,
    sh_info: u32,
    sh_addralign: u64,
    sh_entsize: usize,
}

fn slice(bytes: &[u8], off: usize, size: usize) -> Result<&[u8], String> {
    let end = off.checked_add(size).ok_or("slice overflow")?;
    bytes.get(off..end).ok_or_else(|| "slice past EOF".into())
}

fn cstr_at(tab: &[u8], off: usize) -> Result<String, String> {
    if off >= tab.len() {
        return Err("strtab offset past EOF".into());
    }
    let end = tab[off..]
        .iter()
        .position(|&b| b == 0)
        .map(|i| off + i)
        .unwrap_or(tab.len());
    Ok(String::from_utf8_lossy(&tab[off..end]).into_owned())
}
