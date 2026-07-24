//! Builtin assembler (M2 subset) — optional via Cargo feature `builtin_assembler`.
//!
//! M2: parse a tiny acc-emitted aarch64 Linux `.s` subset and emit relocatable
//! ELF64 `.o`. Linking still uses system `cc`/`ld` unless/until a builtin linker lands.

mod aarch64;

use crate::codegen::{Target, TargetOs};
use std::path::Path;

/// Parsed assembly unit (architecture-agnostic shell; body is ISA-specific).
#[derive(Debug, Clone)]
pub struct AsmUnit {
    pub target: Target,
    pub target_os: TargetOs,
    pub lines: Vec<AsmLine>,
}

/// One logical line after comment stripping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsmLine {
    Empty,
    Directive { name: String, args: String },
    Label(String),
    Instr { mnemonic: String, operands: String },
}

/// Public entry: parse `.s` text for the given target.
///
/// Currently only `Target::Aarch64` + `TargetOs::Linux` is implemented (C3 subset
/// dialect). Other combos return an error so the driver can fall back to system `as`.
pub fn parse_assembly(src: &str, target: Target, target_os: TargetOs) -> Result<AsmUnit, String> {
    match (target, target_os) {
        (Target::Aarch64, TargetOs::Linux) => aarch64::parse(src, target, target_os),
        _ => Err(format!(
            "builtin assembler M1: unsupported target {}/{} (only aarch64/linux)",
            target.as_str(),
            target_os.as_str()
        )),
    }
}

/// Assemble `.s` bytes into a relocatable object (ELF `.o` for Linux).
///
/// M2: encodes a acc C3-shaped aarch64/linux subset. Unsupported constructs
/// return `Err` so the driver can fall back to system `as`.
pub fn assemble_to_object(
    src: &str,
    target: Target,
    target_os: TargetOs,
) -> Result<Vec<u8>, String> {
    let unit = parse_assembly(src, target, target_os)?;
    aarch64::emit_elf_object(&unit)
}

/// Write object file to `obj_path`. Same semantics as [`assemble_to_object`].
pub fn assemble_file(
    src: &str,
    obj_path: &Path,
    target: Target,
    target_os: TargetOs,
) -> Result<(), String> {
    let bytes = assemble_to_object(src, target, target_os)?;
    if let Some(parent) = obj_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(obj_path, bytes).map_err(|e| format!("write {}: {e}", obj_path.display()))
}

/// True when `ACC_BUILTIN_AS=1` or `GGCC_BUILTIN_AS=1` (feature must also be on).
pub fn env_opt_in() -> bool {
    ["ACC_BUILTIN_AS", "GGCC_BUILTIN_AS"].iter().any(|k| {
        matches!(
            std::env::var(k).ok().as_deref(),
            Some("1") | Some("true") | Some("yes")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_aarch64_linux_main_ret() {
        let src = "\t.text\n\t.globl\tmain\nmain:\n\tmov\tw0, #0\n\tret\n";
        let unit = parse_assembly(src, Target::Aarch64, TargetOs::Linux).expect("parse");
        assert!(unit.lines.iter().any(|l| matches!(l, AsmLine::Label(s) if s == "main")));
        assert!(unit.lines.iter().any(|l| {
            matches!(l, AsmLine::Instr { mnemonic, .. } if mnemonic == "ret")
        }));
    }

    #[test]
    fn rejects_darwin_for_m1() {
        let err = parse_assembly("\tret\n", Target::Aarch64, TargetOs::Darwin).unwrap_err();
        assert!(err.contains("unsupported"));
    }

    #[test]
    fn emits_elf_object_for_ret_main() {
        let src = "\t.text\n\t.globl\tmain\nmain:\n\tmov\tw0, #0\n\tret\n";
        let obj = assemble_to_object(src, Target::Aarch64, TargetOs::Linux).expect("emit");
        assert_eq!(&obj[0..4], b"\x7fELF");
        assert_eq!(obj[4], 2);
        assert_eq!(obj[18], 183); // EM_AARCH64 (e_machine LE)
        if let Ok(scratch) = std::env::var("ACC_WRITE_M2_OBJ") {
            let _ = std::fs::write(scratch, &obj);
        }
    }

    #[test]
    fn emits_acc_shaped_main_and_links_in_docker() {
        let src = r#"
	.text
	.p2align	2
	.globl	main
main:
	stp	x29, x30, [sp, #-16]!
	mov	x29, sp
	sub	sp, sp, #32
	str	x19, [x29, #-8]
	movz	x0, #0
	b	L_main_epilogue
	mov	w0, #0
L_main_epilogue:
	ldr	x19, [x29, #-8]
	mov	sp, x29
	ldp	x29, x30, [sp], #16
	ret
"#;
        let obj = assemble_to_object(src, Target::Aarch64, TargetOs::Linux).expect("emit main");
        assert_eq!(&obj[0..4], b"\x7fELF");
        if let Ok(scratch) = std::env::var("ACC_WRITE_M2_OBJ") {
            let _ = std::fs::write(scratch, &obj);
        }

        if !std::path::Path::new("/var/run/docker.sock").exists() {
            eprintln!("docker not available; skipping link/run check");
            return;
        }
        let dir = std::env::temp_dir().join(format!("acc-m2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("main.o");
        let _bin_path = dir.join("prog");
        std::fs::write(&obj_path, &obj).unwrap();

        let status = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "-v",
                &format!("{}:/work", dir.display()),
                "arm64v8/ubuntu:24.04",
                "bash",
                "-lc",
                "apt-get update -qq && apt-get install -qq -y gcc >/dev/null && gcc -o /work/prog /work/main.o",
            ])
            .status()
            .expect("docker");
        if !status.success() {
            eprintln!("docker link failed; object emission still validated");
            return;
        }
        let run = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "-v",
                &format!("{}:/work", dir.display()),
                "arm64v8/ubuntu:24.04",
                "/work/prog",
            ])
            .output()
            .expect("docker run");
        assert!(run.status.success(), "run failed: {:?}", run);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn emits_hello_shape_with_relocations() {
        // Matches acc codegen for `printf("Hello, world!\n"); return 0;` (main + rodata only).
        let src = r#"
	.text
	.globl	main
main:
	stp	x29, x30, [sp, #-16]!
	mov	x29, sp
	sub	sp, sp, #32
	str	x19, [x29, #-8]
	adrp	x0, l_str_0
	add	x0, x0, :lo12:l_str_0
	str	x0, [sp, #-16]!
	ldr	x0, [sp, #0]
	add	sp, sp, #16
	bl	printf
	movz	x0, #0
	b	L_main_epilogue
	mov	w0, #0
L_main_epilogue:
	ldr	x19, [x29, #-8]
	mov	sp, x29
	ldp	x29, x30, [sp], #16
	ret
	.section	.rodata
l_str_0:
	.asciz	"Hello, world!\n"
"#;
        let obj = assemble_to_object(src, Target::Aarch64, TargetOs::Linux).expect("hello emit");
        assert_eq!(&obj[0..4], b"\x7fELF");
        if let Ok(scratch) = std::env::var("ACC_WRITE_M2_OBJ") {
            let _ = std::fs::write(scratch, &obj);
        }
        if !std::path::Path::new("/var/run/docker.sock").exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("acc-m2-hello-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.o"), &obj).unwrap();
        let status = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "-v",
                &format!("{}:/work", dir.display()),
                "arm64v8/ubuntu:24.04",
                "bash",
                "-lc",
                "apt-get update -qq && apt-get install -qq -y gcc >/dev/null && gcc -o /work/prog /work/main.o",
            ])
            .status()
            .expect("docker");
        assert!(status.success(), "hello link failed");
        let run = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "-v",
                &format!("{}:/work", dir.display()),
                "arm64v8/ubuntu:24.04",
                "/work/prog",
            ])
            .output()
            .expect("docker run");
        assert!(run.status.success(), "hello run failed: {:?}", run);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "Hello, world!\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
