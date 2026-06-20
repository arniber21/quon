// AST type definitions — see issue #7, SPEC.md §2–§3
// All nodes are Sp<T> = (T, SimpleSpan) for span-accurate error reporting.

use crate::lexer::Sp;

pub type Name = String;

// ── Top-level ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Decl {
    Fn {
        name: Name,
        params: Vec<(Name, Sp<Type>)>,
        ret: Sp<Type>,
        body: Sp<Expr>,
    },
    TypeAlias {
        name: Name,
        ty: Sp<Type>,
    },
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Type {
    Qubit,
    QReg(Sp<NatExpr>),
    Bit,
    Bool,
    Int,
    Float,
    Unit,
    List(Box<Sp<Type>>),
    Tuple(Vec<Sp<Type>>),
    Fn(Box<Sp<Type>>, Box<Sp<Type>>),     // unrestricted
    Linear(Box<Sp<Type>>, Box<Sp<Type>>), // -o
    Circuit {
        n: Sp<NatExpr>,
        m: Sp<NatExpr>,
        d: Sp<DepthExpr>,
        c: CliffordClass,
    },
    Q(Box<Sp<Type>>),
    Matrix(Sp<NatExpr>, Sp<NatExpr>, Box<Sp<Type>>),
    Var(Name),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliffordClass {
    Clifford,
    Universal,
    Infer, // placeholder during parsing; resolved by type checker
}

// ── Nat / Depth expressions ───────────────────────────────────────────────────

/// Type-level natural number expression (appears in QReg<n>, Circuit<n,...>).
#[derive(Debug, Clone)]
pub enum NatExpr {
    Lit(u64),
    Var(Name),
    Add(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    Mul(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    Sub(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    Exp(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
}

/// Symbolic depth expression stored as `DepthExprAttr` in MLIR.
/// Serialized as S-expressions for the MLIR text format.
#[derive(Debug, Clone, PartialEq)]
pub enum DepthExpr {
    Lit(u64),
    Var(Name),
    Add(Box<DepthExpr>, Box<DepthExpr>),
    Mul(Box<DepthExpr>, Box<DepthExpr>),
    Max(Box<DepthExpr>, Box<DepthExpr>),
}

// ── Expressions ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    // Literals
    Int(i64),
    Float(f64),
    Bool(bool),
    Unit,

    // Variables
    Var(Name),

    // Functions
    Lam {
        param: Name,
        param_ty: Sp<Type>,
        body: Box<Sp<Expr>>,
    },
    App(Box<Sp<Expr>>, Box<Sp<Expr>>),

    // Let
    Let {
        pat: Sp<Pat>,
        rhs: Box<Sp<Expr>>,
        body: Box<Sp<Expr>>,
    },

    // Control
    If {
        cond: Box<Sp<Expr>>,
        then: Box<Sp<Expr>>,
        else_: Box<Sp<Expr>>,
    },
    Match {
        scrutinee: Box<Sp<Expr>>,
        arms: Vec<(Sp<Pat>, Sp<Expr>)>,
    },

    // Tuples
    Tuple(Vec<Sp<Expr>>),

    // Circuit forms
    CircuitBlock(Vec<Sp<Stmt>>),
    Compose(Box<Sp<Expr>>, Box<Sp<Expr>>), // |>
    Par(Box<Sp<Expr>>, Box<Sp<Expr>>),
    Adjoint(Box<Sp<Expr>>),
    Controlled(Box<Sp<Expr>>),
    GateApp {
        gate: Box<Sp<Expr>>,
        qubits: Box<Sp<Expr>>,
    }, // @

    // Monadic (post-desugaring)
    RunBlock(Vec<Sp<Stmt>>), // pre-desugaring
    Bind {
        rhs: Box<Sp<Expr>>,
        param: Name,
        body: Box<Sp<Expr>>,
    }, // post-desugaring
    Return(Box<Sp<Expr>>),

    // Borrow
    Borrow {
        name: Name,
        body: Box<Sp<Expr>>,
    },

    // Type annotation
    Ascribe(Box<Sp<Expr>>, Sp<Type>),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Bind { name: Name, rhs: Sp<Expr> },  // x <- e
    Let { pat: Sp<Pat>, rhs: Sp<Expr> }, // let p = e
    Expr(Sp<Expr>),                      // e (last statement)
}

#[derive(Debug, Clone)]
pub enum Pat {
    Wildcard,
    Var(Name),
    Tuple(Vec<Sp<Pat>>),
    Lit(LitPat),
}

#[derive(Debug, Clone)]
pub enum LitPat {
    Int(i64),
    Bool(bool),
}
