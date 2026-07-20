mod ast;
mod codegen;
mod driver;
mod lexer;
mod parser;
mod preprocess;
mod token;

use codegen::{Target, TargetOs};
use driver::{CompileOptions, compile};
use std::env;
use std::path::PathBuf;
use std::process;

fn usage() -> ! {
    eprintln!(
        "ggcc — from-scratch minimal C compiler\n\
         usage: ggcc [-o <output>] [-S] [--keep-asm] [-m aarch64|x86_64] [--target-os darwin|linux]\n\
                [-I dir] [-Dname[=val]] <input.c>\n\
         \n\
         Compiles C with the in-tree frontend/codegen. System `cc` is used only\n\
         to assemble/link emitted assembly, never to compile the user's .c.\n\
         \n\
         -m aarch64          emit aarch64 (default)\n\
         -m x86_64           emit x86_64 System V\n\
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
    let mut keep_asm = false;
    let mut target = Target::Aarch64;
    let mut target_os = TargetOs::host();
    let mut input: Option<PathBuf> = None;
    let mut include_dirs: Vec<PathBuf> = Vec::new();
    let mut defines: Vec<(String, String)> = Vec::new();
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
            "--keep-asm" => keep_asm = true,
            "-m" => {
                let t = args.next().unwrap_or_else(|| {
                    eprintln!("ERROR: -m requires aarch64 or x86_64");
                    process::exit(2);
                });
                target = Target::parse(&t).unwrap_or_else(|| {
                    eprintln!("ERROR: unknown target '{t}' (use aarch64 or x86_64)");
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
                    defines.push((k.to_string(), v.to_string()));
                } else {
                    defines.push((p, String::new()));
                }
            }
            s if s.starts_with("-D") && s.len() > 2 => {
                let rest = &s[2..];
                if let Some((k, v)) = rest.split_once('=') {
                    defines.push((k.to_string(), v.to_string()));
                } else {
                    defines.push((rest.to_string(), String::new()));
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
                // Support -march style glue: -mx86_64 / -maarch64
                let t = &s[2..];
                // -m32/-m64 are ignored (kernel flags)
                if t == "32" || t == "64" || t.starts_with("cpu") || t.starts_with("arch") {
                    continue;
                }
                target = Target::parse(t).unwrap_or_else(|| {
                    eprintln!("ERROR: unknown target '{t}' (use aarch64 or x86_64)");
                    process::exit(2);
                });
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
