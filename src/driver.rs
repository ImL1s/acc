//! Compile C with the from-scratch pipeline; assemble/link with system tools only.
//! Never invokes an external C compiler on the user's .c file.

use crate::codegen::{self, Target, TargetOs};
use crate::parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct CompileOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub keep_asm: bool,
    pub emit_asm_only: bool,
    pub target: Target,
    pub target_os: TargetOs,
    /// Optional assembler/linker program (default: `cc`). Used for cross tools in Docker.
    pub linker: Option<String>,
}

pub fn compile(opts: &CompileOptions) -> Result<(), String> {
    let src = fs::read_to_string(&opts.input)
        .map_err(|e| format!("read {}: {e}", opts.input.display()))?;

    // Preprocess before lex/parse (handles #define / #if; local #include "..." from input dir).
    let inc_dir = opts.input.parent();
    let for_linux = opts.target_os == TargetOs::Linux;
    let src = crate::preprocess::preprocess_with_options(&src, inc_dir, for_linux)?;
    if let Some(path) = std::env::var_os("GGCC_DUMP_PP") {
        let _ = fs::write(&path, &src);
    }
    let prog = parser::parse(&src)?;
    let asm = codegen::emit_assembly_for_os(&prog, opts.target, opts.target_os)?;

    let asm_path = if opts.emit_asm_only {
        opts.output.clone()
    } else {
        opts.output.with_extension("s")
    };

    if let Some(parent) = asm_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }

    fs::write(&asm_path, &asm).map_err(|e| format!("write {}: {e}", asm_path.display()))?;

    if opts.emit_asm_only {
        return Ok(());
    }

    let linker = opts.linker.as_deref().unwrap_or("cc");
    let mut cmd = Command::new(linker);
    // Select assemble/link ISA. Never pass the user's .c — only our .s.
    // On macOS host with Darwin dialect: clang -arch.
    if opts.target_os == TargetOs::Darwin && cfg!(target_os = "macos") {
        match opts.target {
            Target::Aarch64 => {
                cmd.arg("-arch").arg("arm64");
            }
            Target::X86_64 => {
                cmd.arg("-arch").arg("x86_64");
            }
        }
    }
    // Linux ELF: native cc inside Docker / Linux host.
    cmd.arg("-o").arg(&opts.output).arg(&asm_path);
    // libm for sin/cos/etc when referenced by codegen
    cmd.arg("-lm");

    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn system assembler/linker ({linker}): {e}"))?;

    if !status.success() {
        return Err(format!(
            "system assembler/linker failed with status {status} (asm {}, target {}/{})",
            asm_path.display(),
            opts.target.as_str(),
            opts.target_os.as_str()
        ));
    }

    if !opts.keep_asm {
        let _ = fs::remove_file(&asm_path);
    }
    Ok(())
}

pub fn is_forbidden_fixture_path_special_case(path: &Path) -> bool {
    let _ = path;
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn compiles_and_runs_hello_like_program() {
        let dir = {
            let mut d = std::env::temp_dir();
            d.push(format!(
                "ggcc-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&d).unwrap();
            d
        };
        let src_path = dir.join("t.c");
        let bin_path = dir.join("t");
        let msg = "driver-unit-hello";
        fs::write(
            &src_path,
            format!("#include <stdio.h>\nint main(void) {{ printf(\"{msg}\\n\"); return 0; }}\n"),
        )
        .unwrap();
        compile(&CompileOptions {
            input: src_path,
            output: bin_path.clone(),
            keep_asm: false,
            emit_asm_only: false,
            target: Target::Aarch64,
            target_os: TargetOs::host(),
            linker: None,
        })
        .expect("compile");
        let out = Command::new(&bin_path).output().expect("run");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), msg);
    }
}
