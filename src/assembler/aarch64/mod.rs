//! Tiny aarch64 Linux assembler subset for acc-emitted `.s` (M2 ELF encode).

mod elf;
mod emit;
mod encode;

use super::{AsmLine, AsmUnit};
use crate::codegen::{Target, TargetOs};

pub use emit::emit_elf_object;

/// Mnemonics accepted in the parse subset (operands validated at encode time).
const KNOWN_MNEMONICS: &[&str] = &[
    "ret", "nop", "mov", "movz", "movk", "movn", "add", "sub", "adds", "subs", "cmp", "cmn",
    "and", "orr", "eor", "bic", "lsl", "lsr", "asr", "neg", "mvn", "mul", "madd", "msub", "sdiv",
    "udiv", "ldr", "ldrb", "ldrh", "ldp", "str", "strb", "strh", "stp", "adrp", "adr", "bl", "b",
    "blr", "br", "cbz", "cbnz", "tbz", "tbnz", "b.eq", "b.ne", "b.lt", "b.le", "b.gt", "b.ge",
    "b.lo", "b.hs", "b.hi", "b.ls", "b.mi", "b.pl", "b.vs", "b.vc", "sxtw", "uxtw", "sxtb",
    "sxth", "csel", "cset", "fmov", "fadd", "fsub", "fmul", "fdiv", "fcmp", "scvtf", "fcvtzs",
    "rev", "rev16",
];

const KNOWN_DIRECTIVES: &[&str] = &[
    ".text",
    ".data",
    ".bss",
    ".rodata",
    ".section",
    ".globl",
    ".global",
    ".local",
    ".weak",
    ".p2align",
    ".align",
    ".type",
    ".size",
    ".file",
    ".loc",
    ".cfi_startproc",
    ".cfi_endproc",
    ".cfi_def_cfa",
    ".cfi_offset",
    ".comm",
    ".lcomm",
    ".byte",
    ".short",
    ".word",
    ".long",
    ".quad",
    ".ascii",
    ".asciz",
    ".string",
    ".zero",
    ".space",
    ".set",
    ".equ",
];

pub fn parse(src: &str, target: Target, target_os: TargetOs) -> Result<AsmUnit, String> {
    let mut lines = Vec::new();
    for (idx, raw) in src.lines().enumerate() {
        let lineno = idx + 1;
        let stripped = strip_comment(raw);
        let line = stripped.trim();
        if line.is_empty() {
            lines.push(AsmLine::Empty);
            continue;
        }
        if let Some(rest) = line.strip_suffix(':').filter(|s| is_label_name(s)) {
            lines.push(AsmLine::Label(rest.to_string()));
            continue;
        }
        if let Some((lab, after)) = line.split_once(':') {
            let lab = lab.trim();
            if is_label_name(lab) {
                lines.push(AsmLine::Label(lab.to_string()));
                let after = after.trim();
                if !after.is_empty() {
                    lines.push(parse_stmt(after, lineno)?);
                }
                continue;
            }
        }
        lines.push(parse_stmt(line, lineno)?);
    }
    Ok(AsmUnit {
        target,
        target_os,
        lines,
    })
}

fn parse_stmt(line: &str, lineno: usize) -> Result<AsmLine, String> {
    if line.starts_with('.') {
        let (name, args) = split_mnemonic(line);
        if !KNOWN_DIRECTIVES.iter().any(|d| *d == name)
            && !name.starts_with(".cfi_")
            && !name.starts_with(".section")
        {
            return Err(format!(
                "assembler aarch64 M1:{lineno}: unknown directive '{name}'"
            ));
        }
        return Ok(AsmLine::Directive {
            name: name.to_string(),
            args: args.to_string(),
        });
    }
    let (mnem, ops) = split_mnemonic(line);
    let mnem_l = mnem.to_ascii_lowercase();
    if !KNOWN_MNEMONICS.iter().any(|m| *m == mnem_l) {
        return Err(format!(
            "assembler aarch64 M1:{lineno}: unknown mnemonic '{mnem}'"
        ));
    }
    Ok(AsmLine::Instr {
        mnemonic: mnem_l,
        operands: ops.to_string(),
    })
}

fn split_mnemonic(line: &str) -> (&str, &str) {
    let line = line.trim();
    match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    }
}

fn strip_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            break;
        }
        if bytes[i] == b';' {
            break;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
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

    #[test]
    fn parses_directives_and_labels() {
        let src = r#"
	.text
	.p2align	2
	.globl	main
main:
	stp	x29, x30, [sp, #-16]!
	mov	x29, sp
	mov	w0, #0
	ldp	x29, x30, [sp], #16
	ret
"#;
        let u = parse(src, Target::Aarch64, TargetOs::Linux).expect("parse");
        assert!(u.lines.iter().any(|l| matches!(
            l,
            AsmLine::Directive { name, .. } if name == ".globl"
        )));
        assert_eq!(
            u.lines
                .iter()
                .filter(|l| matches!(l, AsmLine::Label(_)))
                .count(),
            1
        );
    }

    #[test]
    fn rejects_unknown_mnemonic() {
        let err = parse("\tfrobnicate\tx0\n", Target::Aarch64, TargetOs::Linux).unwrap_err();
        assert!(err.contains("unknown mnemonic"));
    }
}
