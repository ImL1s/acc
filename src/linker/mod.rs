//! Builtin linker (M4 freestanding + M5 hosted musl static).
//!
//! M4: freestanding `ET_EXEC` with injected `_start`.
//! M5: static musl `printf` hosted link without system `cc`/`ld`.
//! See `docs/notes/builtin_linker_m4.md`.

mod aarch64;
mod aarch64_hosted;
mod archive;
mod elf_read;

use crate::codegen::{Target, TargetOs};
use std::path::Path;

pub use aarch64::link_aarch64_linux;
pub use elf_read::{ObjectFile, ParsedReloc, ParsedSection, ParsedSymbol};

/// Link relocatable objects into an executable.
pub fn link_to_executable(
    objects: &[Vec<u8>],
    target: Target,
    target_os: TargetOs,
) -> Result<Vec<u8>, String> {
    match (target, target_os) {
        (Target::Aarch64, TargetOs::Linux) => {
            if aarch64_hosted::needs_hosted_link(objects)? {
                aarch64_hosted::link_aarch64_linux_hosted(objects)
            } else {
                aarch64::link_aarch64_linux(objects)
            }
        }
        _ => Err(format!(
            "builtin linker: unsupported target {}/{} (only aarch64/linux)",
            target.as_str(),
            target_os.as_str()
        )),
    }
}

/// Write executable to `out_path`.
pub fn link_files(
    object_paths: &[&Path],
    out_path: &Path,
    target: Target,
    target_os: TargetOs,
) -> Result<(), String> {
    let mut objs = Vec::with_capacity(object_paths.len());
    for p in object_paths {
        let bytes = std::fs::read(p).map_err(|e| format!("read {}: {e}", p.display()))?;
        objs.push(bytes);
    }
    let exe = link_to_executable(&objs, target, target_os)?;
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(out_path, exe).map_err(|e| format!("write {}: {e}", out_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(out_path)
            .map_err(|e| format!("stat {}: {e}", out_path.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(out_path, perms)
            .map_err(|e| format!("chmod {}: {e}", out_path.display()))?;
    }
    Ok(())
}

fn env_truthy(keys: &[&str]) -> bool {
    keys.iter().any(|k| {
        matches!(
            std::env::var(k).ok().as_deref(),
            Some("1") | Some("true") | Some("yes")
        )
    })
}

/// True when `ACC_BUILTIN_LD=1` or `GGCC_BUILTIN_LD=1` (feature must also be on).
pub fn env_opt_in() -> bool {
    env_truthy(&["ACC_BUILTIN_LD", "GGCC_BUILTIN_LD"])
}

/// When set with `env_opt_in()`, driver returns linker errors instead of falling back to system `cc`.
pub fn env_strict() -> bool {
    env_truthy(&["ACC_BUILTIN_LD_STRICT", "GGCC_BUILTIN_LD_STRICT"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler;

    fn freestanding_main_s() -> &'static str {
        r#"
	.text
	.globl	main
main:
	mov	w0, #0
	ret
"#
    }

    #[test]
    fn links_freestanding_main_to_et_exec() {
        let obj =
            assembler::assemble_to_object(freestanding_main_s(), Target::Aarch64, TargetOs::Linux)
                .expect("assemble");
        let exe = link_to_executable(&[obj], Target::Aarch64, TargetOs::Linux).expect("link");
        assert_eq!(&exe[0..4], b"\x7fELF");
        assert_eq!(exe[16], 2); // ET_EXEC
        assert_eq!(exe[18], 183); // EM_AARCH64
                                  // e_entry non-zero
        let entry = u64::from_le_bytes(exe[24..32].try_into().unwrap());
        assert!(entry >= 0x400000, "entry={entry:#x}");
        if let Ok(p) = std::env::var("ACC_WRITE_M4_EXE") {
            let _ = std::fs::write(p, &exe);
        }
    }

    #[test]
    fn links_and_runs_in_docker_without_system_cc() {
        let obj =
            assembler::assemble_to_object(freestanding_main_s(), Target::Aarch64, TargetOs::Linux)
                .expect("assemble");
        let exe = link_to_executable(&[obj], Target::Aarch64, TargetOs::Linux).expect("link");

        if !std::path::Path::new("/var/run/docker.sock").exists() {
            eprintln!("docker not available; skipping M4 run check");
            return;
        }
        let dir = std::env::temp_dir().join(format!("acc-m4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("prog"), &exe).unwrap();
        // No gcc/ld in the container — native aarch64 run of our ET_EXEC.
        let run = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "-v",
                &format!("{}:/work", dir.display()),
                "arm64v8/ubuntu:24.04",
                "bash",
                "-lc",
                "chmod +x /work/prog && /work/prog; echo EXIT:$?",
            ])
            .output()
            .expect("docker");
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            run.status.success() && stdout.contains("EXIT:0"),
            "M4 run failed: status={:?} stdout={stdout:?} stderr={:?}",
            run.status,
            String::from_utf8_lossy(&run.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn applies_local_bl_and_rodata_relocs() {
        let src = r#"
	.text
	.globl	main
main:
	stp	x29, x30, [sp, #-16]!
	mov	x29, sp
	bl	helper
	ldp	x29, x30, [sp], #16
	ret
helper:
	mov	w0, #7
	ret
	.section	.rodata
l_msg:
	.asciz	"x"
"#;
        let obj =
            assembler::assemble_to_object(src, Target::Aarch64, TargetOs::Linux).expect("assemble");
        let exe = link_to_executable(&[obj], Target::Aarch64, TargetOs::Linux).expect("link");
        assert_eq!(exe[16], 2);
    }

    #[test]
    fn rejects_unresolved_externals() {
        let src = r#"
	.text
	.globl	main
main:
	bl	printf
	ret
"#;
        let obj =
            assembler::assemble_to_object(src, Target::Aarch64, TargetOs::Linux).expect("assemble");
        let err = link_to_executable(&[obj], Target::Aarch64, TargetOs::Linux).unwrap_err();
        assert!(
            err.contains("unresolved") && err.contains("printf"),
            "err={err}"
        );
    }

    #[test]
    fn m5_assembles_hello_source() {
        let src = include_str!("../../tests/builtin_m5_hello.c");
        let asm = crate::codegen::emit_assembly_for_os(
            &crate::parser::parse(src).expect("parse"),
            Target::Aarch64,
            TargetOs::Linux,
        )
        .expect("codegen");
        let obj = assembler::assemble_to_object(&asm, Target::Aarch64, TargetOs::Linux)
            .expect("assemble hello");
        assert_eq!(&obj[0..4], b"\x7fELF");
    }

    #[test]
    fn m5_hosted_links_hello_printf() {
        let src = include_str!("../../tests/builtin_m5_hello.c");
        let asm = crate::codegen::emit_assembly_for_os(
            &crate::parser::parse(src).expect("parse"),
            Target::Aarch64,
            TargetOs::Linux,
        )
        .expect("codegen");
        let obj = assembler::assemble_to_object(&asm, Target::Aarch64, TargetOs::Linux)
            .expect("assemble hello");
        if !std::path::Path::new("/usr/lib/aarch64-linux-musl/libc.a").exists() {
            eprintln!("musl not installed; skipping M5 hosted link test");
            return;
        }
        let exe =
            link_to_executable(&[obj], Target::Aarch64, TargetOs::Linux).expect("hosted link");
        assert_eq!(&exe[0..4], b"\x7fELF");
        assert_eq!(exe[16], 2); // ET_EXEC
        if let Ok(p) = std::env::var("ACC_WRITE_M5_EXE") {
            let _ = std::fs::write(&p, &exe);
        }
        let dir = std::env::temp_dir().join(format!("acc-m5-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("prog"), &exe).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(dir.join("prog")).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(dir.join("prog"), perms).unwrap();
        }
        let run = std::process::Command::new(dir.join("prog"))
            .output()
            .expect("run");
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            run.status.success() && stdout.trim_end() == "Hello, world!",
            "run status={:?} stdout={stdout:?} stderr={:?}",
            run.status,
            String::from_utf8_lossy(&run.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
