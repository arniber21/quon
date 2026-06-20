// Pretty-printer — emits valid Quon source from an AST.
//
// The printer is *roundtrip-faithful*: `parse(lex(pretty(d)))` equals `d` up to spans
// (see frontend/tests/support). It achieves this by parenthesizing every operator/binding
// form uniformly, so precedence and associativity can never be misread on re-parse. The
// output is intentionally explicit rather than minimal — it backs the generative fuzzer
// (frontend/fuzz/fuzz_roundtrip) and doubles as a debug dumper.

use crate::ast::*;
use crate::lexer::Sp;

/// Render a sequence of declarations to Quon source.
pub fn pretty(decls: &[Sp<Decl>]) -> String {
    decls
        .iter()
        .map(|(d, _)| decl_str(d))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn decl_str(d: &Decl) -> String {
    match d {
        Decl::Fn {
            name,
            params,
            ret,
            body,
        } => {
            let ps = params
                .iter()
                .map(|(n, t)| format!("{n}: {}", ty_str(&t.0)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn {name}({ps}): {} = {}", ty_str(&ret.0), e_str(body))
        }
        Decl::TypeAlias { name, params, ty } => {
            let head = if params.is_empty() {
                name.clone()
            } else {
                format!("{name}<{}>", params.join(", "))
            };
            format!("type {head} = {}", ty_str(&ty.0))
        }
    }
}

// ── Types ─────────────────────────────────────────────────────────────────────

fn ty_str(t: &Type) -> String {
    match t {
        Type::Qubit => "Qubit".into(),
        Type::Bit => "Bit".into(),
        Type::Bool => "Bool".into(),
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::Unit => "Unit".into(),
        Type::Nat => "Nat".into(),
        Type::QReg(n) => format!("QReg<{}>", nat_str(&n.0)),
        Type::Q(t) => format!("Q<{}>", ty_str(&t.0)),
        Type::List(t) => format!("List<{}>", ty_str(&t.0)),
        Type::Matrix(n, m, t) => {
            format!(
                "Matrix<{}, {}, {}>",
                nat_str(&n.0),
                nat_str(&m.0),
                ty_str(&t.0)
            )
        }
        Type::Circuit { n, m, d, c } => format!(
            "Circuit<{}, {}, {}, {}>",
            nat_str(&n.0),
            nat_str(&m.0),
            nat_str(&d.0),
            class_str(c)
        ),
        Type::Fn(a, b) => format!("({}) -> ({})", ty_str(&a.0), ty_str(&b.0)),
        Type::Linear(a, b) => format!("({}) -o ({})", ty_str(&a.0), ty_str(&b.0)),
        Type::Tuple(ts) => {
            let inner = ts
                .iter()
                .map(|t| ty_str(&t.0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        Type::Var(n) => n.clone(),
        Type::Named { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                let a = args
                    .iter()
                    .map(|n| nat_str(&n.0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}<{a}>")
            }
        }
    }
}

fn class_str(c: &CliffordClass) -> &'static str {
    match c {
        CliffordClass::Clifford => "Clifford",
        CliffordClass::Universal => "Universal",
        // Infer has no surface syntax; the generator never produces it. Fall back so this
        // stays total.
        CliffordClass::Infer => "Clifford",
    }
}

fn nat_str(n: &NatExpr) -> String {
    match n {
        NatExpr::Lit(v) => v.to_string(),
        NatExpr::Var(name) => name.clone(),
        NatExpr::Hole => "_".into(),
        NatExpr::Add(a, b) => format!("({}) + ({})", nat_str(&a.0), nat_str(&b.0)),
        NatExpr::Sub(a, b) => format!("({}) - ({})", nat_str(&a.0), nat_str(&b.0)),
        NatExpr::Mul(a, b) => format!("({}) * ({})", nat_str(&a.0), nat_str(&b.0)),
        NatExpr::Div(a, b) => format!("({}) / ({})", nat_str(&a.0), nat_str(&b.0)),
        NatExpr::Exp(a, b) => format!("({}) ^ ({})", nat_str(&a.0), nat_str(&b.0)),
    }
}

// ── Patterns ──────────────────────────────────────────────────────────────────

fn pat_str(p: &Pat) -> String {
    match p {
        Pat::Wildcard => "_".into(),
        Pat::Var(n) => n.clone(),
        Pat::Tuple(ps) => {
            let inner = ps
                .iter()
                .map(|p| pat_str(&p.0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        Pat::Lit(LitPat::Int(n)) => n.to_string(),
        Pat::Lit(LitPat::Bool(b)) => b.to_string(),
    }
}

// ── Expressions ───────────────────────────────────────────────────────────────

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Pow => "^",
    }
}

fn float_str(f: f64) -> String {
    let s = format!("{f:?}");
    // The lexer only accepts floats containing a `.`; ensure one is present.
    if s.contains('.')
        || s.contains('e')
        || s.contains('E')
        || s.contains("inf")
        || s.contains("NaN")
    {
        s
    } else {
        format!("{s}.0")
    }
}

/// Render an expression in a position where it must be a parsing *atom* (operand of an
/// operator). Self-delimiting and leaf forms are emitted bare; everything else is wrapped
/// in parentheses, which always re-parses to the same node.
fn atom(e: &Sp<Expr>) -> String {
    match &e.0 {
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Bool(_)
        | Expr::Unit
        | Expr::Var(_)
        | Expr::Tuple(_)
        | Expr::List(_)
        | Expr::CircuitBlock(_)
        | Expr::RunBlock(_)
        | Expr::Borrow { .. }
        | Expr::For { .. }
        | Expr::Match { .. }
        | Expr::Adjoint(_)
        | Expr::Controlled(_) => e_str(e),
        // `par { .. } * count` extends right (the count), so it must be parenthesized when used
        // as an operand; everything else here is an operator/binding form that also needs parens.
        _ => format!("({})", e_str(e)),
    }
}

fn block_str(keyword: &str, stmts: &[Sp<Stmt>]) -> String {
    if stmts.is_empty() {
        return format!("{keyword} {{\n}}");
    }
    let body = stmts
        .iter()
        .map(|(s, _)| format!("    {}", stmt_str(s)))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{keyword} {{\n{body}\n}}")
}

fn stmt_str(s: &Stmt) -> String {
    match s {
        Stmt::Bind { pat, rhs } => format!("{} <- {}", pat_str(&pat.0), e_str(rhs)),
        Stmt::Let { pat, rhs } => format!("let {} = {}", pat_str(&pat.0), e_str(rhs)),
        Stmt::Expr(e) => e_str(e),
    }
}

fn e_str(e: &Sp<Expr>) -> String {
    match &e.0 {
        Expr::Int(n) => n.to_string(),
        Expr::Float(f) => float_str(*f),
        Expr::Bool(b) => b.to_string(),
        Expr::Unit => "()".into(),
        Expr::Var(n) => n.clone(),

        Expr::App(f, x) => format!("{}({})", atom(f), e_str(x)),
        Expr::BinOp { op, lhs, rhs } => {
            format!("{} {} {}", atom(lhs), binop_str(*op), atom(rhs))
        }
        // The space matters: `-o…`/`->` would otherwise lex as the linear-arrow / arrow tokens.
        Expr::Neg(x) => format!("- {}", atom(x)),

        Expr::Lam { params, body } => {
            let ps = params
                .iter()
                .map(|(p, t)| match t {
                    Some(t) => format!("{}: {}", pat_str(&p.0), ty_str(&t.0)),
                    None => pat_str(&p.0),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn({ps}) -> {}", e_str(body))
        }
        Expr::Let { pat, rhs, body } => {
            format!(
                "let {} = {} in {}",
                pat_str(&pat.0),
                e_str(rhs),
                e_str(body)
            )
        }
        Expr::If { cond, then, else_ } => {
            format!(
                "if {} then {} else {}",
                e_str(cond),
                e_str(then),
                e_str(else_)
            )
        }
        Expr::Match { scrutinee, arms } => {
            let arms_s = arms
                .iter()
                .map(|(p, body)| format!("    {} => {}", pat_str(&p.0), e_str(body)))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("match {} {{\n{arms_s}\n}}", atom(scrutinee))
        }
        Expr::For { pat, iter, body } => {
            format!(
                "for {} in {} {{ {} }}",
                pat_str(&pat.0),
                atom(iter),
                e_str(body)
            )
        }

        Expr::Tuple(es) => {
            let inner = es.iter().map(e_str).collect::<Vec<_>>().join(", ");
            format!("({inner})")
        }
        Expr::List(es) => {
            let inner = es.iter().map(e_str).collect::<Vec<_>>().join(", ");
            format!("[{inner}]")
        }

        Expr::CircuitBlock(stmts) => block_str("circuit", stmts),
        Expr::RunBlock(stmts) => block_str("run", stmts),
        Expr::Compose(a, b) => format!("{} |> {}", atom(a), atom(b)),
        Expr::Par(body, count) => format!("par {{ {} }} * {}", e_str(body), atom(count)),
        Expr::Adjoint(x) => format!("adjoint({})", e_str(x)),
        Expr::Controlled(x) => format!("controlled({})", e_str(x)),
        Expr::GateApp { gate, qubits } => format!("{} @ {}", atom(gate), atom(qubits)),

        Expr::Borrow { bindings, body } => {
            let bs = bindings
                .iter()
                .map(|(n, t)| format!("{n}: {}", ty_str(&t.0)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("borrow {bs} in {}", block_str("", body).trim_start())
        }

        Expr::Return(x) => format!("return {}", atom(x)),
        Expr::Ascribe(x, t) => format!("{} : {}", atom(x), ty_str(&t.0)),

        // Post-desugaring node; never produced by the parser/generator.
        Expr::Bind { rhs, param, body } => {
            format!("bind({}, fn({param}) -> {})", e_str(rhs), e_str(body))
        }
    }
}
