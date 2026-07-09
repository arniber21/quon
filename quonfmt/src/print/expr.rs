use frontend::ast::{BinOp, Expr, Stmt, Type};
use frontend::lexer::Sp;

use crate::doc::Doc;
use crate::print::{BlockKind, Context, binop_str, pat, render_float, stmt, ty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prec {
    Top = 0,
    IfLet = 1,
    Ascribe = 2,
    Compose = 3,
    Add = 4,
    Mul = 5,
    Pow = 6,
    Neg = 7,
    GateApp = 8,
    App = 9,
    Atom = 10,
}

pub fn print_expr(e: &Sp<Expr>, ctx: &mut Context<'_>, min_prec: Prec) -> Doc {
    let doc = match &e.0 {
        Expr::Int(n) => Doc::text(n.to_string()),
        Expr::Float(f) => Doc::text(render_float(*f)),
        Expr::Bool(b) => Doc::text(b.to_string()),
        Expr::Unit => Doc::text("()"),
        Expr::Var(n) => Doc::text(n.clone()),

        Expr::App(_, _) => print_app(e, ctx),

        Expr::BinOp { op, lhs, rhs } => {
            let self_prec = match op {
                BinOp::Add | BinOp::Sub => Prec::Add,
                BinOp::Mul | BinOp::Div => Prec::Mul,
                BinOp::Pow => Prec::Pow,
            };
            Doc::concat([
                print_expr(lhs, ctx, self_prec),
                Doc::text(format!(" {} ", binop_str(*op))),
                print_expr(rhs, ctx, self_prec),
            ])
        }
        Expr::Neg(x) => Doc::concat([Doc::text("- "), print_expr(x, ctx, Prec::Neg)]),

        Expr::Lam { params, body } => {
            let ps: Vec<Doc> = params
                .iter()
                .map(|(p, t)| match t {
                    Some(t) => Doc::concat([
                        pat::print_pat(p, ctx),
                        Doc::text(": "),
                        ty::print_ty(t, ctx),
                    ]),
                    None => pat::print_pat(p, ctx),
                })
                .collect();
            Doc::group(Doc::concat([
                Doc::text("fn("),
                comma_sep(ps),
                Doc::text(") -> "),
                print_expr(body, ctx, Prec::Top),
            ]))
        }
        Expr::Let { pat, rhs, body } => Doc::group(Doc::concat([
            Doc::text("let "),
            pat::print_pat(pat, ctx),
            Doc::text(" = "),
            print_expr(rhs, ctx, Prec::IfLet),
            Doc::text(" in "),
            print_expr(body, ctx, Prec::IfLet),
        ])),

        Expr::If { cond, then, else_ } => Doc::group(Doc::concat([
            Doc::text("if "),
            print_expr(cond, ctx, Prec::IfLet),
            Doc::text(" then "),
            print_expr(then, ctx, Prec::IfLet),
            Doc::text(" else "),
            print_expr(else_, ctx, Prec::IfLet),
        ])),

        Expr::Match { scrutinee, arms } => {
            let arm_docs: Vec<Doc> = arms
                .iter()
                .map(|(p, body)| {
                    Doc::concat([
                        pat::print_pat(p, ctx),
                        Doc::text(" => "),
                        print_expr(body, ctx, Prec::Top),
                    ])
                })
                .collect();
            Doc::concat([
                Doc::text("match "),
                print_expr(scrutinee, ctx, Prec::Atom),
                Doc::text(" {\n"),
                Doc::nest(
                    1,
                    Doc::concat(
                        arm_docs
                            .into_iter()
                            .flat_map(|a| [a, Doc::text(",\n")])
                            .take(if arms.is_empty() {
                                0
                            } else {
                                arms.len() * 2 - 1
                            }),
                    ),
                ),
                Doc::text("\n}"),
            ])
        }

        Expr::For { pat, iter, body } => print_for(pat, iter, body, ctx),

        Expr::Tuple(es) => Doc::concat([
            Doc::text("("),
            comma_sep(es.iter().map(|e| print_expr(e, ctx, Prec::Top)).collect()),
            Doc::text(")"),
        ]),
        Expr::List(es) => Doc::concat([
            Doc::text("["),
            comma_sep(es.iter().map(|e| print_expr(e, ctx, Prec::Top)).collect()),
            Doc::text("]"),
        ]),

        Expr::CircuitBlock(stmts) => {
            let mut inner = ctx.nested().with_block(BlockKind::Circuit);
            stmt::print_stmt_block("circuit", stmts, &mut inner)
        }
        Expr::RunBlock(stmts) => {
            let mut inner = ctx.nested().with_block(BlockKind::Run);
            stmt::print_stmt_block("run", stmts, &mut inner)
        }
        Expr::Compose(a, b) => print_compose_chain(a, b, ctx),

        Expr::Par(body, count) => Doc::group(Doc::concat([
            Doc::text("par { "),
            expr_in_circuit_context(body, ctx),
            Doc::text(" } * "),
            print_expr(count, ctx, Prec::Mul),
        ])),

        Expr::Adjoint(x) => Doc::concat([
            Doc::text("adjoint("),
            print_expr(x, ctx, Prec::Top),
            Doc::text(")"),
        ]),
        Expr::Controlled(x) => Doc::concat([
            Doc::text("controlled("),
            print_expr(x, ctx, Prec::Top),
            Doc::text(")"),
        ]),
        Expr::GateApp { gate, qubits } => Doc::concat([
            print_expr(gate, ctx, Prec::GateApp),
            Doc::text(" @"),
            print_expr_qubit_target(qubits, ctx),
        ]),

        Expr::Borrow { bindings, body } => print_borrow(bindings, body, ctx),

        Expr::Return(x) => Doc::concat([Doc::text("return "), print_expr(x, ctx, Prec::Top)]),
        Expr::Ascribe(x, t) => Doc::concat([
            print_expr(x, ctx, Prec::Ascribe),
            Doc::text(": "),
            ty::print_ty(t, ctx),
        ]),

        Expr::Bind { rhs, param, body } => Doc::concat([
            Doc::text("bind("),
            print_expr(rhs, ctx, Prec::Top),
            Doc::text(", fn("),
            Doc::text(param.0.clone()),
            Doc::text(") -> "),
            print_expr(body, ctx, Prec::Top),
            Doc::text(")"),
        ]),
    };
    maybe_paren(doc, expr_prec(&e.0), min_prec)
}

fn print_expr_qubit_target(e: &Sp<Expr>, ctx: &mut Context<'_>) -> Doc {
    match &e.0 {
        Expr::Int(_) | Expr::Var(_) | Expr::Tuple(_) | Expr::List(_) | Expr::Unit => {
            print_expr(e, ctx, Prec::Atom)
        }
        _ => Doc::concat([Doc::text(" "), print_expr(e, ctx, Prec::Atom)]),
    }
}

fn expr_in_circuit_context(e: &Sp<Expr>, ctx: &mut Context<'_>) -> Doc {
    if ctx.block_kind == BlockKind::Circuit && stmt::circuit_stmt_needs_parens(&e.0) {
        Doc::concat([
            Doc::text("("),
            print_expr(e, ctx, Prec::Top),
            Doc::text(")"),
        ])
    } else {
        print_expr(e, ctx, Prec::Top)
    }
}

fn print_for(
    pat: &Sp<frontend::ast::Pat>,
    iter: &Sp<Expr>,
    body: &Sp<Expr>,
    ctx: &mut Context<'_>,
) -> Doc {
    let body_doc = expr_in_circuit_context(body, ctx);
    Doc::group(Doc::concat([
        Doc::text("for "),
        pat::print_pat(pat, ctx),
        Doc::text(" in "),
        print_expr(iter, ctx, Prec::Atom),
        Doc::text(" { "),
        body_doc,
        Doc::text(" }"),
    ]))
}

fn print_borrow(
    bindings: &[(Sp<frontend::ast::Name>, Sp<Type>)],
    body: &[Sp<Stmt>],
    ctx: &mut Context<'_>,
) -> Doc {
    let bind_docs: Vec<Doc> = bindings
        .iter()
        .map(|(n, t)| {
            Doc::concat([
                Doc::text(n.0.clone()),
                Doc::text(": "),
                ty::print_ty(t, ctx),
            ])
        })
        .collect();
    let mut inner = ctx.nested().with_block(BlockKind::Borrow);
    Doc::concat([
        Doc::text("borrow "),
        comma_sep(bind_docs),
        Doc::text(" in "),
        stmt::print_stmt_block("", body, &mut inner),
    ])
}

fn print_compose_chain(a: &Sp<Expr>, b: &Sp<Expr>, ctx: &mut Context<'_>) -> Doc {
    let mut parts = vec![a.clone()];
    let mut cur = b.clone();
    loop {
        if let Expr::Compose(l, r) = &cur.0 {
            parts.push(l.as_ref().clone());
            cur = r.as_ref().clone();
        } else {
            parts.push(cur);
            break;
        }
    }

    let docs: Vec<Doc> = parts
        .iter()
        .map(|p| print_expr(p, ctx, Prec::Compose))
        .collect();

    let mut result = docs[0].clone();
    for d in docs.iter().skip(1) {
        result = Doc::concat([
            result,
            Doc::group(Doc::concat([
                Doc::soft_break(),
                Doc::text("|> "),
                d.clone(),
            ])),
        ]);
    }
    Doc::group(result)
}

fn uncurry_app(e: &Sp<Expr>) -> (Sp<Expr>, Vec<Sp<Expr>>) {
    let mut args = Vec::new();
    let mut cur = e.clone();
    while let Expr::App(f, x) = &cur.0 {
        args.push(x.as_ref().clone());
        cur = f.as_ref().clone();
    }
    args.reverse();
    (cur, args)
}

fn print_app(e: &Sp<Expr>, ctx: &mut Context<'_>) -> Doc {
    let (func, args) = uncurry_app(e);

    if args.is_empty() {
        return print_expr(&func, ctx, Prec::App);
    }

    if args.len() == 1 {
        let arg = &args[0];
        if is_unit(arg) {
            return Doc::concat([print_expr(&func, ctx, Prec::App), Doc::text("()")]);
        }
        if is_juxta_safe(&func) && is_juxta_atom(arg) {
            return Doc::concat([
                print_expr(&func, ctx, Prec::App),
                Doc::text(" "),
                print_expr(arg, ctx, Prec::Atom),
            ]);
        }
    }

    let arg_docs: Vec<Doc> = args.iter().map(|a| print_expr(a, ctx, Prec::Top)).collect();
    Doc::concat([
        print_expr(&func, ctx, Prec::App),
        Doc::text("("),
        comma_sep(arg_docs),
        Doc::text(")"),
    ])
}

fn is_unit(e: &Sp<Expr>) -> bool {
    matches!(e.0, Expr::Unit)
}

fn is_juxta_atom(e: &Sp<Expr>) -> bool {
    matches!(
        &e.0,
        Expr::Int(_)
            | Expr::Float(_)
            | Expr::Bool(_)
            | Expr::Unit
            | Expr::Var(_)
            | Expr::Tuple(_)
            | Expr::List(_)
            | Expr::Adjoint(_)
            | Expr::Controlled(_)
    ) || matches!(&e.0, Expr::GateApp { .. })
}

fn is_juxta_safe(e: &Sp<Expr>) -> bool {
    match &e.0 {
        Expr::Var(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Bool(_)
        | Expr::Unit
        | Expr::Adjoint(_)
        | Expr::Controlled(_) => true,
        Expr::App(f, x) if is_juxta_safe(f) && is_juxta_atom(x) => true,
        _ => false,
    }
}

fn comma_sep(items: Vec<Doc>) -> Doc {
    let n = items.len();
    if n == 0 {
        Doc::nil()
    } else {
        Doc::concat(
            items
                .into_iter()
                .flat_map(|d| [d, Doc::text(", ")])
                .take(n * 2 - 1),
        )
    }
}

fn maybe_paren(doc: Doc, self_prec: Prec, min_prec: Prec) -> Doc {
    if self_prec < min_prec {
        Doc::concat([Doc::text("("), doc, Doc::text(")")])
    } else {
        doc
    }
}

fn expr_prec(e: &Expr) -> Prec {
    match e {
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
        | Expr::Controlled(_)
        | Expr::Lam { .. }
        | Expr::Return(_) => Prec::Atom,
        Expr::App(_, _) => Prec::App,
        Expr::GateApp { .. } => Prec::GateApp,
        Expr::Neg(_) => Prec::Neg,
        Expr::BinOp { op: BinOp::Pow, .. } => Prec::Pow,
        Expr::BinOp {
            op: BinOp::Mul | BinOp::Div,
            ..
        }
        | Expr::Par(_, _) => Prec::Mul,
        Expr::BinOp {
            op: BinOp::Add | BinOp::Sub,
            ..
        } => Prec::Add,
        Expr::Compose(_, _) => Prec::Compose,
        Expr::Ascribe(_, _) => Prec::Ascribe,
        Expr::Let { .. } | Expr::If { .. } | Expr::Bind { .. } => Prec::IfLet,
    }
}
