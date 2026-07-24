//! Collect names that are assigned / address-taken in a TU.
//! Used so mutable `static int x = 0` is never const-folded (flex `yy_init`).

use crate::ast::*;
use std::collections::HashSet;

pub fn collect_assigned_names_in_program(prog: &Program) -> HashSet<String> {
    let mut assigned = HashSet::new();
    for item in &prog.items {
        if let Item::Func(f) = item {
            if let Some(body) = &f.body {
                for s in body {
                    collect_assigned_names_stmt(s, &mut assigned);
                }
            }
        }
    }
    assigned
}

fn collect_assigned_names_expr(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Assign { left, right } | Expr::CompoundAssign { left, right, .. } => {
            if let Expr::Var(n) = left.as_ref() {
                out.insert(n.clone());
            }
            collect_assigned_names_expr(left, out);
            collect_assigned_names_expr(right, out);
        }
        Expr::PreInc(ex) | Expr::PreDec(ex) | Expr::PostInc(ex) | Expr::PostDec(ex) => {
            if let Expr::Var(n) = ex.as_ref() {
                out.insert(n.clone());
            }
            collect_assigned_names_expr(ex, out);
        }
        // `&static_var` escapes for external mutation — must not const-fold.
        Expr::Unary {
            op: UnaryOp::Addr,
            expr,
        } => {
            if let Expr::Var(n) = expr.as_ref() {
                out.insert(n.clone());
            }
            collect_assigned_names_expr(expr, out);
        }
        Expr::Call { args, .. } => {
            for a in args {
                collect_assigned_names_expr(a, out);
            }
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::SizeofExpr(expr)
        | Expr::Member { base: expr, .. } => collect_assigned_names_expr(expr, out),
        Expr::Binary { left, right, .. } | Expr::Index { base: left, index: right } => {
            collect_assigned_names_expr(left, out);
            collect_assigned_names_expr(right, out);
        }
        Expr::Cond {
            cond,
            then_e,
            else_e,
        } => {
            collect_assigned_names_expr(cond, out);
            collect_assigned_names_expr(then_e, out);
            collect_assigned_names_expr(else_e, out);
        }
        Expr::InitList { fields } => {
            for (_, e) in fields {
                collect_assigned_names_expr(e, out);
            }
        }
        Expr::StmtExpr(stmts, final_e) => {
            for s in stmts {
                collect_assigned_names_stmt(s, out);
            }
            collect_assigned_names_expr(final_e, out);
        }
        _ => {}
    }
}

fn collect_assigned_names_stmt(st: &Stmt, out: &mut HashSet<String>) {
    match st {
        Stmt::Block(ss) => {
            for s in ss {
                collect_assigned_names_stmt(s, out);
            }
        }
        Stmt::Decl(d) => {
            if let Some(init) = &d.init {
                collect_assigned_names_expr(init, out);
            }
        }
        Stmt::Expr(e) | Stmt::Return(Some(e)) => collect_assigned_names_expr(e, out),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::Goto(_) | Stmt::Empty => {}
        Stmt::GotoIndirect(e) => collect_assigned_names_expr(e, out),
        Stmt::If {
            cond,
            then_b,
            else_b,
        } => {
            collect_assigned_names_expr(cond, out);
            collect_assigned_names_stmt(then_b, out);
            if let Some(e) = else_b {
                collect_assigned_names_stmt(e, out);
            }
        }
        Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
            collect_assigned_names_expr(cond, out);
            collect_assigned_names_stmt(body, out);
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            if let Some(i) = init {
                collect_assigned_names_stmt(i, out);
            }
            if let Some(c) = cond {
                collect_assigned_names_expr(c, out);
            }
            if let Some(s) = step {
                collect_assigned_names_expr(s, out);
            }
            collect_assigned_names_stmt(body, out);
        }
        Stmt::Label(_, s) | Stmt::Default(s) => collect_assigned_names_stmt(s, out),
        Stmt::Switch { cond, body } | Stmt::Case { value: cond, body } => {
            collect_assigned_names_expr(cond, out);
            collect_assigned_names_stmt(body, out);
        }
        Stmt::DeclGroup(ds) => {
            for d in ds {
                if let Some(init) = &d.init {
                    collect_assigned_names_expr(init, out);
                }
            }
        }
        Stmt::Asm { .. } => {}
    }
}
