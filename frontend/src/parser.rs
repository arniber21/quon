// Parser — see issue #7, SPEC.md §2–§5, §12, PRD (GitHub issue #1).
// A chumsky 0.13 parser generator over the spanned token stream produced by the lexer.
// Produces Vec<Sp<Decl>>.
//
// Statement separation uses *significant newlines* with operator continuation:
//   - juxtaposition application never crosses a newline (same-line only),
//   - `|>` composition bridges newlines (a `|>` at end-of-line or start-of-next-line
//     joins one expression),
//   - newlines inside ( ), < >, [ ], and comma lists are insignificant.

use crate::ast::*;
use crate::lexer::{SimpleSpan, Sp, Token};
use chumsky::input::ValueInput;
use chumsky::prelude::*;

/// Join two spans into one covering `a.start ..= b.end`.
fn s2(a: SimpleSpan, b: SimpleSpan) -> SimpleSpan {
    (a.start..b.end).into()
}

fn app(f: Sp<Expr>, x: Sp<Expr>) -> Sp<Expr> {
    let sp = s2(f.1, x.1);
    (Expr::App(Box::new(f), Box::new(x)), sp)
}

fn var(name: &str, sp: SimpleSpan) -> Sp<Expr> {
    (Expr::Var(name.to_string()), sp)
}

/// A trailing operation applied to an atom: call, index, or method.
enum Post {
    Call(Vec<Sp<Expr>>, SimpleSpan),
    Index(Sp<Expr>, SimpleSpan),
    Method(String, Vec<Sp<Expr>>, SimpleSpan),
}

fn apply_post(base: Sp<Expr>, post: Post) -> Sp<Expr> {
    match post {
        // `f()` is application to unit; `f(a, b)` curries.
        Post::Call(args, sp) => {
            if args.is_empty() {
                let span = s2(base.1, sp);
                (Expr::App(Box::new(base), Box::new((Expr::Unit, sp))), span)
            } else {
                args.into_iter().fold(base, app)
            }
        }
        // `q[i]` desugars to `index(q, i)`.
        Post::Index(idx, sp) => {
            let span = s2(base.1, sp);
            let base_sp = base.1;
            let indexer = app(var("index", base_sp), base);
            (Expr::App(Box::new(indexer), Box::new(idx)), span)
        }
        // `x.f(a, b)` desugars (UFCS) to `f(x, a, b)`.
        Post::Method(name, args, sp) => {
            let span = s2(base.1, sp);
            let base_sp = base.1;
            let head = app(var(&name, base_sp), base);
            let folded = args.into_iter().fold(head, app);
            (folded.0, span)
        }
    }
}

pub fn parse(tokens: &[Sp<Token>]) -> Result<Vec<Sp<Decl>>, Vec<Sp<String>>> {
    let eoi: SimpleSpan = tokens
        .last()
        .map(|(_, s)| (s.end..s.end).into())
        .unwrap_or_else(|| (0..0).into());
    let input = tokens.split_token_span(eoi);
    program().parse(input).into_result().map_err(|errs| {
        errs.into_iter()
            .map(|e| (e.to_string(), *e.span()))
            .collect()
    })
}

type PErr<'a> = extra::Err<Rich<'a, Token>>;

fn program<'a, I>() -> impl Parser<'a, I, Vec<Sp<Decl>>, PErr<'a>>
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan>,
{
    let ident = select! { Token::Ident(n) => n };

    // ── Natural-number expressions (type-level arithmetic) ────────────────────
    let nat = recursive(|nat| {
        let nls = just(Token::Newline).repeated();
        let atom = choice((
            select! { Token::Int(n) => n }.map_with(|n, e| (NatExpr::Lit(n as u64), e.span())),
            just(Token::Underscore).map_with(|_, e| (NatExpr::Hole, e.span())),
            ident.map_with(|n, e| (NatExpr::Var(n), e.span())),
            nat.clone()
                .padded_by(nls.clone())
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        ));

        let pow = atom
            .clone()
            .foldl(just(Token::Caret).ignore_then(atom).repeated(), |l, r| {
                let sp = s2(l.1, r.1);
                (NatExpr::Exp(Box::new(l), Box::new(r)), sp)
            });

        let mul = pow.clone().foldl(
            choice((just(Token::Star).to(true), just(Token::Slash).to(false)))
                .then(pow)
                .repeated(),
            |l, (is_mul, r)| {
                let sp = s2(l.1, r.1);
                let n = if is_mul {
                    NatExpr::Mul(Box::new(l), Box::new(r))
                } else {
                    NatExpr::Div(Box::new(l), Box::new(r))
                };
                (n, sp)
            },
        );

        mul.clone().foldl(
            choice((just(Token::Plus).to(true), just(Token::Minus).to(false)))
                .then(mul)
                .repeated(),
            |l, (is_add, r)| {
                let sp = s2(l.1, r.1);
                let n = if is_add {
                    NatExpr::Add(Box::new(l), Box::new(r))
                } else {
                    NatExpr::Sub(Box::new(l), Box::new(r))
                };
                (n, sp)
            },
        )
    })
    .boxed();

    // ── Patterns ──────────────────────────────────────────────────────────────
    let pat = recursive(|pat| {
        let nls = just(Token::Newline).repeated();
        let group = pat
            .clone()
            .separated_by(just(Token::Comma).padded_by(nls.clone()))
            .allow_trailing()
            .collect::<Vec<_>>()
            .padded_by(nls.clone())
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map_with(|mut ps: Vec<Sp<Pat>>, e| {
                if ps.len() == 1 {
                    ps.remove(0)
                } else {
                    (Pat::Tuple(ps), e.span())
                }
            });

        choice((
            just(Token::Underscore).map_with(|_, e| (Pat::Wildcard, e.span())),
            select! { Token::Int(n) => Pat::Lit(LitPat::Int(n)) }.map_with(|p, e| (p, e.span())),
            select! { Token::True => Pat::Lit(LitPat::Bool(true)) }.map_with(|p, e| (p, e.span())),
            select! { Token::False => Pat::Lit(LitPat::Bool(false)) }
                .map_with(|p, e| (p, e.span())),
            select! { Token::Ident(n) => Pat::Var(n) }.map_with(|p, e| (p, e.span())),
            group,
        ))
    })
    .boxed();

    // ── Types ─────────────────────────────────────────────────────────────────
    let ty = recursive(|ty| {
        let nls = just(Token::Newline).repeated();

        let class = select! {
            Token::Ident(n) if n == "Clifford" => CliffordClass::Clifford,
            Token::Ident(n) if n == "Universal" => CliffordClass::Universal,
        };

        // Depth position allows a `_` hole (parsed as NatExpr::Hole inside `nat`).
        let depth = nat.clone();

        let comma = just(Token::Comma).padded_by(nls.clone());

        let named = ident
            .then(
                nat.clone()
                    .separated_by(comma.clone())
                    .at_least(1)
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LAngle), just(Token::RAngle))
                    .or_not(),
            )
            .map_with(|(name, args), e| {
                let node = match (name.as_str(), &args) {
                    ("Qubit", None) => Type::Qubit,
                    ("Bit", None) => Type::Bit,
                    ("Bool", None) => Type::Bool,
                    ("Int", None) => Type::Int,
                    ("Float", None) => Type::Float,
                    ("Unit", None) => Type::Unit,
                    ("Nat", None) => Type::Nat,
                    _ => Type::Named {
                        name,
                        args: args.unwrap_or_default(),
                    },
                };
                (node, e.span())
            });

        let qreg = just(Token::Ident("QReg".into()))
            .ignore_then(
                nat.clone()
                    .delimited_by(just(Token::LAngle), just(Token::RAngle)),
            )
            .map_with(|n, e| (Type::QReg(n), e.span()));

        let qmonad = just(Token::Ident("Q".into()))
            .ignore_then(
                ty.clone()
                    .delimited_by(just(Token::LAngle), just(Token::RAngle)),
            )
            .map_with(|t, e| (Type::Q(Box::new(t)), e.span()));

        let list = just(Token::Ident("List".into()))
            .ignore_then(
                ty.clone()
                    .delimited_by(just(Token::LAngle), just(Token::RAngle)),
            )
            .map_with(|t, e| (Type::List(Box::new(t)), e.span()));

        let matrix = just(Token::Ident("Matrix".into()))
            .ignore_then(just(Token::LAngle))
            .ignore_then(nat.clone())
            .then_ignore(comma.clone())
            .then(nat.clone())
            .then_ignore(comma.clone())
            .then(ty.clone())
            .then_ignore(just(Token::RAngle))
            .map_with(|((n, m), t), e| (Type::Matrix(n, m, Box::new(t)), e.span()));

        let circuit = just(Token::Ident("Circuit".into()))
            .ignore_then(just(Token::LAngle))
            .ignore_then(nat.clone())
            .then_ignore(comma.clone())
            .then(nat.clone())
            .then_ignore(comma.clone())
            .then(depth.clone())
            .then_ignore(comma.clone())
            .then(class)
            .then_ignore(just(Token::RAngle))
            .map_with(|(((n, m), d), c), e| (Type::Circuit { n, m, d, c }, e.span()));

        let tuple = ty
            .clone()
            .separated_by(just(Token::Comma).padded_by(nls.clone()))
            .allow_trailing()
            .collect::<Vec<_>>()
            .padded_by(nls.clone())
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map_with(|mut ts: Vec<Sp<Type>>, e| {
                if ts.is_empty() {
                    (Type::Unit, e.span())
                } else if ts.len() == 1 {
                    ts.remove(0)
                } else {
                    (Type::Tuple(ts), e.span())
                }
            });

        // Order matters: specific keyword-like constructors before the generic `named`.
        let atom = choice((qreg, qmonad, list, matrix, circuit, tuple, named)).boxed();

        // Function arrows are right-associative and the loosest type operator.
        atom.clone()
            .then(
                choice((
                    just(Token::Arrow).to(true),
                    just(Token::LinearArrow).to(false),
                ))
                .then(ty.clone())
                .or_not(),
            )
            .map(|(lhs, rest)| match rest {
                None => lhs,
                Some((is_fn, rhs)) => {
                    let sp = s2(lhs.1, rhs.1);
                    let node = if is_fn {
                        Type::Fn(Box::new(lhs), Box::new(rhs))
                    } else {
                        Type::Linear(Box::new(lhs), Box::new(rhs))
                    };
                    (node, sp)
                }
            })
    })
    .boxed();

    // ── Expressions ───────────────────────────────────────────────────────────
    let expr = recursive(|expr| {
        let nls = just(Token::Newline).repeated();
        let comma = just(Token::Comma).padded_by(nls.clone());

        let args = expr
            .clone()
            .separated_by(comma.clone())
            .allow_trailing()
            .collect::<Vec<_>>()
            .padded_by(nls.clone());

        // Block of statements (run/circuit/borrow bodies).
        let stmt = {
            let bind = pat
                .clone()
                .then_ignore(just(Token::Bind))
                .then(expr.clone())
                .map(|(pat, rhs)| Stmt::Bind { pat, rhs });

            // `let p = e` is a statement; `let p = e in body` is a Let *expression* statement.
            let let_stmt = just(Token::Let)
                .ignore_then(pat.clone())
                .then_ignore(just(Token::Eq))
                .then(expr.clone())
                .then(just(Token::In).ignore_then(expr.clone()).or_not())
                .map(|((pat, rhs), body)| match body {
                    None => Stmt::Let { pat, rhs },
                    Some(body) => {
                        let sp = s2(rhs.1, body.1);
                        Stmt::Expr((
                            Expr::Let {
                                pat,
                                rhs: Box::new(rhs),
                                body: Box::new(body),
                            },
                            sp,
                        ))
                    }
                });

            choice((bind, let_stmt, expr.clone().map(Stmt::Expr))).map_with(|s, e| (s, e.span()))
        };

        let block_body = stmt
            .separated_by(just(Token::Newline).repeated().at_least(1))
            .allow_leading()
            .allow_trailing()
            .collect::<Vec<_>>();

        // ---- atoms ----
        let int = select! { Token::Int(n) => Expr::Int(n) }.map_with(|x, e| (x, e.span()));
        let float = select! { Token::Float(f) => Expr::Float(f) }.map_with(|x, e| (x, e.span()));
        let boolean = select! {
            Token::True => Expr::Bool(true),
            Token::False => Expr::Bool(false),
        }
        .map_with(|x, e| (x, e.span()));
        let variable = ident.map_with(|n, e| (Expr::Var(n), e.span()));

        let paren = expr
            .clone()
            .separated_by(comma.clone())
            .allow_trailing()
            .collect::<Vec<_>>()
            .padded_by(nls.clone())
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map_with(|mut es: Vec<Sp<Expr>>, e| {
                if es.is_empty() {
                    (Expr::Unit, e.span())
                } else if es.len() == 1 {
                    es.remove(0)
                } else {
                    (Expr::Tuple(es), e.span())
                }
            });

        let list = expr
            .clone()
            .separated_by(comma.clone())
            .allow_trailing()
            .collect::<Vec<_>>()
            .padded_by(nls.clone())
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .map_with(|es, e| (Expr::List(es), e.span()));

        let circuit_block = just(Token::Circuit)
            .ignore_then(
                block_body
                    .clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map_with(|stmts, e| (Expr::CircuitBlock(stmts), e.span()));

        let run_block = just(Token::Run)
            .ignore_then(
                block_body
                    .clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map_with(|stmts, e| (Expr::RunBlock(stmts), e.span()));

        let borrow_binding = ident.then_ignore(just(Token::Colon)).then(ty.clone());
        let borrow_block = just(Token::Borrow)
            .ignore_then(
                borrow_binding
                    .separated_by(just(Token::Comma).padded_by(nls.clone()))
                    .at_least(1)
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::In))
            .then(
                block_body
                    .clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map_with(|(bindings, body), e| (Expr::Borrow { bindings, body }, e.span()));

        let brace_expr = expr
            .clone()
            .padded_by(nls.clone())
            .delimited_by(just(Token::LBrace), just(Token::RBrace));

        let for_expr = just(Token::For)
            .ignore_then(pat.clone())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(brace_expr.clone())
            .map_with(|((p, iter), body), e| {
                (
                    Expr::For {
                        pat: p,
                        iter: Box::new(iter),
                        body: Box::new(body),
                    },
                    e.span(),
                )
            });

        let arm = pat
            .clone()
            .then_ignore(just(Token::FatArrow))
            .then(expr.clone());
        let arm_sep = choice((
            just(Token::Comma).ignored(),
            just(Token::Bar).ignored(),
            just(Token::Newline).ignored(),
        ))
        .repeated()
        .at_least(1);
        let match_expr = just(Token::Match)
            .ignore_then(expr.clone())
            .then(
                arm.separated_by(arm_sep)
                    .allow_leading()
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map_with(|(scrutinee, arms), e| {
                (
                    Expr::Match {
                        scrutinee: Box::new(scrutinee),
                        arms,
                    },
                    e.span(),
                )
            });

        let adjoint = just(Token::Adjoint)
            .ignore_then(
                expr.clone()
                    .padded_by(nls.clone())
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map_with(|inner, e| (Expr::Adjoint(Box::new(inner)), e.span()));

        let controlled = just(Token::Controlled)
            .ignore_then(
                expr.clone()
                    .padded_by(nls.clone())
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map_with(|inner, e| (Expr::Controlled(Box::new(inner)), e.span()));

        let atom = choice((
            float,
            int,
            boolean,
            variable,
            paren,
            list,
            circuit_block,
            run_block,
            borrow_block,
            for_expr,
            match_expr,
            adjoint,
            controlled,
        ))
        .boxed();

        // ---- postfix: call / index / method ----
        let call_op = args
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map_with(|a, e| Post::Call(a, e.span()));
        let index_op = expr
            .clone()
            .padded_by(nls.clone())
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .map_with(|i, e| Post::Index(i, e.span()));
        let method_op = just(Token::Dot)
            .ignore_then(ident)
            .then(
                args.clone()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map_with(|(name, a), e| Post::Method(name, a, e.span()));

        let postfix = atom.foldl(
            choice((call_op, index_op, method_op)).repeated(),
            apply_post,
        );

        // ---- par { body } * count (uses postfix-level count) ----
        let par_expr = just(Token::Par)
            .ignore_then(brace_expr.clone())
            .then_ignore(just(Token::Star))
            .then(postfix.clone())
            .map_with(|(body, count), e| (Expr::Par(Box::new(body), Box::new(count)), e.span()));

        let postfix = choice((par_expr, postfix)).boxed();

        // ---- juxtaposition application (same line) ----
        let application = postfix
            .clone()
            .foldl(postfix.clone().repeated(), app)
            .boxed();

        // ---- gate application `@` (same line, left-assoc) ----
        let gateapp = application
            .clone()
            .foldl(
                just(Token::At).ignore_then(application).repeated(),
                |gate, qubits| {
                    let sp = s2(gate.1, qubits.1);
                    (
                        Expr::GateApp {
                            gate: Box::new(gate),
                            qubits: Box::new(qubits),
                        },
                        sp,
                    )
                },
            )
            .boxed();

        // ---- unary minus ----
        let neg = just(Token::Minus)
            .map_with(|_, e| e.span())
            .repeated()
            .foldr(gateapp, |msp, inner| {
                let sp = s2(msp, inner.1);
                (Expr::Neg(Box::new(inner)), sp)
            })
            .boxed();

        // ---- exponent `^` (right-assoc) ----
        let pow = neg
            .clone()
            .foldl(just(Token::Caret).ignore_then(neg).repeated(), |l, r| {
                let sp = s2(l.1, r.1);
                (
                    Expr::BinOp {
                        op: BinOp::Pow,
                        lhs: Box::new(l),
                        rhs: Box::new(r),
                    },
                    sp,
                )
            })
            .boxed();

        // ---- multiplicative `* /` (left-assoc) ----
        let product = pow
            .clone()
            .foldl(
                choice((
                    just(Token::Star).to(BinOp::Mul),
                    just(Token::Slash).to(BinOp::Div),
                ))
                .then(pow)
                .repeated(),
                |l, (op, r)| {
                    let sp = s2(l.1, r.1);
                    (
                        Expr::BinOp {
                            op,
                            lhs: Box::new(l),
                            rhs: Box::new(r),
                        },
                        sp,
                    )
                },
            )
            .boxed();

        // ---- additive `+ -` (left-assoc) ----
        let sum = product
            .clone()
            .foldl(
                choice((
                    just(Token::Plus).to(BinOp::Add),
                    just(Token::Minus).to(BinOp::Sub),
                ))
                .then(product)
                .repeated(),
                |l, (op, r)| {
                    let sp = s2(l.1, r.1);
                    (
                        Expr::BinOp {
                            op,
                            lhs: Box::new(l),
                            rhs: Box::new(r),
                        },
                        sp,
                    )
                },
            )
            .boxed();

        // ---- backtick infix `` a `f` b `` (left-assoc) → f(a, b) ----
        let backtick = sum
            .clone()
            .foldl(
                just(Token::Backtick)
                    .ignore_then(ident)
                    .then_ignore(just(Token::Backtick))
                    .then(sum)
                    .repeated(),
                |l, (name, r): (String, Sp<Expr>)| {
                    let lsp = l.1;
                    app(app(var(&name, lsp), l), r)
                },
            )
            .boxed();

        // ---- `|>` composition (left-assoc, bridges newlines, looser than `@`) ----
        let compose = backtick
            .clone()
            .foldl(
                just(Token::Newline)
                    .repeated()
                    .ignore_then(just(Token::Pipe))
                    .ignore_then(just(Token::Newline).repeated())
                    .ignore_then(backtick)
                    .repeated(),
                |l, r| {
                    let sp = s2(l.1, r.1);
                    (Expr::Compose(Box::new(l), Box::new(r)), sp)
                },
            )
            .boxed();

        // ---- optional ascription `e : ty` ----
        let ascribed = compose
            .then(just(Token::Colon).ignore_then(ty.clone()).or_not())
            .map(|(e, t)| match t {
                None => e,
                Some(t) => {
                    let sp = s2(e.1, t.1);
                    (Expr::Ascribe(Box::new(e), t), sp)
                }
            });

        // ---- right-extending prefix forms ----
        let lam_param = pat
            .clone()
            .then(just(Token::Colon).ignore_then(ty.clone()).or_not());
        let lambda = just(Token::Fn)
            .ignore_then(
                lam_param
                    .separated_by(comma.clone())
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .padded_by(nls.clone())
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then_ignore(just(Token::Arrow))
            .then_ignore(nls.clone())
            .then(expr.clone())
            .map_with(|(params, body), e| {
                (
                    Expr::Lam {
                        params,
                        body: Box::new(body),
                    },
                    e.span(),
                )
            });

        let let_in = just(Token::Let)
            .ignore_then(pat.clone())
            .then_ignore(just(Token::Eq))
            .then(expr.clone())
            .then_ignore(nls.clone())
            .then_ignore(just(Token::In))
            .then_ignore(nls.clone())
            .then(expr.clone())
            .map_with(|((pat, rhs), body), e| {
                (
                    Expr::Let {
                        pat,
                        rhs: Box::new(rhs),
                        body: Box::new(body),
                    },
                    e.span(),
                )
            });

        let if_expr = just(Token::If)
            .ignore_then(expr.clone())
            .then_ignore(just(Token::Then))
            .then(expr.clone())
            .then_ignore(just(Token::Else))
            .then(expr.clone())
            .map_with(|((cond, then), else_), e| {
                (
                    Expr::If {
                        cond: Box::new(cond),
                        then: Box::new(then),
                        else_: Box::new(else_),
                    },
                    e.span(),
                )
            });

        let return_expr = just(Token::Return)
            .ignore_then(expr.clone())
            .map_with(|inner, e| (Expr::Return(Box::new(inner)), e.span()));

        choice((lambda, let_in, if_expr, return_expr, ascribed)).boxed()
    });

    // ── Declarations ──────────────────────────────────────────────────────────
    let nls = just(Token::Newline).repeated();

    let fn_param = ident.then_ignore(just(Token::Colon)).then(ty.clone());
    let fn_decl = just(Token::Fn)
        .ignore_then(ident)
        .then(
            fn_param
                .separated_by(just(Token::Comma).padded_by(nls.clone()))
                .allow_trailing()
                .collect::<Vec<_>>()
                .padded_by(nls.clone())
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then_ignore(nls.clone())
        .then_ignore(just(Token::Colon))
        .then(ty.clone())
        .then_ignore(nls.clone())
        .then_ignore(just(Token::Eq))
        .then_ignore(nls.clone())
        .then(expr.clone())
        .map_with(|(((name, params), ret), body), e| {
            (
                Decl::Fn {
                    name,
                    params,
                    ret,
                    body,
                },
                e.span(),
            )
        });

    let type_alias = just(Token::Type)
        .ignore_then(ident)
        .then(
            ident
                .separated_by(just(Token::Comma).padded_by(nls.clone()))
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LAngle), just(Token::RAngle))
                .or_not(),
        )
        .then_ignore(nls.clone())
        .then_ignore(just(Token::Eq))
        .then_ignore(nls.clone())
        .then(ty.clone())
        .map_with(|((name, params), ty), e| {
            (
                Decl::TypeAlias {
                    name,
                    params: params.unwrap_or_default(),
                    ty,
                },
                e.span(),
            )
        });

    let decl = choice((fn_decl, type_alias));

    decl.separated_by(just(Token::Newline).repeated().at_least(1))
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .then_ignore(just(Token::Newline).repeated())
        .then_ignore(end())
}
