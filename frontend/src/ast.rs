// AST type definitions — see issue #7, SPEC.md §2–§5, §12, PRD (GitHub issue #1)
// All nodes are Sp<T> = (T, SimpleSpan) for span-accurate error reporting.
//
// PartialEq is derived throughout so the test harness can compare ASTs structurally
// (after span normalization — see frontend/tests/support). f64 in `Expr::Float`/binary
// arithmetic is why these derive PartialEq, not Eq.

use crate::lexer::Sp;

pub type Name = String;

// ── Top-level ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    Fn {
        name: Sp<Name>,
        params: Vec<(Sp<Name>, Sp<Type>)>,
        ret: Sp<Type>,
        body: Sp<Expr>,
    },
    TypeAlias {
        name: Sp<Name>,
        /// Type-level parameters, e.g. `n` in `type Oracle<n> = ...`. Empty for plain aliases.
        params: Vec<Sp<Name>>,
        ty: Sp<Type>,
    },
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Qubit,
    QReg(Sp<NatExpr>),
    Bit,
    Bool,
    Int,
    Float,
    Unit,
    /// Type-level natural number kind, used for value parameters like `n: Nat`.
    Nat,
    List(Box<Sp<Type>>),
    Tuple(Vec<Sp<Type>>),
    Fn(Box<Sp<Type>>, Box<Sp<Type>>),     // unrestricted ->
    Linear(Box<Sp<Type>>, Box<Sp<Type>>), // -o
    Circuit {
        n: Sp<NatExpr>,
        m: Sp<NatExpr>,
        // Surface depth annotation. This is full Nat arithmetic (`n-1`, `n/2`, `n^3`, `p*(n*n+1)`,
        // or a `_` hole) as written by the user; the type checker normalizes/verifies it against
        // the symbolic `DepthExpr` (Add/Mul/Max only) used downstream in MLIR.
        d: Sp<NatExpr>,
        c: CliffordClass,
    },
    Q(Box<Sp<Type>>),
    Matrix(Sp<NatExpr>, Sp<NatExpr>, Box<Sp<Type>>),
    /// Bare type variable (no arguments), e.g. a free `n` used as a type.
    Var(Name),
    /// Reference to a (possibly parameterized) type alias, e.g. `Oracle<n>`, `BVOracle<n+1>`,
    /// or `Bell` (with no args). Resolved against the alias table by the type checker.
    Named {
        name: Name,
        args: Vec<Sp<NatExpr>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliffordClass {
    Clifford,
    Universal,
    Infer, // placeholder during parsing; resolved by type checker
}

impl CliffordClass {
    /// The classification lattice join (`⊔`, SPEC §3.3): `Universal` absorbs `Clifford`.
    /// Used to propagate the class through composition (`|>`, `par`, `controlled`, …).
    /// `Infer` is treated as the `Clifford` identity so it never poisons a join.
    pub fn join(&self, other: &CliffordClass) -> CliffordClass {
        match (self, other) {
            (CliffordClass::Universal, _) | (_, CliffordClass::Universal) => {
                CliffordClass::Universal
            }
            _ => CliffordClass::Clifford,
        }
    }

    /// The classification lattice order (`⊑`, SPEC §3.3/§3.7): `Clifford ⊑ Universal`, with
    /// `Universal` the top. This is the *subsumption* used when checking an inferred class
    /// against an expected one — a `Clifford` circuit is acceptable wherever a `Universal`
    /// circuit is expected, but not the reverse. Distinct from [`Self::join`] (inference):
    /// `join` is the least upper bound used to *propagate* a class through composition, `leq`
    /// is the order used to *check* one against an annotation. `Infer` is the `Clifford`
    /// bottom, so it is below everything.
    pub fn leq(&self, other: &CliffordClass) -> bool {
        match (self, other) {
            // Universal is only ⊑ Universal.
            (CliffordClass::Universal, CliffordClass::Universal) => true,
            (CliffordClass::Universal, _) => false,
            // Clifford (and the Infer bottom) are ⊑ anything.
            _ => true,
        }
    }
}

// ── Nat expressions ───────────────────────────────────────────────────────────

/// Type-level natural number expression (appears in QReg<n>, Circuit<n,...>).
#[derive(Debug, Clone, PartialEq)]
pub enum NatExpr {
    Lit(u64),
    Var(Name),
    Add(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    Mul(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    Sub(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    Div(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    Exp(Box<Sp<NatExpr>>, Box<Sp<NatExpr>>),
    /// `_` placeholder in a depth position (`Circuit<n, m, _, C>`); resolved by the type checker.
    Hole,
}

// ── Expressions ──────────────────────────────────────────────────────────────

/// Binary arithmetic operator at the value level (`+ - * / ^`).
/// Type-level arithmetic uses `NatExpr` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    // Literals
    Int(i64),
    Float(f64),
    Bool(bool),
    Unit,

    // Variables
    Var(Name),

    // Functions
    /// Lambda: `fn(p1, p2, ...) -> body`. Each parameter is a pattern with an optional type
    /// annotation (annotations are usually omitted in the surface syntax).
    Lam {
        params: Vec<(Sp<Pat>, Option<Sp<Type>>)>,
        body: Box<Sp<Expr>>,
    },
    App(Box<Sp<Expr>>, Box<Sp<Expr>>),

    // Arithmetic
    BinOp {
        op: BinOp,
        lhs: Box<Sp<Expr>>,
        rhs: Box<Sp<Expr>>,
    },
    Neg(Box<Sp<Expr>>),

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
    /// `for pat in iter { body }` — produces a circuit (parallel/sequential per §5.8).
    For {
        pat: Sp<Pat>,
        iter: Box<Sp<Expr>>,
        body: Box<Sp<Expr>>,
    },

    // Collections
    Tuple(Vec<Sp<Expr>>),
    List(Vec<Sp<Expr>>),

    // Circuit forms
    CircuitBlock(Vec<Sp<Stmt>>),
    Compose(Box<Sp<Expr>>, Box<Sp<Expr>>), // |>
    /// `par { body } * count` — n-fold tensor product of `body` with itself (§5.8).
    /// First field is the circuit body, second is the repeat count.
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
        param: Sp<Name>,
        body: Box<Sp<Expr>>,
    }, // post-desugaring
    Return(Box<Sp<Expr>>),

    // Borrow
    /// `borrow b1: T1, b2: T2 in { stmts }` — scoped ancilla allocation (§3.4, ADR-0003).
    Borrow {
        bindings: Vec<(Sp<Name>, Sp<Type>)>,
        body: Vec<Sp<Stmt>>,
    },

    // Type annotation
    Ascribe(Box<Sp<Expr>>, Sp<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Bind { pat: Sp<Pat>, rhs: Sp<Expr> }, // pat <- e
    Let { pat: Sp<Pat>, rhs: Sp<Expr> },  // let p = e
    Expr(Sp<Expr>),                       // e (last statement)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pat {
    Wildcard,
    Var(Name),
    Tuple(Vec<Sp<Pat>>),
    Lit(LitPat),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LitPat {
    Int(i64),
    Bool(bool),
}
