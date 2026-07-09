use frontend::ast::NatExpr;
use frontend::lexer::Sp;

use crate::doc::Doc;
use crate::print::{Context, binop_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prec {
    Top = 0,
    Add = 1,
    Mul = 2,
    Pow = 3,
    Atom = 4,
}

pub fn print_nat(n: &Sp<NatExpr>, ctx: &mut Context<'_>, min_prec: Prec) -> Doc {
    let (self_prec, doc) = match &n.0 {
        NatExpr::Lit(v) => (Prec::Atom, Doc::text(v.to_string())),
        NatExpr::Var(name) => (Prec::Atom, Doc::text(name.clone())),
        NatExpr::Hole => (Prec::Atom, Doc::text("_")),
        NatExpr::Add(a, b) | NatExpr::Sub(a, b) => {
            (Prec::Add, binop_doc(a, b, nat_binop(&n.0), Prec::Add, ctx))
        }
        NatExpr::Mul(a, b) | NatExpr::Div(a, b) => {
            (Prec::Mul, binop_doc(a, b, nat_binop(&n.0), Prec::Mul, ctx))
        }
        NatExpr::Exp(a, b) => (
            Prec::Pow,
            Doc::concat([
                print_nat(a, ctx, Prec::Pow),
                Doc::text(format!(" {} ", binop_str(nat_binop(&n.0)))),
                print_nat(b, ctx, Prec::Pow),
            ]),
        ),
    };
    if self_prec < min_prec {
        Doc::concat([Doc::text("("), doc, Doc::text(")")])
    } else {
        doc
    }
}

fn nat_binop(n: &NatExpr) -> frontend::ast::BinOp {
    match n {
        NatExpr::Add(_, _) => frontend::ast::BinOp::Add,
        NatExpr::Sub(_, _) => frontend::ast::BinOp::Sub,
        NatExpr::Mul(_, _) => frontend::ast::BinOp::Mul,
        NatExpr::Div(_, _) => frontend::ast::BinOp::Div,
        NatExpr::Exp(_, _) => frontend::ast::BinOp::Pow,
        _ => frontend::ast::BinOp::Add,
    }
}

fn binop_doc(
    a: &Sp<NatExpr>,
    b: &Sp<NatExpr>,
    op: frontend::ast::BinOp,
    self_prec: Prec,
    ctx: &mut Context<'_>,
) -> Doc {
    Doc::concat([
        print_nat(a, ctx, self_prec),
        Doc::text(format!(" {} ", binop_str(op))),
        print_nat(b, ctx, rhs_min_prec(self_prec)),
    ])
}

fn rhs_min_prec(self_prec: Prec) -> Prec {
    match self_prec {
        Prec::Add => Prec::Mul,
        Prec::Mul => Prec::Pow,
        Prec::Pow => Prec::Atom,
        _ => Prec::Atom,
    }
}
