use frontend::ast::Type;
use frontend::lexer::Sp;

use crate::doc::Doc;
use crate::print::{Context, class_str, nat};

pub fn print_ty(t: &Sp<Type>, ctx: &mut Context<'_>) -> Doc {
    match &t.0 {
        Type::Qubit => Doc::text("Qubit"),
        Type::Bit => Doc::text("Bit"),
        Type::Bool => Doc::text("Bool"),
        Type::Int => Doc::text("Int"),
        Type::Float => Doc::text("Float"),
        Type::Unit => Doc::text("Unit"),
        Type::Nat => Doc::text("Nat"),
        Type::QReg(n) => Doc::concat([
            Doc::text("QReg<"),
            nat::print_nat(n, ctx, nat::Prec::Top),
            Doc::text(">"),
        ]),
        Type::Q(t) => Doc::concat([Doc::text("Q<"), print_ty(t, ctx), Doc::text(">")]),
        Type::List(t) => Doc::concat([Doc::text("List<"), print_ty(t, ctx), Doc::text(">")]),
        Type::Matrix(n, m, t) => Doc::group(Doc::concat([
            Doc::text("Matrix<"),
            nat::print_nat(n, ctx, nat::Prec::Top),
            Doc::text(", "),
            nat::print_nat(m, ctx, nat::Prec::Top),
            Doc::text(", "),
            print_ty(t, ctx),
            Doc::text(">"),
        ])),
        Type::Circuit { n, m, d, c } => Doc::group(Doc::concat([
            Doc::text("Circuit<"),
            nat::print_nat(n, ctx, nat::Prec::Top),
            Doc::text(", "),
            nat::print_nat(m, ctx, nat::Prec::Top),
            Doc::text(", "),
            nat::print_nat(d, ctx, nat::Prec::Top),
            Doc::text(", "),
            Doc::text(class_str(c)),
            Doc::text(">"),
        ])),
        Type::Fn(a, b) => Doc::group(Doc::concat([
            Doc::text("("),
            print_ty(a, ctx),
            Doc::text(") -> ("),
            print_ty(b, ctx),
            Doc::text(")"),
        ])),
        Type::Linear(a, b) => Doc::group(Doc::concat([
            Doc::text("("),
            print_ty(a, ctx),
            Doc::text(") -o ("),
            print_ty(b, ctx),
            Doc::text(")"),
        ])),
        Type::Tuple(ts) => {
            let inner: Vec<Doc> = ts.iter().map(|t| print_ty(t, ctx)).collect();
            Doc::concat([
                Doc::text("("),
                Doc::concat(
                    inner
                        .into_iter()
                        .flat_map(|d| [d, Doc::text(", ")])
                        .take(if ts.is_empty() { 0 } else { ts.len() * 2 - 1 }),
                ),
                Doc::text(")"),
            ])
        }
        Type::QecBlock { family, distance } => Doc::group(Doc::concat([
            Doc::text("QecBlock<"),
            print_ty(family, ctx),
            Doc::text(", "),
            nat::print_nat(distance, ctx, nat::Prec::Top),
            Doc::text(">"),
        ])),
        Type::Var(n) => Doc::text(n.clone()),
        Type::Named { name, args } => {
            if args.is_empty() {
                Doc::text(name.clone())
            } else {
                let a: Vec<Doc> = args
                    .iter()
                    .map(|n| nat::print_nat(n, ctx, nat::Prec::Top))
                    .collect();
                Doc::concat([
                    Doc::text(format!("{name}<")),
                    Doc::concat(a.into_iter().flat_map(|d| [d, Doc::text(", ")]).take(
                        if args.is_empty() {
                            0
                        } else {
                            args.len() * 2 - 1
                        },
                    )),
                    Doc::text(">"),
                ])
            }
        }
    }
}
