//! aarch64 instruction encoding for the acc builtin-assembler M2 subset.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reg {
    W(u8),
    X(u8),
    Sp,
    Wzr,
    Xzr,
}

impl Reg {
    pub fn x_num(self) -> Result<u8, String> {
        match self {
            Reg::X(n) => Ok(n),
            Reg::Sp => Ok(31),
            Reg::Xzr => Ok(31),
            Reg::W(n) => Ok(n),
            Reg::Wzr => Ok(31),
        }
    }

    pub fn is_32(self) -> bool {
        matches!(self, Reg::W(_) | Reg::Wzr)
    }
}

pub fn parse_reg(s: &str) -> Result<Reg, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("sp") {
        return Ok(Reg::Sp);
    }
    if s.eq_ignore_ascii_case("xzr") {
        return Ok(Reg::Xzr);
    }
    if s.eq_ignore_ascii_case("wzr") {
        return Ok(Reg::Wzr);
    }
    if let Some(rest) = s.strip_prefix('x').or_else(|| s.strip_prefix('X')) {
        let n: u8 = rest
            .parse()
            .map_err(|_| format!("bad x register '{s}'"))?;
        if n > 30 {
            return Err(format!("register out of range '{s}'"));
        }
        return Ok(Reg::X(n));
    }
    if let Some(rest) = s.strip_prefix('w').or_else(|| s.strip_prefix('W')) {
        let n: u8 = rest
            .parse()
            .map_err(|_| format!("bad w register '{s}'"))?;
        if n > 30 {
            return Err(format!("register out of range '{s}'"));
        }
        return Ok(Reg::W(n));
    }
    Err(format!("unknown register '{s}'"))
}

pub fn encode_ret() -> u32 {
    0xD65F_03C0
}

pub fn encode_nop() -> u32 {
    0xD503_201F
}

pub fn encode_mov_reg(rd: Reg, rm: Reg) -> Result<u32, String> {
    if matches!(rd, Reg::Sp) || matches!(rm, Reg::Sp) {
        return encode_add_imm(rd, rm, 0);
    }
    let sf = if rd.is_32() || rm.is_32() {
        if rd.is_32() != rm.is_32() {
            return Err("mov register width mismatch".into());
        }
        0u32
    } else {
        1
    };
    let rd_n = rd.x_num()?;
    let rm_n = rm.x_num()?;
    Ok((sf << 31) | 0x2A0003E0 | ((rm_n as u32) << 16) | rd_n as u32)
}

pub fn encode_movz(rd: Reg, imm: u16, hw: u8) -> Result<u32, String> {
    let sf = if rd.is_32() { 0 } else { 1 };
    let rd_n = rd.x_num()?;
    if hw > 3 {
        return Err("movz hw out of range".into());
    }
    Ok((sf << 31) | 0x5280_0000 | ((hw as u32) << 21) | ((imm as u32) << 5) | rd_n as u32)
}

pub fn encode_add_imm(rd: Reg, rn: Reg, imm: u32) -> Result<u32, String> {
    let sf = if rd.is_32() || rn.is_32() {
        if rd.is_32() != rn.is_32() {
            return Err("add width mismatch".into());
        }
        0u32
    } else {
        1
    };
    if imm > 0xFFF {
        return Err(format!("add immediate too large: {imm}"));
    }
    let rd_n = rd.x_num()?;
    let rn_n = rn.x_num()?;
    Ok((sf << 31) | 0x1100_0000 | (imm << 10) | ((rn_n as u32) << 5) | rd_n as u32)
}

pub fn encode_sub_imm(rd: Reg, rn: Reg, imm: u32) -> Result<u32, String> {
    let sf = if rd.is_32() || rn.is_32() {
        if rd.is_32() != rn.is_32() {
            return Err("sub width mismatch".into());
        }
        0u32
    } else {
        1
    };
    if imm > 0xFFF {
        return Err(format!("sub immediate too large: {imm}"));
    }
    let rd_n = rd.x_num()?;
    let rn_n = rn.x_num()?;
    Ok((sf << 31) | 0x5100_0000 | (imm << 10) | ((rn_n as u32) << 5) | rd_n as u32)
}

pub fn encode_stp_pre(rt: Reg, rt2: Reg, rn: Reg, imm: i32) -> Result<u32, String> {
    encode_pair(true, true, rt, rt2, rn, imm)
}

pub fn encode_ldp_post(rt: Reg, rt2: Reg, rn: Reg, imm: i32) -> Result<u32, String> {
    encode_pair(false, false, rt, rt2, rn, imm)
}

fn encode_pair(store: bool, pre: bool, rt: Reg, rt2: Reg, rn: Reg, imm: i32) -> Result<u32, String> {
    if rt.is_32() || rt2.is_32() || rn.is_32() {
        return Err("32-bit stp/ldp not implemented in M2".into());
    }
    if imm % 8 != 0 {
        return Err(format!("pair offset must be multiple of 8: {imm}"));
    }
    let imm7 = (imm / 8) as i32;
    if imm7 < -64 || imm7 > 63 {
        return Err(format!("pair offset out of range: {imm}"));
    }
    let imm7_u = (imm7 as u32) & 0x7F;
    let rt_n = rt.x_num()?;
    let rt2_n = rt2.x_num()?;
    let rn_n = rn.x_num()?;
    let regs = ((rt2_n as u32) << 10) | ((rn_n as u32) << 5) | rt_n as u32;
    let base = match (store, pre) {
        (true, true) => 0xA980_0000,   // stp Rt, Rt2, [Rn, #imm]!
        (false, false) => 0xA8C0_0000, // ldp Rt, Rt2, [Rn], #imm
        _ => {
            return Err(format!(
                "stp/ldp addressing mode not supported in M2 (store={store}, pre={pre})"
            ));
        }
    };
    Ok(base | (imm7_u << 15) | regs)
}

pub fn encode_str_reg(rt: Reg, rn: Reg, offset: i32) -> Result<u32, String> {
    encode_ldst(true, rt, rn, offset)
}

pub fn encode_ldr_reg(rt: Reg, rn: Reg, offset: i32) -> Result<u32, String> {
    encode_ldst(false, rt, rn, offset)
}

fn encode_ldst(store: bool, rt: Reg, rn: Reg, offset: i32) -> Result<u32, String> {
    if rt.is_32() || rn.is_32() {
        return Err("32-bit ldr/str not implemented in M2".into());
    }
    if offset < 0 || (offset > 0 && offset % 8 != 0) {
        return if store {
            encode_stur(rt, rn, offset)
        } else {
            encode_ldur(rt, rn, offset)
        };
    }
    encode_ldst_unsigned(store, rt, rn, offset)
}

pub fn encode_str_pre(rt: Reg, rn: Reg, offset: i32) -> Result<u32, String> {
    if rt.is_32() || rn.is_32() {
        return Err("32-bit str pre-index not implemented in M2".into());
    }
    if offset < -256 || offset > 255 {
        return Err(format!("str pre-index offset out of range: {offset}"));
    }
    let rt_n = rt.x_num()?;
    let rn_n = rn.x_num()?;
    let imm9 = (offset as u32) & 0x1FF;
    Ok(0xF800_0000 | (imm9 << 12) | (0b11 << 10) | ((rn_n as u32) << 5) | rt_n as u32)
}

pub fn encode_stur(rt: Reg, rn: Reg, offset: i32) -> Result<u32, String> {
    if rt.is_32() || rn.is_32() {
        return Err("32-bit stur not implemented in M2".into());
    }
    if offset < -256 || offset > 255 {
        return Err(format!("stur offset out of range: {offset}"));
    }
    let rt_n = rt.x_num()?;
    let rn_n = rn.x_num()?;
    let imm9 = (offset as u32) & 0x1FF;
    Ok(0xF800_0000 | (imm9 << 12) | ((rn_n as u32) << 5) | rt_n as u32)
}

pub fn encode_ldur(rt: Reg, rn: Reg, offset: i32) -> Result<u32, String> {
    if rt.is_32() || rn.is_32() {
        return Err("32-bit ldur not implemented in M2".into());
    }
    if offset < -256 || offset > 255 {
        return Err(format!("ldur offset out of range: {offset}"));
    }
    let rt_n = rt.x_num()?;
    let rn_n = rn.x_num()?;
    let imm9 = (offset as u32) & 0x1FF;
    Ok(0xF840_0000 | (imm9 << 12) | ((rn_n as u32) << 5) | rt_n as u32)
}

fn encode_ldst_unsigned(store: bool, rt: Reg, rn: Reg, offset: i32) -> Result<u32, String> {
    let rt_n = rt.x_num()?;
    let rn_n = rn.x_num()?;
    if offset >= 0 && offset % 8 == 0 && (offset / 8) <= 0xFFF {
        let imm12 = (offset / 8) as u32;
        let opc = if store { 0b00 } else { 0b01 };
        return Ok(0xF900_0000 | (opc << 22) | (imm12 << 10) | ((rn_n as u32) << 5) | rt_n as u32);
    }
    Err(format!("ldr/str offset out of range: {offset}"))
}

pub fn encode_b_imm26(imm26: i32) -> Result<u32, String> {
    if imm26 < -(1 << 25) || imm26 >= (1 << 25) {
        return Err(format!("branch out of range: {imm26}"));
    }
    let enc = (imm26 as u32) & 0x03FF_FFFF;
    Ok(0x1400_0000 | enc)
}

pub fn encode_bl_placeholder() -> u32 {
    0x9400_0000
}

pub fn encode_adrp_placeholder(rd: Reg) -> Result<u32, String> {
    let rd_n = rd.x_num()?;
    Ok(0x9000_0000 | rd_n as u32)
}

pub fn encode_add_lo12_placeholder(rd: Reg, rn: Reg) -> Result<u32, String> {
    let rd_n = rd.x_num()?;
    let rn_n = rn.x_num()?;
    Ok(0x9100_0000 | ((rn_n as u32) << 5) | rd_n as u32)
}

/// REV / REV16 (32- or 64-bit) for weak bswap stubs in hosted TUs.
pub fn encode_rev(rd: Reg, rn: Reg) -> Result<u32, String> {
    let rd_n = rd.x_num()?;
    let rn_n = rn.x_num()?;
    let base = if rd.is_32() {
        0x5AC0_0800
    } else {
        0x5AC0_0C00
    };
    Ok(base | ((rn_n as u32) << 5) | rd_n as u32)
}

pub fn encode_rev16(rd: Reg, rn: Reg) -> Result<u32, String> {
    if !rd.is_32() || !rn.is_32() {
        return Err("rev16 requires 32-bit registers".into());
    }
    let rd_n = rd.x_num()?;
    let rn_n = rn.x_num()?;
    Ok(0x5AC0_0400 | ((rn_n as u32) << 5) | rd_n as u32)
}

/// AND (register, bitmask immediate).
pub fn encode_and_imm(rd: Reg, rn: Reg, imm: u64) -> Result<u32, String> {
    let (n, immr, imms) = encode_bitmask_imm(imm, rd.is_32())?;
    let sf = if rd.is_32() { 0u32 } else { 1u32 << 31 };
    let rd_n = rd.x_num()?;
    let rn_n = rn.x_num()?;
    Ok(0x1200_0000 | sf | (n << 12) | (immr << 16) | (imms << 10) | ((rn_n as u32) << 5) | rd_n as u32)
}

fn encode_bitmask_imm(imm: u64, is_32: bool) -> Result<(u32, u32, u32), String> {
    let size: u32 = if is_32 { 32 } else { 64 };
    let mask = if is_32 { imm as u32 as u64 } else { imm };
    for n in [0u32, 1u32] {
        let max_s: u32 = if n == 0 { 64 } else { 32 };
        for s in (2..=max_s) {
            if size < s {
                continue;
            }
            let replicate = ((1u64 << s) - 1) & mask;
            let mut rep = 0u64;
            let mut i = 0u32;
            while i < size {
                rep |= replicate << i;
                i += s;
            }
            if rep != mask {
                continue;
            }
            let we = mask.trailing_zeros();
            let w = (mask >> we).trailing_ones();
            if w == 0 || we + w > s {
                continue;
            }
            let immr = (we + w) % s;
            let imms = if n == 0 {
                ((s - w) << 1) | 1
            } else {
                (s - w) << 1
            };
            return Ok((n, immr, imms));
        }
    }
    Err(format!("cannot encode logical immediate #{mask:x}"))
}

pub fn write_u32_le(out: &mut Vec<u8>, word: u32) {
    out.extend_from_slice(&word.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_encodings() {
        assert_eq!(encode_ret(), 0xD65F_03C0);
        assert_eq!(encode_movz(Reg::X(0), 0, 0).unwrap(), 0xD280_0000);
        assert_eq!(encode_movz(Reg::W(0), 0, 0).unwrap(), 0x5280_0000);
        assert_eq!(encode_mov_reg(Reg::X(0), Reg::Xzr).unwrap(), 0xAA1F_03E0);
        assert_eq!(
            encode_stp_pre(Reg::X(29), Reg::X(30), Reg::Sp, -16).unwrap(),
            0xA9BF_7BFD
        );
        assert_eq!(
            encode_ldp_post(Reg::X(29), Reg::X(30), Reg::Sp, 16).unwrap(),
            0xA8C1_7BFD
        );
    }
}
