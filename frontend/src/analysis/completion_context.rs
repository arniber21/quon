//! Context detection for IntelliSense completion (#174).

use crate::ast::{Decl, Expr, Stmt};
use crate::lexer::{SimpleSpan, Sp};

use super::cursor::{NodeAt, node_at_offset, partial_ident};

/// Where the cursor sits for filtering completion items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionContext {
    /// Immediately after `@` (gate / circuit application) — gates and applyables.
    AfterAt,
    /// After `:` or inside a type annotation / type argument list.
    TypePosition,
    /// General expression / statement position.
    Expression,
}

/// Detect completion context at a byte offset.
///
/// Prefers lexical cues (so incomplete buffers while typing still work), then
/// falls back to the smallest enclosing AST node.
pub fn completion_context_at(src: &str, decls: &[Sp<Decl>], offset: usize) -> CompletionContext {
    let offset = offset.min(src.len());
    let (ident_start, _, _) = partial_ident(src, offset);
    let before = skip_ws_back(src, ident_start);

    if let Some(b) = byte_before(src, before) {
        if b == b'@' {
            return CompletionContext::AfterAt;
        }
        if b == b':' || b == b'<' {
            return CompletionContext::TypePosition;
        }
    }

    if node_at_offset(decls, offset).is_some_and(|n| matches!(n, NodeAt::Type(_))) {
        return CompletionContext::TypePosition;
    }

    if inside_type_brackets(src, offset) {
        return CompletionContext::TypePosition;
    }

    CompletionContext::Expression
}

/// Whether `offset` lies inside a `circuit { … }` block.
pub fn in_circuit_block(decls: &[Sp<Decl>], offset: usize) -> bool {
    for (decl, _) in decls {
        if let Decl::Fn { body, .. } = decl
            && contains_circuit_block(&body.0, body.1, offset)
        {
            return true;
        }
    }
    false
}

fn contains_circuit_block(e: &Expr, span: SimpleSpan, offset: usize) -> bool {
    if !(span.start <= offset && offset <= span.end) {
        return false;
    }
    match e {
        Expr::CircuitBlock(_) => true,
        Expr::RunBlock(stmts) | Expr::Borrow { body: stmts, .. } => {
            stmts.iter().any(|s| stmt_has_circuit(s, offset))
        }
        Expr::Lam { body, .. }
        | Expr::Let { body, .. }
        | Expr::Bind { body, .. }
        | Expr::Neg(body)
        | Expr::Adjoint(body)
        | Expr::Controlled(body)
        | Expr::Return(body)
        | Expr::Ascribe(body, _) => contains_circuit_block(&body.0, body.1, offset),
        Expr::App(a, b)
        | Expr::Compose(a, b)
        | Expr::Par(a, b)
        | Expr::BinOp { lhs: a, rhs: b, .. }
        | Expr::GateApp { gate: a, qubits: b } => {
            contains_circuit_block(&a.0, a.1, offset) || contains_circuit_block(&b.0, b.1, offset)
        }
        Expr::If { cond, then, else_ } => {
            contains_circuit_block(&cond.0, cond.1, offset)
                || contains_circuit_block(&then.0, then.1, offset)
                || contains_circuit_block(&else_.0, else_.1, offset)
        }
        Expr::Match { scrutinee, arms } => {
            contains_circuit_block(&scrutinee.0, scrutinee.1, offset)
                || arms
                    .iter()
                    .any(|(_, arm)| contains_circuit_block(&arm.0, arm.1, offset))
        }
        Expr::For { iter, body, .. } => {
            contains_circuit_block(&iter.0, iter.1, offset)
                || contains_circuit_block(&body.0, body.1, offset)
        }
        Expr::Tuple(es) | Expr::List(es) => {
            es.iter().any(|e| contains_circuit_block(&e.0, e.1, offset))
        }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => false,
    }
}

fn stmt_has_circuit(stmt: &Sp<Stmt>, offset: usize) -> bool {
    match &stmt.0 {
        Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } | Stmt::Expr(rhs) => {
            contains_circuit_block(&rhs.0, rhs.1, offset)
        }
    }
}

fn skip_ws_back(src: &str, mut i: usize) -> usize {
    let bytes = src.as_bytes();
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i
}

fn byte_before(src: &str, i: usize) -> Option<u8> {
    if i == 0 {
        None
    } else {
        Some(src.as_bytes()[i - 1])
    }
}

/// Heuristic: inside `<…>` that looks like a type argument list (not comparison).
fn inside_type_brackets(src: &str, offset: usize) -> bool {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut i = offset;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'>' => depth += 1,
            b'<' => {
                if depth == 0 {
                    // Look further back for a type-ish prefix: Ident or `:`.
                    let before = skip_ws_back(src, i);
                    return matches!(byte_before(src, before), Some(b) if b == b':' || is_ident_part(b));
                }
                depth -= 1;
            }
            b')' | b']' | b'}' if depth == 0 => return false,
            b'(' | b'[' | b'{' if depth == 0 => return false,
            _ => {}
        }
    }
    false
}

fn is_ident_part(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Builtin type names offered in type position.
pub fn type_names() -> &'static [&'static str] {
    &[
        "Qubit", "QReg", "Bit", "Bool", "Int", "Float", "Unit", "Nat", "List", "Matrix", "Circuit",
        "Q",
    ]
}

/// Combinators / apply helpers offered after `@` alongside gates.
pub fn applyables() -> &'static [&'static str] {
    &["apply", "apply_dyn"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{analyze_program, cursor_at};

    fn ctx(src: &str) -> CompletionContext {
        let offset = cursor_at(src, "/*cursor*/");
        let clean = src.replace("/*cursor*/", "");
        // Adjust offset: marker removed, so offset in clean == offset in src (marker at end of prefix).
        let a = analyze_program(&clean);
        completion_context_at(&clean, &a.decls, offset)
    }

    #[test]
    fn after_at() {
        let src = "fn c(): Circuit<1,1,1,Clifford> = circuit { H @/*cursor*/ }\n";
        assert_eq!(ctx(src), CompletionContext::AfterAt);
    }

    #[test]
    fn type_after_colon() {
        let src = "fn f(x: /*cursor*/Int): Int = x\n";
        assert_eq!(ctx(src), CompletionContext::TypePosition);
    }

    #[test]
    fn expression_default() {
        let src = "fn f(): Int = /*cursor*/1\n";
        assert_eq!(ctx(src), CompletionContext::Expression);
    }
}
