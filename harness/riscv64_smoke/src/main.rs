//! Emit riscv64 asm for hand-built oracle-shaped programs; write .s to out/.
//!
//! Usage:
//!   cargo run --manifest-path harness/riscv64_smoke/Cargo.toml -- <hello|return_code|arith|multi_fn> [out.s]
//!
//! Then: harness/riscv64_smoke/run_qemu.sh out/<name>.s

use riscv64_smoke::ast::*;
use riscv64_smoke::codegen_riscv::emit_assembly;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

fn prog_return_code() -> Program {
    Program {
        items: vec![Item::Func(Function {
            name: "main".into(),
            ret: Type::Int,
            params: vec![],
            variadic: false,
            body: Some(vec![Stmt::Return(Some(Expr::Int(7)))]),
            is_static: false,
            is_weak: false,
            section: None,
        })],
        type_layouts: vec![],
    }
}

fn prog_arith() -> Program {
    // return 10 + 20 + 12  → exit 42
    Program {
        items: vec![Item::Func(Function {
            name: "main".into(),
            ret: Type::Int,
            params: vec![],
            variadic: false,
            body: Some(vec![Stmt::Return(Some(Expr::Binary {
                op: BinOp::Add,
                left: Box::new(Expr::Binary {
                    op: BinOp::Add,
                    left: Box::new(Expr::Int(10)),
                    right: Box::new(Expr::Int(20)),
                }),
                right: Box::new(Expr::Int(12)),
            }))]),
            is_static: false,
            is_weak: false,
            section: None,
        })],
        type_layouts: vec![],
    }
}

fn prog_hello() -> Program {
    Program {
        items: vec![
            Item::Func(Function {
                name: "printf".into(),
                ret: Type::Int,
                params: vec![("fmt".into(), Type::Ptr(Box::new(Type::Char)))],
                variadic: true,
                body: None,
                is_static: false,
                is_weak: false,
                section: None,
            }),
            Item::Func(Function {
                name: "main".into(),
                ret: Type::Int,
                params: vec![],
                variadic: false,
                body: Some(vec![
                    Stmt::Expr(Expr::Call {
                        name: "printf".into(),
                        args: vec![Expr::String("Hello, world!\n".into())],
                    }),
                    Stmt::Return(Some(Expr::Int(0))),
                ]),
                is_static: false,
                is_weak: false,
                section: None,
            }),
        ],
        type_layouts: vec![],
    }
}

fn prog_multi_fn() -> Program {
    // add(20,22) - 42 → 0
    Program {
        items: vec![
            Item::Func(Function {
                name: "add".into(),
                ret: Type::Int,
                params: vec![("a".into(), Type::Int), ("b".into(), Type::Int)],
                variadic: false,
                body: Some(vec![Stmt::Return(Some(Expr::Binary {
                    op: BinOp::Add,
                    left: Box::new(Expr::Var("a".into())),
                    right: Box::new(Expr::Var("b".into())),
                }))]),
                is_static: false,
                is_weak: false,
                section: None,
            }),
            Item::Func(Function {
                name: "main".into(),
                ret: Type::Int,
                params: vec![],
                variadic: false,
                body: Some(vec![
                    Stmt::Decl(VarDecl {
                        name: "x".into(),
                        ty: Type::Int,
                        init: Some(Expr::Call {
                            name: "add".into(),
                            args: vec![Expr::Int(20), Expr::Int(22)],
                        }),
                        is_static: false,
                        is_extern: false,
                        is_weak: false,
                        section: None,
                    }),
                    Stmt::Return(Some(Expr::Binary {
                        op: BinOp::Sub,
                        left: Box::new(Expr::Var("x".into())),
                        right: Box::new(Expr::Int(42)),
                    })),
                ]),
                is_static: false,
                is_weak: false,
                section: None,
            }),
        ],
        type_layouts: vec![],
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let which = args.next().unwrap_or_else(|| {
        eprintln!("usage: riscv64_smoke <hello|return_code|arith|multi_fn> [out.s]");
        process::exit(2);
    });
    let prog = match which.as_str() {
        "hello" => prog_hello(),
        "return_code" => prog_return_code(),
        "arith" => prog_arith(),
        "multi_fn" => prog_multi_fn(),
        other => {
            eprintln!("unknown program '{other}'");
            process::exit(2);
        }
    };
    let asm = emit_assembly(&prog).unwrap_or_else(|e| {
        eprintln!("emit failed: {e}");
        process::exit(1);
    });
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("harness/riscv64_smoke/out/{which}.s")));
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&out, &asm).unwrap_or_else(|e| {
        eprintln!("write {}: {e}", out.display());
        process::exit(1);
    });
    println!("wrote {} ({} bytes)", out.display(), asm.len());
}
