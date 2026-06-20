// Generative roundtrip property test — the centerpiece of the harness.
//
// Generate a random (but syntactically valid) AST, pretty-print it, lex + parse the result,
// and assert the reparsed tree equals the original (spans ignored). This is the precedence /
// associativity / desugaring oracle: any printer/parser asymmetry produces a shrinkable
// counterexample.

mod support;

use frontend::ast::*;
use frontend::lexer::{lex, SimpleSpan, Sp};
use frontend::parser::parse;
use frontend::pretty::pretty;
use proptest::prelude::*;
use support::strip_decls;

fn sp<T>(t: T) -> Sp<T> {
    (t, SimpleSpan::from(0..0))
}

const KEYWORDS: &[&str] = &[
    "fn",
    "type",
    "let",
    "in",
    "return",
    "match",
    "circuit",
    "run",
    "borrow",
    "for",
    "if",
    "then",
    "else",
    "true",
    "false",
    "adjoint",
    "controlled",
    "par",
];
const TYPE_NAMES: &[&str] = &[
    "Qubit",
    "Bit",
    "Bool",
    "Int",
    "Float",
    "Unit",
    "Nat",
    "QReg",
    "Q",
    "List",
    "Matrix",
    "Circuit",
    "Clifford",
    "Universal",
];

fn ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,4}".prop_map(|s| {
        if KEYWORDS.contains(&s.as_str()) {
            format!("{s}_")
        } else {
            s
        }
    })
}

fn type_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z0-9]{0,4}".prop_map(|s| {
        if TYPE_NAMES.contains(&s.as_str()) {
            format!("{s}x")
        } else {
            s
        }
    })
}

fn float_lit() -> impl Strategy<Value = f64> {
    // Clean two-decimal values that always print with a `.` and round-trip exactly.
    (0u32..10_000).prop_map(|n| n as f64 / 100.0)
}

fn nat_strategy() -> BoxedStrategy<Sp<NatExpr>> {
    let leaf = prop_oneof![
        4 => (0u64..100).prop_map(NatExpr::Lit),
        4 => ident().prop_map(NatExpr::Var),
        1 => Just(NatExpr::Hole),
    ]
    .prop_map(sp);
    leaf.prop_recursive(3, 12, 2, |inner| {
        let pair = (inner.clone(), inner);
        prop_oneof![
            pair.clone()
                .prop_map(|(a, b)| sp(NatExpr::Add(Box::new(a), Box::new(b)))),
            pair.clone()
                .prop_map(|(a, b)| sp(NatExpr::Sub(Box::new(a), Box::new(b)))),
            pair.clone()
                .prop_map(|(a, b)| sp(NatExpr::Mul(Box::new(a), Box::new(b)))),
            pair.clone()
                .prop_map(|(a, b)| sp(NatExpr::Div(Box::new(a), Box::new(b)))),
            pair.prop_map(|(a, b)| sp(NatExpr::Exp(Box::new(a), Box::new(b)))),
        ]
    })
    .boxed()
}

fn class_strategy() -> impl Strategy<Value = CliffordClass> {
    prop_oneof![
        Just(CliffordClass::Clifford),
        Just(CliffordClass::Universal),
    ]
}

fn ty_strategy() -> BoxedStrategy<Sp<Type>> {
    let leaf = prop_oneof![
        Just(Type::Qubit),
        Just(Type::Bit),
        Just(Type::Bool),
        Just(Type::Int),
        Just(Type::Float),
        Just(Type::Unit),
        Just(Type::Nat),
        (type_name(), prop::collection::vec(nat_strategy(), 0..3))
            .prop_map(|(name, args)| Type::Named { name, args }),
    ]
    .prop_map(sp);

    leaf.prop_recursive(3, 16, 3, |inner| {
        prop_oneof![
            nat_strategy().prop_map(|n| sp(Type::QReg(n))),
            inner.clone().prop_map(|t| sp(Type::Q(Box::new(t)))),
            inner.clone().prop_map(|t| sp(Type::List(Box::new(t)))),
            (nat_strategy(), nat_strategy(), inner.clone()).prop_map(|(n, m, t)| sp(Type::Matrix(
                n,
                m,
                Box::new(t)
            ))),
            (
                nat_strategy(),
                nat_strategy(),
                nat_strategy(),
                class_strategy()
            )
                .prop_map(|(n, m, d, c)| sp(Type::Circuit { n, m, d, c })),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| sp(Type::Fn(Box::new(a), Box::new(b)))),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| sp(Type::Linear(Box::new(a), Box::new(b)))),
            prop::collection::vec(inner, 2..4).prop_map(|ts| sp(Type::Tuple(ts))),
        ]
    })
    .boxed()
}

fn pat_strategy() -> BoxedStrategy<Sp<Pat>> {
    let leaf = prop_oneof![
        Just(Pat::Wildcard),
        ident().prop_map(Pat::Var),
        (0i64..100).prop_map(|n| Pat::Lit(LitPat::Int(n))),
        any::<bool>().prop_map(|b| Pat::Lit(LitPat::Bool(b))),
    ]
    .prop_map(sp);
    leaf.prop_recursive(2, 6, 3, |inner| {
        prop::collection::vec(inner, 2..4).prop_map(|ps| sp(Pat::Tuple(ps)))
    })
    .boxed()
}

fn expr_strategy() -> BoxedStrategy<Sp<Expr>> {
    let leaf = prop_oneof![
        (0i64..1000).prop_map(Expr::Int),
        float_lit().prop_map(Expr::Float),
        any::<bool>().prop_map(Expr::Bool),
        Just(Expr::Unit),
        ident().prop_map(Expr::Var),
    ]
    .prop_map(sp);

    leaf.prop_recursive(4, 96, 4, move |inner| {
        let binop = prop_oneof![
            Just(BinOp::Add),
            Just(BinOp::Sub),
            Just(BinOp::Mul),
            Just(BinOp::Div),
            Just(BinOp::Pow),
        ];
        let stmt = prop_oneof![
            (pat_strategy(), inner.clone()).prop_map(|(pat, rhs)| sp(Stmt::Bind { pat, rhs })),
            (pat_strategy(), inner.clone()).prop_map(|(pat, rhs)| sp(Stmt::Let { pat, rhs })),
            inner.clone().prop_map(|e| sp(Stmt::Expr(e))),
        ];
        let stmts = prop::collection::vec(stmt, 1..4);

        prop_oneof![
            // application / operators
            (inner.clone(), inner.clone())
                .prop_map(|(f, x)| sp(Expr::App(Box::new(f), Box::new(x)))),
            (inner.clone(), binop, inner.clone()).prop_map(|(l, op, r)| sp(Expr::BinOp {
                op,
                lhs: Box::new(l),
                rhs: Box::new(r),
            })),
            inner.clone().prop_map(|e| sp(Expr::Neg(Box::new(e)))),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| sp(Expr::Compose(Box::new(a), Box::new(b)))),
            (inner.clone(), inner.clone()).prop_map(|(g, q)| sp(Expr::GateApp {
                gate: Box::new(g),
                qubits: Box::new(q),
            })),
            inner.clone().prop_map(|e| sp(Expr::Adjoint(Box::new(e)))),
            inner
                .clone()
                .prop_map(|e| sp(Expr::Controlled(Box::new(e)))),
            inner.clone().prop_map(|e| sp(Expr::Return(Box::new(e)))),
            (inner.clone(), ty_strategy()).prop_map(|(e, t)| sp(Expr::Ascribe(Box::new(e), t))),
            // collections (avoid 1-tuples: they are grouping, not tuples)
            prop::collection::vec(inner.clone(), 2..4).prop_map(|es| sp(Expr::Tuple(es))),
            prop::collection::vec(inner.clone(), 0..4).prop_map(|es| sp(Expr::List(es))),
            // binding / control
            (
                prop::collection::vec((pat_strategy(), prop::option::of(ty_strategy())), 1..3),
                inner.clone()
            )
                .prop_map(|(params, body)| sp(Expr::Lam {
                    params,
                    body: Box::new(body)
                })),
            (pat_strategy(), inner.clone(), inner.clone()).prop_map(|(pat, rhs, body)| sp(
                Expr::Let {
                    pat,
                    rhs: Box::new(rhs),
                    body: Box::new(body)
                }
            )),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(c, t, e)| sp(Expr::If {
                cond: Box::new(c),
                then: Box::new(t),
                else_: Box::new(e),
            })),
            (
                inner.clone(),
                prop::collection::vec((pat_strategy(), inner.clone()), 1..3)
            )
                .prop_map(|(s, arms)| sp(Expr::Match {
                    scrutinee: Box::new(s),
                    arms
                })),
            (pat_strategy(), inner.clone(), inner.clone()).prop_map(|(pat, it, body)| sp(
                Expr::For {
                    pat,
                    iter: Box::new(it),
                    body: Box::new(body)
                }
            )),
            (inner.clone(), inner.clone())
                .prop_map(|(b, c)| sp(Expr::Par(Box::new(b), Box::new(c)))),
            // blocks
            stmts.clone().prop_map(|ss| sp(Expr::CircuitBlock(ss))),
            stmts.clone().prop_map(|ss| sp(Expr::RunBlock(ss))),
            (prop::collection::vec((ident(), ty_strategy()), 1..3), stmts)
                .prop_map(|(bindings, body)| sp(Expr::Borrow { bindings, body })),
        ]
    })
    .boxed()
}

fn decl_strategy() -> BoxedStrategy<Sp<Decl>> {
    let fn_decl = (
        ident(),
        prop::collection::vec((ident(), ty_strategy()), 0..3),
        ty_strategy(),
        expr_strategy(),
    )
        .prop_map(|(name, params, ret, body)| {
            sp(Decl::Fn {
                name,
                params,
                ret,
                body,
            })
        });
    let alias = (
        type_name(),
        prop::collection::vec(ident(), 0..3),
        ty_strategy(),
    )
        .prop_map(|(name, params, ty)| sp(Decl::TypeAlias { name, params, ty }));
    prop_oneof![fn_decl, alias].boxed()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    /// pretty → lex → parse → strip equals the original AST.
    #[test]
    fn ast_roundtrips(decls in prop::collection::vec(decl_strategy(), 1..3)) {
        let printed = pretty(&decls);
        let tokens = lex(&printed)
            .map_err(|e| TestCaseError::fail(format!("re-lex failed: {e:?}\n---\n{printed}")))?;
        let mut reparsed = parse(&tokens)
            .map_err(|e| TestCaseError::fail(format!("re-parse failed: {e:?}\n---\n{printed}")))?;
        strip_decls(&mut reparsed);
        prop_assert_eq!(reparsed, decls, "roundtrip mismatch\n--- printed ---\n{}", printed);
    }

    /// Printing is idempotent: printing a reparsed tree yields byte-identical source.
    #[test]
    fn print_is_idempotent(decls in prop::collection::vec(decl_strategy(), 1..3)) {
        let printed = pretty(&decls);
        let tokens = lex(&printed)
            .map_err(|e| TestCaseError::fail(format!("re-lex failed: {e:?}")))?;
        let reparsed = parse(&tokens)
            .map_err(|e| TestCaseError::fail(format!("re-parse failed: {e:?}")))?;
        let printed2 = pretty(&reparsed);
        prop_assert_eq!(printed, printed2);
    }
}
