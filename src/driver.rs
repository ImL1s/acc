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
        .name("acc-compiler".into())
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
        Target::I686 => "i386",
        Target::Riscv64 => "riscv64",
    };
    let src = crate::preprocess::preprocess_with_options_arch(
        &prefixed,
        inc_dir,
        &extra,
        for_linux,
        source_name,
        arch,
    )?;
    if let Some(path) = std::env::var_os("ACC_DUMP_PP") {
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

    // Optional builtin assembler (feature `builtin_assembler` + ACC_BUILTIN_AS=1).
    // On success, produce `.o` for system or builtin link. On any error, fall back
    // to the default system assemble/link of `.s`.
    #[cfg(feature = "builtin_assembler")]
    let link_input: PathBuf = {
        if crate::assembler::env_opt_in() {
            let obj_path = opts.output.with_extension("o");
            match crate::assembler::assemble_file(&asm, &obj_path, opts.target, opts.target_os) {
                Ok(()) => obj_path,
                Err(e) => {
                    #[cfg(feature = "builtin_linker")]
                    if crate::linker::env_strict() {
                        return Err(format!("builtin assembler failed: {e}"));
                    }
                    // Keep default system-as path until M1 encode is green.
                    asm_path.clone()
                }
            }
        } else {
            asm_path.clone()
        }
    };
    #[cfg(not(feature = "builtin_assembler"))]
    let link_input: PathBuf = asm_path.clone();

    // Optional builtin linker (feature `builtin_linker` + ACC_BUILTIN_LD=1`).
    // Freestanding aarch64 (M4) or static musl hosted (M5). With
    // ACC_BUILTIN_LD_STRICT=1, linker errors are fatal (no system `cc` fallback).
    #[cfg(feature = "builtin_linker")]
    {
        if crate::linker::env_opt_in() {
            if link_input.extension().and_then(|e| e.to_str()) != Some("o") {
                if crate::linker::env_strict() {
                    return Err(
                        "builtin link requires a `.o` input (enable builtin assembler or fix asm)"
                            .into(),
                    );
                }
            } else {
                match crate::linker::link_files(
                    &[&link_input],
                    &opts.output,
                    opts.target,
                    opts.target_os,
                ) {
                    Ok(()) => {
                        if !opts.keep_asm {
                            let _ = fs::remove_file(&asm_path);
                        }
                        if link_input != opts.output {
                            let _ = fs::remove_file(&link_input);
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        if crate::linker::env_strict() {
                            return Err(e);
                        }
                        // Fall through to system linker (non-marker paths only).
                    }
                }
            }
        }
    }

    // Pick linker: optional override, else cross tool for riscv64, else `cc`.
    let default_linker = match opts.target {
        Target::Riscv64 if opts.linker.is_none() => {
            if Command::new("riscv64-linux-gnu-gcc")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                "riscv64-linux-gnu-gcc"
            } else {
                "cc"
            }
        }
        _ => "cc",
    };
    let linker = opts.linker.as_deref().unwrap_or(default_linker);
    let mut cmd = Command::new(linker);
    // Select assemble/link ISA. Never pass the user's .c — only our .s/.o.
    // On macOS host with Darwin dialect: clang -arch.
    if opts.target_os == TargetOs::Darwin && cfg!(target_os = "macos") {
        match opts.target {
            Target::Aarch64 => {
                cmd.arg("-arch").arg("arm64");
            }
            Target::X86_64 => {
                cmd.arg("-arch").arg("x86_64");
            }
            Target::I686 | Target::Riscv64 => {
                return Err(format!(
                    "target {} does not support Darwin assemble/link on this host",
                    opts.target.as_str()
                ));
            }
        }
    }
    // i686 Linux ELF: ILP32 absolute addressing requires -m32 -no-pie.
    if opts.target == Target::I686 && opts.target_os == TargetOs::Linux {
        cmd.arg("-m32").arg("-no-pie");
    }
    // riscv64: prefer static when using the cross gcc (qemu-user friendly).
    if opts.target == Target::Riscv64 && opts.target_os == TargetOs::Linux {
        cmd.arg("-static");
    }
    // Linux ELF: native cc inside Docker / Linux host.
    cmd.arg("-o").arg(&opts.output).arg(&link_input);
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

    #[cfg(target_os = "macos")]
    if opts.target_os == TargetOs::Darwin {
        let _ = Command::new("codesign")
            .arg("-s")
            .arg("-")
            .arg(&opts.output)
            .output();
    }

    if !opts.keep_asm {
        let _ = fs::remove_file(&asm_path);
    }
    #[cfg(feature = "builtin_assembler")]
    {
        // Drop intermediate .o when we produced one and it is not the final output.
        if link_input != asm_path && link_input != opts.output {
            let _ = fs::remove_file(&link_input);
        }
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
                "acc-test-{}-{}",
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
                "acc-shadow-test-{}-{}",
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
