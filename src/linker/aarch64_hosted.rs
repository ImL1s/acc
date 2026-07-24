//! M5: static musl hosted link for aarch64 Linux (hello `printf` bar).
//!
//! Link order mirrors `musl-gcc -static` without spawning system `ld`:
//! `Scrt1.o crti.o <user> crtn.o --start-group libgcc.a libc.a --end-group`

use super::archive::read_archive;
use super::elf_read::{
    parse_elf_rel, ObjectFile, ParsedReloc, ParsedSection, SHF_ALLOC, SHF_EXECINSTR, SHF_WRITE,
    SHN_COMMON, SHN_UNDEF, SHT_NOBITS, SHT_PROGBITS, STB_GLOBAL, STB_LOCAL, STB_WEAK,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const R_AARCH64_NONE: u32 = 0;
const R_AARCH64_ABS64: u32 = 257;
const R_AARCH64_ABS32: u32 = 258;
const R_AARCH64_PREL32: u32 = 261;
const R_AARCH64_PREL64: u32 = 262;
const R_AARCH64_ADR_PREL_HI21: u32 = 275;
const R_AARCH64_ADR_PREL_LO21: u32 = 274;
const R_AARCH64_CONDBR19: u32 = 280;
const R_AARCH64_TSTBR14: u32 = 279;
const R_AARCH64_ADD_ABS_LO12_NC: u32 = 277;
const R_AARCH64_LDST8_ABS_LO12_NC: u32 = 278;
const R_AARCH64_LDST32_ABS_LO12_NC: u32 = 285;
const R_AARCH64_LDST64_ABS_LO12_NC: u32 = 286;
const R_AARCH64_JUMP26: u32 = 282;
const R_AARCH64_CALL26: u32 = 283;
const R_AARCH64_MOVW_UABS_G0_NC: u32 = 263;
const R_AARCH64_MOVW_UABS_G1_NC: u32 = 264;
const R_AARCH64_MOVW_UABS_G2_NC: u32 = 265;
const R_AARCH64_MOVW_UABS_G3_NC: u32 = 266;
const R_AARCH64_GLOB_DAT: u32 = 1025;
const R_AARCH64_JUMP_SLOT: u32 = 1026;
const R_AARCH64_RELATIVE: u32 = 1027;
const R_AARCH64_TLS_TPREL64: u32 = 1030;
const R_AARCH64_TLSLE_ADD_TPREL_HI12: u32 = 1039;
const R_AARCH64_TLSLE_LDST8_TPREL_LO12: u32 = 1040;
const R_AARCH64_TLSLE_LDST32_TPREL_LO12: u32 = 1042;
const R_AARCH64_TLSLE_LDST64_TPREL_LO12: u32 = 1043;
const R_AARCH64_ADR_GOT_PAGE: u32 = 311;
const R_AARCH64_LD64_GOT_LO12_NC: u32 = 312;

const LOAD_BASE: u64 = 0x400000;
const PAGE: u64 = 0x10000;
/// File bytes [0, HDR_PAD) hold ELF headers; first RX section starts here.
const HDR_PAD: u64 = 0x120;

#[derive(Clone)]
struct Def {
    sec: String,
    value: u64,
    binding: u8,
}

pub fn musl_lib_dir() -> PathBuf {
    std::env::var("ACC_MUSL_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/lib/aarch64-linux-musl"))
}

pub fn libgcc_archive() -> PathBuf {
    std::env::var("ACC_LIBGCC_A")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from("/usr/lib/gcc/aarch64-linux-gnu/13/libgcc.a")
        })
}

/// True when any input object has an undefined global function/data symbol (hosted TU).
pub fn needs_hosted_link(objects: &[Vec<u8>]) -> Result<bool, String> {
    for (i, bytes) in objects.iter().enumerate() {
        let obj = parse_elf_rel(bytes).map_err(|e| format!("object[{i}]: {e}"))?;
        for (si, sym) in obj.symbols.iter().enumerate() {
            if si == 0 || sym.shndx != SHN_UNDEF || sym.name.is_empty() {
                continue;
            }
            if sym.name == "_GLOBAL_OFFSET_TABLE_" {
                continue;
            }
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn link_aarch64_linux_hosted(user_objects: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let musl = musl_lib_dir();
    let libgcc = libgcc_archive();
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for name in ["Scrt1.o", "crti.o"] {
        let p = musl.join(name);
        inputs.push(
            std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?,
        );
    }
    inputs.extend_from_slice(user_objects);
    let crtn = musl.join("crtn.o");
    inputs.push(
        std::fs::read(&crtn).map_err(|e| format!("read {}: {e}", crtn.display()))?,
    );

    // Archive-driven symbol resolution (--start-group style).
    let mut archives: Vec<(PathBuf, Vec<(String, Vec<u8>)>)> = Vec::new();
    for path in [libgcc.clone(), musl.join("libc.a")] {
        if !path.exists() {
            return Err(format!(
                "hosted link: missing {} (set ACC_MUSL_LIB_DIR / ACC_LIBGCC_A)",
                path.display()
            ));
        }
        let members = read_archive(&path)?;
        archives.push((path, members));
    }

    let mut included: Vec<Vec<u8>> = inputs;
    let mut undefs = collect_undefs(&included)?;
    let mut changed = true;
    let mut rounds = 0u32;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for (_path, members) in &archives {
            for (_name, bytes) in members {
                if !object_defines_any(bytes, &undefs)? {
                    continue;
                }
                if !included.iter().any(|o| o.as_slice() == bytes.as_slice()) {
                    included.push(bytes.clone());
                    changed = true;
                }
            }
        }
        let new_undefs = collect_undefs(&included)?;
        if new_undefs != undefs {
            undefs = new_undefs;
            changed = true;
        }
    }
    if !undefs.is_empty() {
        return Err(format!(
            "hosted link: unresolved symbols: {}",
            undefs.join(", ")
        ));
    }
    link_objects_hosted(&included)
}

fn object_defines_any(bytes: &[u8], undefs: &[String]) -> Result<bool, String> {
    let obj = parse_elf_rel(bytes)?;
    let set: HashSet<&str> = undefs.iter().map(|s| s.as_str()).collect();
    for (si, sym) in obj.symbols.iter().enumerate() {
        if si == 0 || sym.shndx == SHN_UNDEF || sym.name.is_empty() {
            continue;
        }
        if set.contains(sym.name.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

const HOSTED_LINKER_SYMS: &[&str] = &[
    "_DYNAMIC",
    "__init_array_start",
    "__init_array_end",
    "__fini_array_start",
    "__fini_array_end",
    "_GLOBAL_OFFSET_TABLE_",
    "__preinit_array_start",
    "__preinit_array_end",
    "_edata",
    "_end",
    "__bss_start",
    "__bss_start__",
    "__bss_end__",
    "__tls_size",
    "__tls_align",
    "__tdata_base",
    "__tls_base",
];

fn collect_undefs(objects: &[Vec<u8>]) -> Result<Vec<String>, String> {
    let linker_syms: HashSet<&str> = HOSTED_LINKER_SYMS.iter().copied().collect();
    let mut defs: HashSet<String> = HashSet::new();
    let mut undefs: Vec<String> = Vec::new();
    for bytes in objects {
        let obj = parse_elf_rel(bytes)?;
        for (si, sym) in obj.symbols.iter().enumerate() {
            if si == 0 || sym.name.is_empty() {
                continue;
            }
            if sym.shndx == SHN_UNDEF {
                if !linker_syms.contains(sym.name.as_str()) {
                    undefs.push(sym.name.clone());
                }
            } else {
                defs.insert(sym.name.clone());
            }
        }
    }
    undefs.retain(|u| !defs.contains(u) && !linker_syms.contains(u.as_str()));
    undefs.sort();
    undefs.dedup();
    Ok(undefs)
}

/// Map `.text.foo` → `.text` so musl Scrt1 `.text._start_c` lands with `.text`.
fn canonical_sec_name(name: &str) -> &str {
    if let Some(rest) = name.strip_prefix(".text.") {
        if !rest.is_empty() {
            return ".text";
        }
    }
    if let Some(rest) = name.strip_prefix(".rodata.") {
        if !rest.is_empty() {
            return ".rodata";
        }
    }
    name
}

fn link_objects_hosted(objects: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let mut parsed: Vec<ObjectFile> = Vec::with_capacity(objects.len());
    for (i, bytes) in objects.iter().enumerate() {
        parsed.push(parse_elf_rel(bytes).map_err(|e| format!("object[{i}]: {e}"))?);
    }

    let mut merged: HashMap<String, MergedSec> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut obj_sec_base: Vec<HashMap<u16, u64>> = Vec::new();
    let mut commons: HashMap<String, (u64, u64)> = HashMap::new(); // name -> (size, align)

    for obj in &parsed {
        let mut bases = HashMap::new();
        for sec in &obj.sections {
            if sec.name.starts_with(".debug") || sec.name.starts_with(".comment") {
                continue;
            }
            if sec.sh_flags & SHF_ALLOC == 0 && sec.sh_type != SHT_NOBITS {
                if !sec.name.starts_with('.') {
                    continue;
                }
            }
            let canon = canonical_sec_name(&sec.name).to_string();
            let entry = merged.entry(canon.clone()).or_insert_with(|| {
                order.push(canon.clone());
                MergedSec {
                    name: canon.clone(),
                    sh_type: sec.sh_type,
                    sh_flags: sec.sh_flags,
                    align: sec.align.max(1),
                    data: Vec::new(),
                    relocs: Vec::new(),
                }
            });
            entry.align = entry.align.max(sec.align.max(1));
            entry.sh_flags |= sec.sh_flags;
            align_vec(&mut entry.data, entry.align);
            let base = entry.data.len() as u64;
            bases.insert(sec.shndx, base);
            if sec.sh_type == SHT_NOBITS {
                entry.data.resize(entry.data.len() + sec.data.len(), 0);
            } else {
                entry.data.extend_from_slice(&sec.data);
            }
            for r in &sec.relocs {
                entry.relocs.push(PendingReloc {
                    offset: base + r.offset,
                    obj_index: obj_sec_base.len(),
                    sym_index: r.sym_index,
                    r_type: r.r_type,
                    addend: r.addend,
                });
            }
        }
        for (si, sym) in obj.symbols.iter().enumerate() {
            if si == 0 || sym.shndx != SHN_COMMON || sym.name.is_empty() {
                continue;
            }
            let align = sym.value.max(1);
            let e = commons.entry(sym.name.clone()).or_insert((sym.size, align));
            e.0 = e.0.max(sym.size);
            e.1 = e.1.max(align);
        }
        obj_sec_base.push(bases);
    }

    let mut defs: HashMap<String, Def> = HashMap::new();
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, sym) in obj.symbols.iter().enumerate() {
            if si == 0 || sym.name.is_empty() || sym.name.starts_with('$') {
                continue;
            }
            if sym.binding == STB_LOCAL {
                continue;
            }
            if sym.shndx == SHN_UNDEF {
                continue;
            }
            if sym.shndx == SHN_COMMON {
                let bss = merged.get(".bss").ok_or("missing .bss for COMMON")?;
                // COMMON symbols get assigned lazily — handled below
                let _ = bss;
                continue;
            }
            // CRT objects (Scrt1/crti/user/crtn) own _start; ignore archive duplicates.
            if sym.name == "_start" && oi >= 4 {
                continue;
            }
            let Some(base) = obj_sec_base[oi].get(&sym.shndx) else {
                continue;
            };
            let sec_name = obj
                .sections
                .iter()
                .find(|s| s.shndx == sym.shndx)
                .map(|s| canonical_sec_name(&s.name).to_string())
                .ok_or_else(|| format!("symbol {} section missing", sym.name))?;
            let def = Def {
                sec: sec_name,
                value: base + sym.value,
                binding: sym.binding,
            };
            merge_def(&mut defs, sym.name.clone(), def)?;
        }
    }

    // COMMON symbol addresses in .bss
    if !commons.is_empty() {
        merged.entry(".bss".into()).or_insert_with(|| {
            order.push(".bss".into());
            MergedSec {
                name: ".bss".into(),
                sh_type: SHT_NOBITS,
                sh_flags: SHF_ALLOC | SHF_WRITE,
                align: 8,
                data: Vec::new(),
                relocs: Vec::new(),
            }
        });
        let mut names: Vec<_> = commons.keys().cloned().collect();
        names.sort();
        let mut off = 0u64;
        for name in names {
            let (size, align) = commons[&name];
            off = align_u64(off, align);
            merge_def(
                &mut defs,
                name,
                Def {
                    sec: ".bss".into(),
                    value: off,
                    binding: STB_GLOBAL,
                },
            )?;
            off += size;
        }
        if let Some(bss) = merged.get_mut(".bss") {
            bss.data.resize(off as usize, 0);
        }
    }

    if !defs.contains_key("main") {
        return Err("hosted link: no `main`".into());
    }
    if !defs.contains_key("_start") {
        return Err("hosted link: no `_start` (missing Scrt1.o?)".into());
    }

    // Ensure linker-script sections exist (empty is fine).
    // `.dynamic` must be a real DT_NULL terminator — Scrt1 passes &_DYNAMIC into
    // musl; pointing it at the ELF header (LOAD_BASE) makes libc walk garbage.
    for (name, flags) in [
        (".preinit_array", SHF_ALLOC | SHF_WRITE),
        (".init_array", SHF_ALLOC | SHF_WRITE),
        (".fini_array", SHF_ALLOC | SHF_WRITE),
        (".data.rel.ro", SHF_ALLOC | SHF_WRITE),
        (".dynamic", SHF_ALLOC | SHF_WRITE),
    ] {
        if !merged.contains_key(name) {
            order.push(name.to_string());
            let data = if name == ".dynamic" {
                // One Elf64_Dyn { d_tag: DT_NULL, d_un: 0 }
                vec![0u8; 16]
            } else {
                Vec::new()
            };
            merged.insert(
                name.to_string(),
                MergedSec {
                    name: name.to_string(),
                    sh_type: SHT_PROGBITS,
                    sh_flags: flags,
                    align: 8,
                    data,
                    relocs: Vec::new(),
                },
            );
        } else if name == ".dynamic" {
            let d = merged.get_mut(name).unwrap();
            if d.data.len() < 16 {
                d.data.resize(16, 0);
            }
        }
    }

    // Pre-scan GOT requirements before layout so `.got` lands in the RW image.
    let mut got_off: HashMap<String, u64> = HashMap::new();
    let got_base = merged
        .get(".got")
        .map(|g| g.data.len() as u64)
        .unwrap_or(0);
    let mut got_extra = 0u64;
    // Honour existing `.got` reloc layout from libc objects first.
    if let Some(got) = merged.get(".got") {
        for r in &got.relocs {
            if r.r_type == R_AARCH64_GLOB_DAT || r.r_type == R_AARCH64_JUMP_SLOT {
                let sym_name = resolve_sym_name(&parsed, r)?;
                if sym_name.is_empty() {
                    continue;
                }
                got_off.entry(sym_name).or_insert(r.offset);
            }
        }
    }
    for sec in merged.values() {
        for r in &sec.relocs {
            if r.r_type == R_AARCH64_ADR_GOT_PAGE || r.r_type == R_AARCH64_LD64_GOT_LO12_NC {
                let sym_name = resolve_sym_name(&parsed, r)?;
                if sym_name.is_empty() {
                    continue;
                }
                got_off.entry(sym_name).or_insert_with(|| {
                    let o = got_base + got_extra;
                    got_extra += 8;
                    o
                });
            }
        }
    }
    got_extra = got_off
        .values()
        .map(|o| o.saturating_add(8))
        .max()
        .unwrap_or(0)
        .saturating_sub(got_base);
    if got_extra > 0 {
        let got_name = ".got".to_string();
        if let Some(g) = merged.get_mut(&got_name) {
            g.data.resize((got_base + got_extra) as usize, 0);
        } else {
            order.push(got_name.clone());
            merged.insert(
                got_name,
                MergedSec {
                    name: ".got".into(),
                    sh_type: SHT_PROGBITS,
                    sh_flags: SHF_ALLOC | SHF_WRITE,
                    align: 8,
                    data: vec![0u8; (got_base + got_extra) as usize],
                    relocs: Vec::new(),
                },
            );
        }
    }
    if std::env::var("ACC_DEBUG_M5").is_ok() {
        eprintln!(
            "m5: got_base={got_base} got_extra={got_extra} got_off={got_off:?} merged_got={}",
            merged.get(".got").map(|g| g.data.len()).unwrap_or(0)
        );
    }

    // Section layout: RX cluster then RW (match musl static ET_EXEC).
    order.sort_by(|a, b| section_rank(a, &merged).cmp(&section_rank(b, &merged)));
    let mut rx_names: Vec<String> = Vec::new();
    let mut rw_names: Vec<String> = Vec::new();
    for name in &order {
        let flags = merged[name].sh_flags;
        if is_rw_section(name, flags) {
            rw_names.push(name.clone());
        } else if flags & SHF_ALLOC != 0 || merged[name].data.len() > 0 {
            rx_names.push(name.clone());
        }
    }

    let mut sec_addr: HashMap<String, u64> = HashMap::new();
    let mut addr = LOAD_BASE + HDR_PAD;
    let mut rx_file_end = HDR_PAD;
    for name in &rx_names {
        let sec = merged.get_mut(name).unwrap();
        if sec.data.is_empty() && sec.sh_type != SHT_NOBITS {
            continue;
        }
        let align = section_align(sec);
        addr = align_u64(addr, align);
        sec_addr.insert(name.clone(), addr);
        let fo = addr - LOAD_BASE;
        rx_file_end = rx_file_end.max(fo + sec.data.len() as u64);
        addr += sec.data.len() as u64;
    }
    let rx_end_va = addr;
    let rx_filesz = rx_file_end;

    let rw_vma_start = align_u64(rx_end_va, PAGE);
    // Linux requires (p_vaddr - p_offset) % p_align == 0. With LOAD_BASE
    // page-aligned, that means p_offset == p_vaddr - LOAD_BASE (pad the file).
    let rw_file_start = rw_vma_start - LOAD_BASE;
    addr = rw_vma_start;
    let mut rw_file_end = rw_file_start;
    for name in &rw_names {
        let sec = merged.get_mut(name).unwrap();
        let align = section_align(sec);
        addr = align_u64(addr, align);
        sec_addr.insert(name.clone(), addr);
        let fo = rw_file_start + (addr - rw_vma_start);
        if sec.sh_type != SHT_NOBITS {
            rw_file_end = rw_file_end.max(fo + sec.data.len() as u64);
        }
        addr += sec.data.len() as u64;
    }
    let rw_mem_end = align_u64(addr, 16);

    // Linker-script symbols required by musl Scrt1 (before GOT fill).
    for (sym, sec, off) in [
        ("__preinit_array_start", ".preinit_array", 0u64),
        (
            "__preinit_array_end",
            ".preinit_array",
            merged[".preinit_array"].data.len() as u64,
        ),
        ("__init_array_start", ".init_array", 0u64),
        ("__init_array_end", ".init_array", merged[".init_array"].data.len() as u64),
        ("__fini_array_start", ".fini_array", 0u64),
        ("__fini_array_end", ".fini_array", merged[".fini_array"].data.len() as u64),
    ] {
        if sec_addr.contains_key(sec) {
            defs.insert(
                sym.into(),
                Def {
                    sec: sec.into(),
                    value: off,
                    binding: STB_GLOBAL,
                },
            );
        }
    }
    // Static musl Scrt1 passes &_DYNAMIC into __libc_start_main; use DT_NULL block.
    if sec_addr.contains_key(".dynamic") {
        defs.insert(
            "_DYNAMIC".into(),
            Def {
                sec: ".dynamic".into(),
                value: 0,
                binding: STB_GLOBAL,
            },
        );
    } else {
        return Err("hosted link: missing .dynamic for _DYNAMIC".into());
    }
    let tls_size = merged
        .get(".tdata")
        .map(|s| s.data.len() as u64)
        .unwrap_or(0)
        + merged
            .get(".tbss")
            .map(|s| s.data.len() as u64)
            .unwrap_or(0);
    if tls_size > 0 {
        defs.insert(
            "__tls_size".into(),
            Def {
                sec: String::new(),
                value: tls_size,
                binding: STB_GLOBAL,
            },
        );
        defs.insert(
            "__tls_align".into(),
            Def {
                sec: String::new(),
                value: 16,
                binding: STB_GLOBAL,
            },
        );
        if let Some(&td_va) = sec_addr.get(".tdata") {
            defs.insert(
                "__tdata_base".into(),
                Def {
                    sec: String::new(),
                    value: td_va,
                    binding: STB_GLOBAL,
                },
            );
            defs.insert(
                "__tls_base".into(),
                Def {
                    sec: String::new(),
                    value: td_va,
                    binding: STB_GLOBAL,
                },
            );
        }
    }
    if let Some(&bss_va) = sec_addr.get(".bss") {
        let bss_sz = merged.get(".bss").map(|b| b.data.len() as u64).unwrap_or(0);
        defs.insert(
            "__bss_start".into(),
            Def {
                sec: ".bss".into(),
                value: 0,
                binding: STB_GLOBAL,
            },
        );
        defs.insert(
            "__bss_start__".into(),
            Def {
                sec: ".bss".into(),
                value: 0,
                binding: STB_GLOBAL,
            },
        );
        defs.insert(
            "__bss_end__".into(),
            Def {
                sec: ".bss".into(),
                value: bss_sz,
                binding: STB_GLOBAL,
            },
        );
        defs.insert(
            "_end".into(),
            Def {
                sec: String::new(),
                value: bss_va + bss_sz,
                binding: STB_GLOBAL,
            },
        );
        let edata_va = rw_names
            .iter()
            .filter(|n| *n != ".bss" && *n != ".tbss")
            .filter_map(|n| {
                let sec = merged.get(n)?;
                let va = sec_addr.get(n)?;
                Some(va + sec.data.len() as u64)
            })
            .max()
            .unwrap_or(bss_va);
        defs.insert(
            "_edata".into(),
            Def {
                sec: String::new(),
                value: edata_va,
                binding: STB_GLOBAL,
            },
        );
    }
    if got_extra > 0 {
        defs.insert(
            "_GLOBAL_OFFSET_TABLE_".into(),
            Def {
                sec: ".got".into(),
                value: 0,
                binding: STB_GLOBAL,
            },
        );
    }

    if std::env::var("ACC_DEBUG_M5").is_ok() {
        for sym in ["__libc_start_main", "printf", "main", "_fini", "_init"] {
            if let Ok(va) = symbol_va(sym, &defs, &sec_addr) {
                eprintln!("m5 sym {sym} = {va:#x}");
            }
        }
    }

    let file_size = rw_file_end;
    let mut file = vec![0u8; file_size as usize];
    for name in &rx_names {
        let Some(&va) = sec_addr.get(name) else {
            continue;
        };
        let sec = &merged[name];
        let fo = (va - LOAD_BASE) as usize;
        if sec.sh_type != SHT_NOBITS {
            file[fo..fo + sec.data.len()].copy_from_slice(&sec.data);
        }
    }
    for name in &rw_names {
        let Some(&va) = sec_addr.get(name) else {
            continue;
        };
        let sec = &merged[name];
        if sec.sh_type == SHT_NOBITS {
            continue;
        }
        let fo = (rw_file_start + (va - rw_vma_start)) as usize;
        file[fo..fo + sec.data.len()].copy_from_slice(&sec.data);
    }

    if got_extra > 0 {
        for (sym, off) in &got_off {
            let val = symbol_va(sym, &defs, &sec_addr)?;
            let got_va = sec_addr.get(".got").copied().unwrap_or(rw_vma_start);
            let fo = (rw_file_start + (got_va - rw_vma_start) + off) as usize;
            if std::env::var("ACC_DEBUG_M5").is_ok() && matches!(sym.as_str(), "_fini" | "_init" | "main") {
                eprintln!("m5 got fill {sym} off={off} fo={fo:#x} val={val:#x} file_len={}", file.len());
            }
            if fo + 8 <= file.len() {
                file[fo..fo + 8].copy_from_slice(&val.to_le_bytes());
            }
        }
    }

    // Apply relocations.
    for name in rx_names.iter().chain(rw_names.iter()) {
        let Some(&sec_va) = sec_addr.get(name) else {
            continue;
        };
        let sec = &merged[name];
        for r in &sec.relocs {
            let sym_name = resolve_sym_name(&parsed, r)?;
            let s_va = if r.r_type == R_AARCH64_TLS_TPREL64
                || r.r_type == R_AARCH64_TLSLE_ADD_TPREL_HI12
                || r.r_type == R_AARCH64_TLSLE_LDST8_TPREL_LO12
                || r.r_type == R_AARCH64_TLSLE_LDST32_TPREL_LO12
                || r.r_type == R_AARCH64_TLSLE_LDST64_TPREL_LO12
            {
                symbol_tls_offset(&sym_name, &defs, &merged)?
            } else if sym_name.is_empty() {
                sec_va
            } else if sym_name.starts_with('.') {
                lookup_symbol_va(
                    &sym_name,
                    &parsed,
                    r.obj_index,
                    &obj_sec_base,
                    &sec_addr,
                )?
                .ok_or_else(|| format!("hosted link: local symbol `{sym_name}` missing"))?
            } else if let Some(va) = resolve_sym_va(
                &sym_name,
                &parsed,
                r,
                &defs,
                &obj_sec_base,
                &sec_addr,
            )? {
                va
            } else {
                symbol_va(&sym_name, &defs, &sec_addr)?
            };
            let p = sec_va + r.offset;
            let fo = if sec_va < rw_vma_start {
                (p - LOAD_BASE) as usize
            } else {
                (rw_file_start + (p - rw_vma_start)) as usize
            };
            apply_reloc_hosted(
                &mut file,
                fo,
                r.r_type,
                s_va,
                p,
                r.addend,
                &sym_name,
                &got_off,
                sec_addr.get(".got").copied().unwrap_or(rw_vma_start),
            )?;
        }
    }

    let entry = symbol_va("_start", &defs, &sec_addr)?;
    let tls_layout = if tls_size > 0 {
        let rw_off = |va: u64| -> u64 {
            if va >= rw_vma_start {
                rw_file_start + (va - rw_vma_start)
            } else {
                va - LOAD_BASE
            }
        };
        if let Some(&td_va) = sec_addr.get(".tdata") {
            let td = merged.get(".tdata").unwrap();
            let tb_sz = merged
                .get(".tbss")
                .map(|b| b.data.len() as u64)
                .unwrap_or(0);
            Some(TlsLayout {
                file_off: rw_off(td_va),
                vaddr: td_va,
                filesz: td.data.len() as u64,
                memsz: td.data.len() as u64 + tb_sz,
                align: section_align(td).max(16),
            })
        } else if let Some(&tb_va) = sec_addr.get(".tbss") {
            let tb = merged.get(".tbss").unwrap();
            Some(TlsLayout {
                file_off: rw_off(tb_va),
                vaddr: tb_va,
                filesz: 0,
                memsz: tb.data.len() as u64,
                align: section_align(tb).max(16),
            })
        } else {
            None
        }
    } else {
        None
    };
    emit_hosted_exec(
        &file,
        entry,
        LOAD_BASE,
        rx_filesz,
        rx_end_va - LOAD_BASE,
        rw_vma_start,
        rw_file_start,
        rw_mem_end - rw_vma_start,
        tls_layout,
    )
}

fn merge_def(defs: &mut HashMap<String, Def>, name: String, def: Def) -> Result<(), String> {
    match defs.get(&name) {
        None => {
            defs.insert(name, def);
        }
        Some(prev) if prev.binding == STB_WEAK && def.binding != STB_WEAK => {
            defs.insert(name, def);
        }
        Some(_) if def.binding == STB_WEAK => {}
        Some(_) => {
            return Err(format!("multiple definition of `{name}`"));
        }
    }
    Ok(())
}

fn resolve_sym_name(parsed: &[ObjectFile], r: &PendingReloc) -> Result<String, String> {
    let obj = &parsed[r.obj_index];
    obj.symbols
        .get(r.sym_index as usize)
        .map(|s| s.name.clone())
        .ok_or_else(|| format!("bad reloc sym index {}", r.sym_index))
}

/// Resolve a symbol for reloc application: globals from `defs`, locals from any object.
fn resolve_sym_va(
    name: &str,
    parsed: &[ObjectFile],
    r: &PendingReloc,
    defs: &HashMap<String, Def>,
    obj_sec_base: &[HashMap<u16, u64>],
    sec_addr: &HashMap<String, u64>,
) -> Result<Option<u64>, String> {
    if let Some(d) = defs.get(name) {
        if d.sec.is_empty() {
            return Ok(Some(d.value));
        }
        if let Some(base) = sec_addr.get(&d.sec) {
            return Ok(Some(base + d.value));
        }
    }
    if let Some(va) = lookup_symbol_va(name, parsed, r.obj_index, obj_sec_base, sec_addr)? {
        return Ok(Some(va));
    }
    for oi in 0..parsed.len() {
        if oi == r.obj_index {
            continue;
        }
        if let Some(va) = lookup_symbol_va(name, parsed, oi, obj_sec_base, sec_addr)? {
            return Ok(Some(va));
        }
    }
    // Weak undefined → 0 (musl optional hooks).
    let obj = &parsed[r.obj_index];
    if let Some(sym) = obj.symbols.get(r.sym_index as usize) {
        if sym.shndx == SHN_UNDEF && sym.binding == STB_WEAK {
            return Ok(Some(0));
        }
    }
    Ok(None)
}

fn lookup_symbol_va(
    name: &str,
    parsed: &[ObjectFile],
    obj_index: usize,
    obj_sec_base: &[HashMap<u16, u64>],
    sec_addr: &HashMap<String, u64>,
) -> Result<Option<u64>, String> {
    let obj = &parsed[obj_index];
    for (si, sym) in obj.symbols.iter().enumerate() {
        if si == 0 || sym.name != name || sym.shndx == SHN_UNDEF {
            continue;
        }
        if sym.shndx == SHN_COMMON {
            return Ok(None);
        }
        let Some(&base) = obj_sec_base[obj_index].get(&sym.shndx) else {
            continue;
        };
        let sec_name = obj
            .sections
            .iter()
            .find(|s| s.shndx == sym.shndx)
            .map(|s| canonical_sec_name(&s.name).to_string())
            .ok_or_else(|| format!("local symbol {name} section missing"))?;
        let sec_va = sec_addr
            .get(&sec_name)
            .ok_or_else(|| format!("section {sec_name} for local `{name}` not laid out"))?;
        return Ok(Some(sec_va + base + sym.value));
    }
    Ok(None)
}

fn symbol_tls_offset(
    name: &str,
    defs: &HashMap<String, Def>,
    merged: &HashMap<String, MergedSec>,
) -> Result<u64, String> {
    let d = defs
        .get(name)
        .ok_or_else(|| format!("hosted link: tls symbol `{name}` missing"))?;
    let mut base = 0u64;
    if d.sec == ".tbss" {
        base = merged
            .get(".tdata")
            .map(|t| t.data.len() as u64)
            .unwrap_or(0);
    } else if d.sec != ".tdata" {
        return Err(format!("hosted link: tls symbol `{name}` not in .tdata/.tbss"));
    }
    Ok(base + d.value)
}

fn symbol_va(
    name: &str,
    defs: &HashMap<String, Def>,
    sec_addr: &HashMap<String, u64>,
) -> Result<u64, String> {
    if let Some(d) = defs.get(name) {
        if d.sec.is_empty() {
            return Ok(d.value);
        }
        let base = sec_addr
            .get(&d.sec)
            .ok_or_else(|| format!("section {} missing for `{name}`", d.sec))?;
        return Ok(base + d.value);
    }
    // Section-relative reloc targets (sym name == section name).
    if let Some(&va) = sec_addr.get(name) {
        return Ok(va);
    }
    Err(format!("hosted link: symbol `{name}` missing"))
}

struct MergedSec {
    name: String,
    sh_type: u32,
    sh_flags: u64,
    align: u64,
    data: Vec<u8>,
    relocs: Vec<PendingReloc>,
}

struct PendingReloc {
    offset: u64,
    obj_index: usize,
    sym_index: u32,
    r_type: u32,
    addend: i64,
}

fn is_rw_section(name: &str, flags: u64) -> bool {
    if name.starts_with(".rodata") {
        return false;
    }
    if name == ".bss"
        || name == ".got"
        || name == ".got.plt"
        || name == ".tbss"
        || name == ".tdata"
        || name == ".data.rel.ro"
        || name == ".dynamic"
    {
        return true;
    }
    if flags & SHF_WRITE != 0 {
        return true;
    }
    name == ".data" || name.starts_with(".data.")
}

fn section_rank(name: &str, map: &HashMap<String, MergedSec>) -> (u8, String) {
    let flags = map.get(name).map(|s| s.sh_flags).unwrap_or(0);
    let rank = if name == ".init" {
        0
    } else if name == ".text" {
        1
    } else if name == ".fini" {
        3
    } else if flags & SHF_EXECINSTR != 0 {
        2
    } else if name == ".rodata" || name.starts_with(".rodata") {
        4
    } else if name == ".data.rel.ro" {
        5
    } else if name == ".dynamic" {
        6
    } else if is_rw_section(name, flags) {
        7
    } else {
        6
    };
    (rank, name.to_string())
}

fn section_align(sec: &MergedSec) -> u64 {
    if sec.sh_flags & SHF_EXECINSTR != 0 {
        sec.align.max(16)
    } else {
        sec.align.max(8)
    }
}

fn apply_reloc_hosted(
    file: &mut [u8],
    fo: usize,
    r_type: u32,
    s: u64,
    p: u64,
    a: i64,
    sym_name: &str,
    got_off: &HashMap<String, u64>,
    got_va: u64,
) -> Result<(), String> {
    if fo + 4 > file.len() && r_type != R_AARCH64_NONE {
        return Err(format!("reloc at file offset {fo} past image"));
    }
    match r_type {
        R_AARCH64_NONE => Ok(()),
        R_AARCH64_CALL26 | R_AARCH64_JUMP26 => {
            let delta = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            if delta & 0x3 != 0 {
                return Err("CALL26/JUMP26 not 4-byte aligned".into());
            }
            let imm = delta >> 2;
            if imm < -(1 << 25) || imm >= (1 << 25) {
                return Err(format!("CALL26/JUMP26 out of range ({delta})"));
            }
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & 0xFC00_0000) | ((imm as u32) & 0x03FF_FFFF);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_CONDBR19 => {
            let delta = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            if delta & 0x3 != 0 {
                return Err("CONDBR19 not 4-byte aligned".into());
            }
            let imm = delta >> 2;
            if imm < -(1 << 18) || imm >= (1 << 18) {
                return Err(format!("CONDBR19 out of range ({delta})"));
            }
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & 0xFF00_001F) | (((imm as u32) & 0x7FFFF) << 5);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_TSTBR14 => {
            let delta = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            if delta & 0x3 != 0 {
                return Err("TSTBR14 not 4-byte aligned".into());
            }
            let imm = delta >> 2;
            if imm < 0 || imm >= (1 << 14) {
                return Err(format!("TSTBR14 out of range ({delta})"));
            }
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & 0xFFF8_001F) | (((imm as u32) & 0x3FFF) << 5);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ADR_PREL_HI21 => {
            let page_s = (s.wrapping_add(a as u64)) & !0xfff;
            let page_p = p & !0xfff;
            let delta = page_s as i64 - page_p as i64;
            let imm = delta >> 12;
            if imm < -(1 << 20) || imm >= (1 << 20) {
                return Err(format!("ADR_PREL_HI21 out of range ({delta})"));
            }
            let imm_u = imm as u32;
            let immlo = imm_u & 0x3;
            let immhi = (imm_u >> 2) & 0x1f_ffff;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & 0x9000_001F) | (immlo << 29) | (immhi << 5);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ADR_PREL_LO21 => {
            let delta = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            let imm = delta;
            if imm < -(1 << 20) || imm >= (1 << 20) {
                return Err(format!("ADR_PREL_LO21 out of range ({delta})"));
            }
            let imm_u = imm as u32;
            let immlo = imm_u & 0x3;
            let immhi = (imm_u >> 2) & 0x7ffff;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & 0x9F00001F) | (immlo << 29) | (immhi << 5);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ADD_ABS_LO12_NC => {
            let imm12 = ((s.wrapping_add(a as u64)) & 0xfff) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_LDST8_ABS_LO12_NC => {
            let imm12 = ((s.wrapping_add(a as u64)) & 0xfff) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_LDST32_ABS_LO12_NC => {
            let imm12 = (((s.wrapping_add(a as u64)) & 0xfff) >> 2) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_LDST64_ABS_LO12_NC => {
            let imm12 = (((s.wrapping_add(a as u64)) & 0xfff) >> 3) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ABS64 => {
            if fo + 8 > file.len() {
                return Err("ABS64 past image".into());
            }
            let val = (s as i64).wrapping_add(a) as u64;
            file[fo..fo + 8].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ABS32 => {
            let val = (s as i64).wrapping_add(a) as u32;
            file[fo..fo + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_PREL32 => {
            let val = (s as i64)
                .wrapping_add(a)
                .wrapping_sub(p as i64) as u32;
            file[fo..fo + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_PREL64 => {
            if fo + 8 > file.len() {
                return Err("PREL64 past image".into());
            }
            let val = (s as i64)
                .wrapping_add(a)
                .wrapping_sub(p as i64) as u64;
            file[fo..fo + 8].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_GLOB_DAT | R_AARCH64_JUMP_SLOT => {
            if fo + 8 > file.len() {
                return Err("GLOB_DAT/JUMP_SLOT past image".into());
            }
            let val = (s as i64).wrapping_add(a) as u64;
            file[fo..fo + 8].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_RELATIVE => {
            if fo + 8 > file.len() {
                return Err("RELATIVE past image".into());
            }
            let val = (p as i64).wrapping_add(a) as u64;
            file[fo..fo + 8].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_MOVW_UABS_G0_NC => apply_movw_uabs(file, fo, s, a, 0),
        R_AARCH64_MOVW_UABS_G1_NC => apply_movw_uabs(file, fo, s, a, 16),
        R_AARCH64_MOVW_UABS_G2_NC => apply_movw_uabs(file, fo, s, a, 32),
        R_AARCH64_MOVW_UABS_G3_NC => apply_movw_uabs(file, fo, s, a, 48),
        R_AARCH64_TLS_TPREL64 => {
            if fo + 8 > file.len() {
                return Err("TLS_TPREL64 past image".into());
            }
            let val = (s as i64).wrapping_add(a) as u64;
            file[fo..fo + 8].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_TLSLE_ADD_TPREL_HI12 => {
            let imm12 = ((s.wrapping_add(a as u64)) & 0xfff) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_TLSLE_LDST8_TPREL_LO12 => {
            let imm12 = (s.wrapping_add(a as u64) & 0xfff) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_TLSLE_LDST32_TPREL_LO12 => {
            let imm12 = ((s.wrapping_add(a as u64) & 0xfff) >> 2) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_TLSLE_LDST64_TPREL_LO12 => {
            let imm12 = ((s.wrapping_add(a as u64) & 0xfff) >> 3) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ADR_GOT_PAGE => {
            let got_entry = got_off
                .get(sym_name)
                .ok_or_else(|| format!("GOT missing for `{sym_name}`"))?;
            let g_addr = got_va + *got_entry;
            let page_s = g_addr & !0xfff;
            let page_p = p & !0xfff;
            let delta = page_s as i64 - page_p as i64;
            let imm = delta >> 12;
            let imm_u = imm as u32;
            let immlo = imm_u & 0x3;
            let immhi = (imm_u >> 2) & 0x1f_ffff;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & 0x9000_001F) | (immlo << 29) | (immhi << 5);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_LD64_GOT_LO12_NC => {
            let got_entry = got_off
                .get(sym_name)
                .ok_or_else(|| format!("GOT missing for `{sym_name}`"))?;
            let g_addr = got_va + *got_entry;
            let imm12 = ((g_addr & 0xfff) >> 3) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        other => Err(format!("hosted link: unsupported reloc type {other}")),
    }
}

fn apply_movw_uabs(
    file: &mut [u8],
    fo: usize,
    s: u64,
    a: i64,
    shift: u32,
) -> Result<(), String> {
    let val = s.wrapping_add(a as u64);
    let imm16 = ((val >> shift) & 0xffff) as u32;
    let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
    insn = (insn & 0xFFE0_001F) | (imm16 << 5);
    file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
    Ok(())
}

struct TlsLayout {
    file_off: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

fn emit_hosted_exec(
    payload: &[u8],
    entry: u64,
    load_va: u64,
    rx_filesz: u64,
    rx_memsz: u64,
    rw_vaddr: u64,
    rw_file_off: u64,
    rw_memsz: u64,
    tls: Option<TlsLayout>,
) -> Result<Vec<u8>, String> {
    let phoff = 64u64;
    let ehsize = 64u16;
    let phentsize = 56u16;
    let phnum: u16 = if tls.is_some() { 4 } else { 3 };
    let mut out = payload.to_vec();

    // Ensure headers fit before first load byte.
    let hdr_end = phoff + (phnum as u64) * (phentsize as u64);
    if rx_filesz < hdr_end {
        out.resize(hdr_end as usize, 0);
    }

    out[0..4].copy_from_slice(b"\x7fELF");
    out[4] = 2;
    out[5] = 1;
    out[6] = 1;
    out[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    out[18..20].copy_from_slice(&183u16.to_le_bytes()); // EM_AARCH64
    out[20..24].copy_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
    out[24..32].copy_from_slice(&entry.to_le_bytes());
    out[32..40].copy_from_slice(&phoff.to_le_bytes());
    out[52..54].copy_from_slice(&ehsize.to_le_bytes());
    out[54..56].copy_from_slice(&phentsize.to_le_bytes());
    out[56..58].copy_from_slice(&phnum.to_le_bytes());

    write_phdr(
        &mut out,
        phoff as usize,
        1,
        5, // R+X
        0,
        load_va,
        rx_filesz.max(hdr_end),
        rx_memsz,
        PAGE,
    );
    let rw_filesz = out.len() as u64 - rw_file_off;
    write_phdr(
        &mut out,
        phoff as usize + 56,
        1,
        6, // RW
        rw_file_off,
        rw_vaddr,
        rw_filesz,
        rw_memsz,
        PAGE,
    );
    let mut ph_idx = 2usize;
    if let Some(t) = &tls {
        write_phdr(
            &mut out,
            phoff as usize + ph_idx * 56,
            7, // PT_TLS
            4, // PF_R
            t.file_off,
            t.vaddr,
            t.filesz,
            t.memsz,
            t.align,
        );
        ph_idx += 1;
    }
    // PT_GNU_STACK
    write_phdr(
        &mut out,
        phoff as usize + ph_idx * 56,
        0x6474e551, // PT_GNU_STACK
        6,          // RW
        0,
        0,
        0,
        0,
        16,
    );
    Ok(out)
}

fn write_phdr(
    out: &mut [u8],
    off: usize,
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
) {
    let mut ph = Vec::with_capacity(56);
    write_u32(&mut ph, p_type);
    write_u32(&mut ph, p_flags);
    write_u64(&mut ph, p_offset);
    write_u64(&mut ph, p_vaddr);
    write_u64(&mut ph, p_vaddr);
    write_u64(&mut ph, p_filesz);
    write_u64(&mut ph, p_memsz);
    write_u64(&mut ph, p_align);
    out[off..off + 56].copy_from_slice(&ph);
}

fn align_vec(v: &mut Vec<u8>, align: u64) {
    let a = align.max(1);
    while (v.len() as u64) % a != 0 {
        v.push(0);
    }
}

fn align_u64(v: u64, align: u64) -> u64 {
    let a = align.max(1);
    (v + a - 1) & !(a - 1)
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
