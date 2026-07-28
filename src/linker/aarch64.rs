//! AArch64 Linux freestanding static linker (M4 subset).

use super::elf_read::{
    parse_elf_rel, ObjectFile, EM_AARCH64, SHF_ALLOC, SHF_EXECINSTR, SHF_WRITE, SHN_UNDEF,
    SHT_NOBITS, SHT_PROGBITS, STB_GLOBAL, STB_WEAK,
};
use std::collections::HashMap;

const R_AARCH64_NONE: u32 = 0;
const R_AARCH64_CALL26: u32 = 283;
const R_AARCH64_JUMP26: u32 = 282;
const R_AARCH64_ADR_PREL_HI21: u32 = 275;
const R_AARCH64_ADD_ABS_LO12_NC: u32 = 277;
const R_AARCH64_ABS64: u32 = 257;
const R_AARCH64_ABS32: u32 = 258;

const LOAD_BASE: u64 = 0x400000;
const PAGE: u64 = 0x1000;

/// Link aarch64 Linux ET_REL object bytes into a static freestanding ET_EXEC.
pub fn link_aarch64_linux(objects: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if objects.is_empty() {
        return Err("linker: no input objects".into());
    }
    let mut parsed: Vec<ObjectFile> = Vec::with_capacity(objects.len());
    for (i, bytes) in objects.iter().enumerate() {
        parsed.push(parse_elf_rel(bytes).map_err(|e| format!("object[{i}]: {e}"))?);
    }

    // Merge alloc sections by name (simple M4: one input typically).
    let mut merged: HashMap<String, MergedSec> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    // Per-object section base offsets within the merged section.
    let mut obj_sec_base: Vec<HashMap<u16, u64>> = Vec::new();

    for obj in &parsed {
        let mut bases = HashMap::new();
        for sec in &obj.sections {
            if sec.sh_flags & SHF_ALLOC == 0 && sec.sh_type != SHT_NOBITS {
                // Still keep non-alloc progbits if named .rodata/.data/.text — assembler marks ALLOC.
                // Skip pure debug.
                if !sec.name.starts_with('.') {
                    continue;
                }
                if sec.name.starts_with(".debug") || sec.name.starts_with(".comment") {
                    continue;
                }
            }
            let entry = merged.entry(sec.name.clone()).or_insert_with(|| {
                order.push(sec.name.clone());
                MergedSec {
                    name: sec.name.clone(),
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
        obj_sec_base.push(bases);
    }

    // Global symbol table: name -> absolute (section, offset within merged section).
    #[derive(Clone)]
    struct Def {
        sec: String,
        value: u64,
        binding: u8,
    }
    let mut defs: HashMap<String, Def> = HashMap::new();
    let mut undefs: Vec<String> = Vec::new();

    for (oi, obj) in parsed.iter().enumerate() {
        for (si, sym) in obj.symbols.iter().enumerate() {
            if si == 0 {
                continue;
            }
            if sym.shndx == SHN_UNDEF {
                if !sym.name.is_empty() {
                    undefs.push(sym.name.clone());
                }
                continue;
            }
            if sym.name.is_empty() {
                continue;
            }
            let Some(base) = obj_sec_base[oi].get(&sym.shndx) else {
                // Absolute / non-merged — skip for M4.
                continue;
            };
            let sec_name = obj
                .sections
                .iter()
                .find(|s| s.shndx == sym.shndx)
                .map(|s| s.name.clone())
                .ok_or_else(|| format!("symbol {} section missing", sym.name))?;
            let def = Def {
                sec: sec_name,
                value: base + sym.value,
                binding: sym.binding,
            };
            match defs.get(&sym.name) {
                None => {
                    defs.insert(sym.name.clone(), def);
                }
                Some(prev) if prev.binding == STB_WEAK && def.binding != STB_WEAK => {
                    defs.insert(sym.name.clone(), def);
                }
                Some(_) if def.binding == STB_WEAK => {}
                Some(_) => {
                    return Err(format!("multiple definition of `{}`", sym.name));
                }
            }
        }
    }

    if !defs.contains_key("main") {
        return Err("linker: no `main` definition".into());
    }

    // Inject `_start` into .text if absent.
    if !defs.contains_key("_start") {
        let text = merged.entry(".text".into()).or_insert_with(|| {
            order.insert(0, ".text".into());
            MergedSec {
                name: ".text".into(),
                sh_type: SHT_PROGBITS,
                sh_flags: SHF_ALLOC | SHF_EXECINSTR,
                align: 4,
                data: Vec::new(),
                relocs: Vec::new(),
            }
        });
        align_vec(&mut text.data, 4);
        let start_off = text.data.len() as u64;
        // bl main  (reloc patched below) ; mov x8,#93 ; mov x0,x0 ; svc #0
        // We emit: bl #0 placeholder, movz x8, #93, svc #0
        // Actually keep w0 from main as exit code: mov x8,#93; svc #0
        text.data.extend_from_slice(&0x94000000u32.to_le_bytes()); // bl imm26=0
        text.data.extend_from_slice(&0xD2800BA8u32.to_le_bytes()); // movz x8, #93
        text.data.extend_from_slice(&0xD4000001u32.to_le_bytes()); // svc #0
                                                                   // Synthetic reloc for bl → main (CALL26), addend 0, at start_off.
        text.relocs.push(PendingReloc {
            offset: start_off,
            obj_index: usize::MAX, // synthetic
            sym_index: 0,
            r_type: R_AARCH64_CALL26,
            addend: 0,
        });
        // Mark synthetic target as "main" via special obj_index.
        defs.insert(
            "_start".into(),
            Def {
                sec: ".text".into(),
                value: start_off,
                binding: STB_GLOBAL,
            },
        );
        // Store synthetic name on the reloc via a side channel: use sym_index=u32::MAX
        text.relocs.last_mut().unwrap().sym_index = u32::MAX;
    }

    // Resolve remaining undefs against defs.
    let mut unresolved = Vec::new();
    for name in &undefs {
        if name == "_GLOBAL_OFFSET_TABLE_" {
            continue;
        }
        if !defs.contains_key(name) {
            unresolved.push(name.clone());
        }
    }
    unresolved.sort();
    unresolved.dedup();
    if !unresolved.is_empty() {
        return Err(format!(
            "linker: unresolved symbols: {}",
            unresolved.join(", ")
        ));
    }

    // Layout virtual addresses (RW after RX). Prefer: .text, then other RX, then RO, then RW.
    order.sort_by(|a, b| section_rank(a, &merged).cmp(&section_rank(b, &merged)));
    let mut addr = LOAD_BASE;
    let mut sec_addr: HashMap<String, u64> = HashMap::new();
    let file_off_base = 0x1000u64; // after headers in PT_LOAD

    for name in &order {
        let sec = merged.get_mut(name).unwrap();
        if sec.data.is_empty() && sec.sh_type != SHT_NOBITS {
            continue;
        }
        let align = sec.align.max(if sec.sh_flags & SHF_EXECINSTR != 0 {
            16
        } else {
            8
        });
        addr = align_u64(addr, align);
        sec_addr.insert(name.clone(), addr);
        addr += sec.data.len() as u64;
        addr = align_u64(addr, 16);
    }

    // Rebuild image bytes at file offsets matching VA - LOAD_BASE + file_off_base.
    let mut max_end = file_off_base;
    for name in &order {
        let Some(&va) = sec_addr.get(name) else {
            continue;
        };
        let sec = &merged[name];
        let fo = (va - LOAD_BASE) + file_off_base;
        max_end = max_end.max(fo + sec.data.len() as u64);
    }
    let mut file = vec![0u8; max_end as usize];

    for name in &order {
        let Some(&va) = sec_addr.get(name) else {
            continue;
        };
        let sec = &merged[name];
        let fo = ((va - LOAD_BASE) + file_off_base) as usize;
        file[fo..fo + sec.data.len()].copy_from_slice(&sec.data);
    }

    // Apply relocations into `file`.
    for name in &order {
        let Some(&sec_va) = sec_addr.get(name) else {
            continue;
        };
        let sec = &merged[name];
        for r in &sec.relocs {
            let sym_name = if r.sym_index == u32::MAX {
                "main".to_string()
            } else if r.obj_index == usize::MAX {
                "main".to_string()
            } else {
                let obj = &parsed[r.obj_index];
                obj.symbols
                    .get(r.sym_index as usize)
                    .map(|s| s.name.clone())
                    .ok_or_else(|| format!("bad reloc sym index {}", r.sym_index))?
            };
            let (s_va, _) = if let Some(d) = defs.get(&sym_name) {
                let s_base = *sec_addr
                    .get(&d.sec)
                    .ok_or_else(|| format!("def section {} missing", d.sec))?;
                (s_base + d.value, d)
            } else if sym_name.is_empty() {
                // Section-relative? treat addend against this section — unsupported.
                return Err("linker: empty reloc symbol".into());
            } else {
                return Err(format!("linker: unresolved reloc symbol `{sym_name}`"));
            };
            let p = sec_va + r.offset;
            let fo = ((p - LOAD_BASE) + file_off_base) as usize;
            apply_reloc(&mut file, fo, r.r_type, s_va, p, r.addend)?;
        }
    }

    let entry = {
        let d = defs.get("_start").unwrap();
        sec_addr.get(&d.sec).unwrap() + d.value
    };

    emit_et_exec(&file, file_off_base, max_end, entry, LOAD_BASE)
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

fn section_rank(name: &str, map: &HashMap<String, MergedSec>) -> (u8, String) {
    let flags = map.get(name).map(|s| s.sh_flags).unwrap_or(0);
    let rank = if flags & SHF_EXECINSTR != 0 {
        0
    } else if flags & SHF_WRITE != 0 {
        2
    } else {
        1
    };
    (rank, name.to_string())
}

fn apply_reloc(
    file: &mut [u8],
    fo: usize,
    r_type: u32,
    s: u64,
    p: u64,
    a: i64,
) -> Result<(), String> {
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
        R_AARCH64_ADD_ABS_LO12_NC => {
            let imm12 = ((s.wrapping_add(a as u64)) & 0xfff) as u32;
            let mut insn = u32::from_le_bytes(file[fo..fo + 4].try_into().unwrap());
            insn = (insn & !(0xFFF << 10)) | (imm12 << 10);
            file[fo..fo + 4].copy_from_slice(&insn.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ABS64 => {
            let val = (s as i64).wrapping_add(a) as u64;
            file[fo..fo + 8].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        R_AARCH64_ABS32 => {
            let val = (s as i64).wrapping_add(a) as u32;
            file[fo..fo + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        other => Err(format!("linker: unsupported reloc type {other}")),
    }
}

fn emit_et_exec(
    payload: &[u8],
    load_file_off: u64,
    payload_end: u64,
    entry: u64,
    load_va: u64,
) -> Result<Vec<u8>, String> {
    // Layout:
    // [0, 64) ehdr
    // [64, 64+56) phdr
    // pad to load_file_off
    // payload[load_file_off ..]
    let phoff = 64u64;
    let ehsize = 64u16;
    let phentsize = 56u16;
    let phnum = 1u16;

    let mut out = vec![0u8; payload_end as usize];
    // Copy payload region (includes zeros before load_file_off from earlier builder —
    // our `payload`/`file` already sized to payload_end with content at load offsets).
    out.copy_from_slice(payload);

    // ELF header
    out[0..4].copy_from_slice(b"\x7fELF");
    out[4] = 2; // ELFCLASS64
    out[5] = 1; // LSB
    out[6] = 1; // EV_CURRENT
                // e_type ET_EXEC
    out[16..18].copy_from_slice(&2u16.to_le_bytes());
    out[18..20].copy_from_slice(&EM_AARCH64.to_le_bytes());
    out[20..24].copy_from_slice(&1u32.to_le_bytes());
    out[24..32].copy_from_slice(&entry.to_le_bytes());
    out[32..40].copy_from_slice(&phoff.to_le_bytes()); // e_phoff
    out[40..48].copy_from_slice(&0u64.to_le_bytes()); // e_shoff
    out[48..52].copy_from_slice(&0u32.to_le_bytes()); // e_flags
    out[52..54].copy_from_slice(&ehsize.to_le_bytes());
    out[54..56].copy_from_slice(&phentsize.to_le_bytes());
    out[56..58].copy_from_slice(&phnum.to_le_bytes());
    out[58..60].copy_from_slice(&0u16.to_le_bytes()); // e_shentsize
    out[60..62].copy_from_slice(&0u16.to_le_bytes()); // e_shnum
    out[62..64].copy_from_slice(&0u16.to_le_bytes()); // e_shstrndx

    // PT_LOAD: cover from file 0 to payload_end, VA = load_va - load_file_off
    // so that file offset load_file_off maps to load_va.
    let p_vaddr = load_va - load_file_off;
    let filesz = payload_end;
    let memsz = payload_end;
    let mut ph = Vec::new();
    write_u32(&mut ph, 1); // PT_LOAD
    write_u32(&mut ph, 5); // PF_R|PF_X (and we may have RW data — M4 uses R|W|X for simplicity)
                           // Actually include write for .data/.bss:
    ph.clear();
    write_u32(&mut ph, 1);
    write_u32(&mut ph, 7); // R|W|X freestanding single segment
    write_u64(&mut ph, 0); // p_offset
    write_u64(&mut ph, p_vaddr);
    write_u64(&mut ph, p_vaddr); // p_paddr
    write_u64(&mut ph, filesz);
    write_u64(&mut ph, memsz);
    write_u64(&mut ph, PAGE); // align
    out[phoff as usize..phoff as usize + 56].copy_from_slice(&ph);

    let _ = LOAD_BASE;
    Ok(out)
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
