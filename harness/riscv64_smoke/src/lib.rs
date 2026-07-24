//! Path-include ggcc AST + riscv64 codegen without touching driver/main.

#[path = "../../../src/ast.rs"]
pub mod ast;

#[path = "../../../src/codegen_riscv.rs"]
pub mod codegen_riscv;
