mod ast;
mod assigned_names;
mod codegen;
mod driver;
mod lexer;
mod parser;
mod preprocess;
mod token;

#[cfg(feature = "builtin_assembler")]
mod assembler;

#[cfg(feature = "builtin_linker")]
mod linker;

use codegen::{Target, TargetOs};
use driver::{CompileOptions, compile};
use std::env;
use std::path::PathBuf;
use std::process;

fn usage() -> ! {
    eprintln!(
        "acc — from-scratch minimal C compiler\n\
         usage: acc [-o <output>] [-S] [--keep-asm] [-m aarch64|x86_64|i686|riscv64]\n\
                [--target-os darwin|linux] [-I dir] [-Dname[=val]] <input.c>\n\
         \n\
         Compiles C with the in-tree frontend/codegen. System `cc` is used only\n\
         to assemble/link emitted assembly, never to compile the user's .c.\n\
         \n\
         -m aarch64          emit aarch64 (default)\n\
         -m x86_64           emit x86_64 System V\n\
         -m i686             emit i686 ILP32 (Linux ELF; link -m32 -no-pie)\n\
         -m riscv64          emit riscv64 LP64 (Linux ELF; prefer riscv64-linux-gnu-gcc)\n\
         --target-os darwin  Mach-O / Darwin asm (default on macOS host)\n\
         --target-os linux   ELF / Linux asm (for Docker Stage C)\n\
         -I dir              add include search path\n\
         -Dname[=val]        define macro (default val=1)\n\
         -include file       pre-include file (like first #include)"
    );
    process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mut output: Option<PathBuf> = None;
    let mut emit_asm_only = false;
    let mut preprocess_only = false;
    let mut keep_asm = false;
    let mut target = Target::Aarch64;
    let mut target_os = TargetOs::host();
    let mut input: Option<PathBuf> = None;
    let mut include_dirs: Vec<PathBuf> = Vec::new();
    // None = bare -DNAME; Some(v) = -DNAME=v (Some("") for empty replacement).
    let mut defines: Vec<(String, Option<String>)> = Vec::new();
    let mut force_includes: Vec<PathBuf> = Vec::new();

    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => usage(),
            "-o" => {
                let p = args.next().unwrap_or_else(|| {
                    eprintln!("ERROR: -o requires a path");
                    process::exit(2);
                });
                output = Some(PathBuf::from(p));
            }
            "-S" => emit_asm_only = true,
            "-E" => preprocess_only = true,
            "--keep-asm" => keep_asm = true,
            "-m" => {
                let t = args.next().unwrap_or_else(|| {
                    eprintln!("ERROR: -m requires aarch64, x86_64, i686, or riscv64");
                    process::exit(2);
                });
                target = Target::parse(&t).unwrap_or_else(|| {
                    eprintln!(
                        "ERROR: unknown target '{t}' (use aarch64, x86_64, i686, or riscv64)"
                    );
                    process::exit(2);
                });
            }
            "--target-os" => {
                let t = args.next().unwrap_or_else(|| {
                    eprintln!("ERROR: --target-os requires darwin or linux");
                    process::exit(2);
                });
                target_os = TargetOs::parse(&t).unwrap_or_else(|| {
                    eprintln!("ERROR: unknown target-os '{t}' (use darwin or linux)");
                    process::exit(2);
                });
            }
            "-I" => {
                let p = args.next().unwrap_or_else(|| {
                    eprintln!("ERROR: -I requires a directory");
                    process::exit(2);
                });
                include_dirs.push(PathBuf::from(p));
            }
            s if s.starts_with("-I") && s.len() > 2 => {
                include_dirs.push(PathBuf::from(&s[2..]));
            }
            "-D" => {
                let p = args.next().unwrap_or_else(|| {
                    eprintln!("ERROR: -D requires NAME or NAME=value");
                    process::exit(2);
                });
                if let Some((k, v)) = p.split_once('=') {
                    defines.push((k.to_string(), Some(v.to_string())));
                } else {
                    defines.push((p, None));
                }
            }
            s if s.starts_with("-D") && s.len() > 2 => {
                let rest = &s[2..];
                if let Some((k, v)) = rest.split_once('=') {
                    defines.push((k.to_string(), Some(v.to_string())));
                } else {
                    defines.push((rest.to_string(), None));
                }
            }
            "-include" => {
                let p = args.next().unwrap_or_else(|| {
                    eprintln!("ERROR: -include requires a file path");
                    process::exit(2);
                });
                force_includes.push(PathBuf::from(p));
            }
            s if s.starts_with("-include") && s.len() > 8 => {
                // -includeFILE (no space) — rare but accept
                force_includes.push(PathBuf::from(&s[8..]));
            }
            s if s.starts_with("-m") && s.len() > 2 => {
                let t = &s[2..];
                if let Some(parsed) = Target::parse(t) {
                    target = parsed;
                }
                continue;
            }
            s if s.starts_with('-') => {
                // Silently ignore unknown gcc-style flags (kernel builds pass many).
                // Critical flags (-I/-D/-include/-S/-o/-m) are handled above.
                continue;
            }
            s => {
                if input.is_some() {
                    eprintln!("ERROR: multiple inputs not supported yet");
                    process::exit(2);
                }
                input = Some(PathBuf::from(s));
            }
        }
    }

    let input = input.unwrap_or_else(|| usage());
    let output = output.unwrap_or_else(|| {
        if emit_asm_only {
            input.with_extension("s")
        } else {
            PathBuf::from("a.out")
        }
    });

    // Policy: never special-case fixture basenames.
    debug_assert!(!driver::is_forbidden_fixture_path_special_case(&input));

    if preprocess_only {
        let src = match std::fs::read_to_string(&input) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ERROR: failed to read input file {}: {}", input.display(), e);
                process::exit(1);
            }
        };
        // Match `compile()`: inject -D / -include before preprocessing so
        // `acc -E -DFOO=` agrees with `acc -S -DFOO=` (empty #define REDIS_STATIC=).
        let mut prefixed = String::new();
        for (k, v) in &defines {
            match v {
                None => prefixed.push_str(&format!("#define {k} 1\n")),
                Some(val) if val.is_empty() => prefixed.push_str(&format!("#define {k}\n")),
                Some(val) => prefixed.push_str(&format!("#define {k} {val}\n")),
            }
        }
        for fi in &force_includes {
            let abs = if fi.is_absolute() {
                fi.clone()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(fi)
            };
            prefixed.push_str(&format!("#include \"{}\"\n", abs.display()));
        }
        prefixed.push_str(&src);
        let inc_paths: Vec<&std::path::Path> = include_dirs.iter().map(|p| p.as_path()).collect();
        let first_inc = inc_paths.first().copied();
        let rest_inc = if inc_paths.is_empty() {
            &[]
        } else {
            &inc_paths[1..]
        };
        let for_linux = target_os == TargetOs::Linux;
        match preprocess::preprocess_with_options(
            &prefixed,
            first_inc,
            rest_inc,
            for_linux,
            &input.to_string_lossy(),
        ) {
            Ok(pp) => {
                if output.as_os_str() != "a.out" {
                    let _ = std::fs::write(output, pp);
                } else {
                    println!("{pp}");
                }
                process::exit(0);
            }
            Err(e) => {
                eprintln!("ERROR: {e}");
                process::exit(1);
            }
        }
    }

    if let Err(e) = compile(&CompileOptions {
        input,
        output,
        keep_asm,
        emit_asm_only,
        target,
        target_os,
        linker: None,
        include_dirs,
        defines,
        force_includes,
    }) {
        eprintln!("ERROR: {e}");
        process::exit(1);
    }
}
