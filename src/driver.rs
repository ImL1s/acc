//! Compile C with the from-scratch pipeline; assemble/link with system tools only.
//! Never invokes an external C compiler on the user's .c file.

use crate::codegen::{self, Target, TargetOs};
use crate::parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub keep_asm: bool,
    pub emit_asm_only: bool,
    pub target: Target,
    pub target_os: TargetOs,
    /// Optional assembler/linker program (default: `cc`). Used for cross tools in Docker.
    pub linker: Option<String>,
    /// Additional `-I` include search paths (kernel builds need many).
    pub include_dirs: Vec<PathBuf>,
    /// Predefined macros from `-DNAME` / `-DNAME=value`.
    /// `None` = bare `-DNAME` (gcc: `#define NAME 1`);
    /// `Some("")` = `-DNAME=` (empty replacement, needed for `SQLITE_PRIVATE=""`);
    /// `Some(v)` = `-DNAME=v`.
    pub defines: Vec<(String, Option<String>)>,
    /// Files from gcc-style `-include path` (pre-included before the TU).
    pub force_includes: Vec<PathBuf>,
}

/// Entrypoint for compilation. Spawns a dedicated compiler thread with a 64MB stack
/// size to prevent stack overflow when parsing large preprocessed translation units (~148k lines).
pub fn compile(opts: &CompileOptions) -> Result<(), String> {
    const STACK_SIZE: usize = 64 * 1024 * 1024; // 64 MB
    let opts_clone = opts.clone();
    let handle = std::thread::Builder::new()
        .name("ggcc-compiler".into())
        .stack_size(STACK_SIZE)
        .spawn(move || compile_internal(&opts_clone))
        .map_err(|e| format!("failed to spawn compiler thread: {e}"))?;

    match handle.join() {
        Ok(res) => res,
        Err(_) => Err("compiler thread panicked (stack overflow or internal error)".into()),
    }
}

fn compile_internal(opts: &CompileOptions) -> Result<(), String> {
    let src = fs::read_to_string(&opts.input)
        .map_err(|e| format!("read {}: {e}", opts.input.display()))?;

    // Preprocess before lex/parse (handles #define / #if; local #include "..." from input dir).
    let inc_dir = opts.input.parent();
    let for_linux = opts.target_os == TargetOs::Linux;
    let source_name = opts
        .input
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("input.c");
    let extra: Vec<&std::path::Path> = opts.include_dirs.iter().map(|p| p.as_path()).collect();
    // Inject -D macros and -include files at the top of the TU (gcc order).
    let mut prefixed = String::new();
    for (k, v) in &opts.defines {
        match v {
            // Bare -DNAME → #define NAME 1 (gcc default).
            None => prefixed.push_str(&format!("#define {k} 1\n")),
            // -DNAME= → empty replacement (critical for SQLITE_PRIVATE="").
            Some(val) if val.is_empty() => prefixed.push_str(&format!("#define {k}\n")),
            Some(val) => prefixed.push_str(&format!("#define {k} {val}\n")),
        }
    }
    // `-include file` ≡ `#include "file"` before the primary source.
    // gcc resolves relative -include paths against the process CWD (kernel make
    // runs from the tree root with `-include ./include/linux/kconfig.h`).
    for fi in &opts.force_includes {
        let abs = if fi.is_absolute() {
            fi.clone()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(fi)
        };
        // Absolute path in the directive so the preprocessor does not join it
        // against the .c file's parent directory.
        prefixed.push_str(&format!("#include \"{}\"\n", abs.display()));
    }
    prefixed.push_str(&src);
    let arch = match opts.target {
        Target::X86_64 => "x86_64",
        Target::Aarch64 => "aarch64",
    };
    let src = crate::preprocess::preprocess_with_options_arch(
        &prefixed,
        inc_dir,
        &extra,
        for_linux,
        source_name,
        arch,
    )?;
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
            include_dirs: Vec::new(),
            defines: Vec::new(),
            force_includes: Vec::new(),
        })
        .expect("compile");
        let out = Command::new(&bin_path).output().expect("run");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), msg);
    }

    #[test]
    fn compiles_and_runs_scope_shadowing_program() {
        let dir = {
            let mut d = std::env::temp_dir();
            d.push(format!(
                "ggcc-shadow-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&d).unwrap();
            d
        };
        let src_path = dir.join("shadow.c");
        let bin_path_arm = dir.join("shadow_arm");
        let bin_path_x86 = dir.join("shadow_x86");
        let code = r#"
        int main(void) {
            int x = 10;
            {
                int x = 20;
                if (x != 20) return 1;
            }
            if (x != 10) return 2;

            int y = ({ int x = 30; x + 5; });
            if (x != 10 || y != 35) return 3;

            return 0;
        }
        "#;
        fs::write(&src_path, code).unwrap();

        compile(&CompileOptions {
            input: src_path.clone(),
            output: bin_path_arm.clone(),
            keep_asm: false,
            emit_asm_only: false,
            target: Target::Aarch64,
            target_os: TargetOs::host(),
            linker: None,
            include_dirs: Vec::new(),
            defines: Vec::new(),
            force_includes: Vec::new(),
        })
        .expect("compile aarch64");
        let out_arm = Command::new(&bin_path_arm).output().expect("run arm");
        assert_eq!(out_arm.status.code(), Some(0));

        compile(&CompileOptions {
            input: src_path,
            output: bin_path_x86.clone(),
            keep_asm: false,
            emit_asm_only: false,
            target: Target::X86_64,
            target_os: TargetOs::host(),
            linker: None,
            include_dirs: Vec::new(),
            defines: Vec::new(),
            force_includes: Vec::new(),
        })
        .expect("compile x86_64");

        if Command::new("arch").arg("-x86_64").arg("true").output().map(|o| o.status.success()).unwrap_or(false) {
            let out_x86 = Command::new("arch").arg("-x86_64").arg(&bin_path_x86).output().expect("run x86");
            assert_eq!(out_x86.status.code(), Some(0));
        }
    }
}
