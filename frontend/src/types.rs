// Type definitions and kind checker — see SPEC.md §3.1–§3.2
// Canonical runtime representations of Quon types used by the type checker.

use crate::ast::{CliffordClass, Name};
use quon_core::DepthExpr;
use std::fmt;

/// Fully resolved Quon type (post kind-checking).
///
/// `Var` is a *rigid* type-level name (a quantified variable in a builtin scheme,
/// before instantiation). `Meta` is a *flexible* unification variable created by the
/// type checker during inference; every `Meta` is solved (or defaulted) and zonked
/// away before a type leaves the checker, so well-typed output never contains one.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Qubit,
    /// A register of `n` qubits. The size is a symbolic [`DepthExpr`] so registers over
    /// type-level variables (`QReg<n>`, `QReg<n+1>`) are representable, not just literals.
    QReg(DepthExpr),
    Bit,
    Bool,
    Int,
    Float,
    Unit,
    List(Box<Ty>),
    Tuple(Vec<Ty>),
    Fn(Box<Ty>, Box<Ty>),
    Linear(Box<Ty>, Box<Ty>),
    /// A unitary circuit morphism. Qubit counts `n`/`m` and depth `d` are symbolic
    /// [`DepthExpr`]s; `c` is the inferred Clifford classification.
    Circuit {
        n: DepthExpr,
        m: DepthExpr,
        d: DepthExpr,
        c: CliffordClass,
    },
    Q(Box<Ty>),
    Matrix(DepthExpr, DepthExpr, Box<Ty>),
    /// Rigid type-level variable (e.g. a quantified `A` in a prelude scheme).
    Var(Name),
    /// Flexible unification variable, keyed into the checker's substitution.
    Meta(u32),
}

impl Ty {
    /// Whether a value of this type is a **linear resource** — one that must be consumed
    /// exactly once and lives in the linear context `Δ` (SPEC §3.4). These are the quantum
    /// values: a `Qubit`, a `QReg<n>`, a `Circuit{..}` value, and any aggregate (`Tuple`,
    /// `List`) that carries one.
    ///
    /// Note what is *excluded*: a `Ty::Linear(a, b)` (a `-o` function) is a reusable
    /// function value — the linearity is a promise about its *argument*, not about the
    /// function itself, so `measure : Qubit -o Q<Bit>` may be called freely. Likewise `Q<τ>`
    /// is an unrestricted monadic computation. Both stay in the unrestricted context `Γ`.
    pub fn is_linear_resource(&self) -> bool {
        match self {
            Ty::Qubit | Ty::QReg(_) | Ty::Circuit { .. } => true,
            Ty::List(t) => t.is_linear_resource(),
            Ty::Tuple(ts) => ts.iter().any(Ty::is_linear_resource),
            _ => false,
        }
    }

    /// Convenience constructor for an unrestricted function type `a -> b`.
    pub fn func(a: Ty, b: Ty) -> Ty {
        Ty::Fn(Box::new(a), Box::new(b))
    }

    /// Convenience constructor for `List<τ>`.
    pub fn list(t: Ty) -> Ty {
        Ty::List(Box::new(t))
    }
}

/// Surface syntax for types, matching how a programmer would write them. Used in
/// error messages so the user sees `(Int, Bool) -> List<Int>` rather than the raw
/// `Tuple([Int, Bool])` debug form.
impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Qubit => f.write_str("Qubit"),
            Ty::QReg(n) => write!(f, "QReg<{n}>"),
            Ty::Bit => f.write_str("Bit"),
            Ty::Bool => f.write_str("Bool"),
            Ty::Int => f.write_str("Int"),
            Ty::Float => f.write_str("Float"),
            Ty::Unit => f.write_str("Unit"),
            Ty::List(t) => write!(f, "List<{t}>"),
            Ty::Tuple(ts) => {
                f.write_str("(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{t}")?;
                }
                f.write_str(")")
            }
            // Parenthesize the domain of a function so `(A -> B) -> C` is unambiguous.
            Ty::Fn(a, b) => write!(f, "{} -> {b}", ArrowDomain(a)),
            Ty::Linear(a, b) => write!(f, "{} -o {b}", ArrowDomain(a)),
            Ty::Circuit { n, m, d, c } => write!(f, "Circuit<{n}, {m}, {d}, {c:?}>"),
            Ty::Q(t) => write!(f, "Q<{t}>"),
            Ty::Matrix(n, m, t) => write!(f, "Matrix<{n}, {m}, {t}>"),
            Ty::Var(name) => f.write_str(name),
            Ty::Meta(id) => write!(f, "?{id}"),
        }
    }
}

/// Wraps a function's domain type, adding parentheses only when the domain is itself
/// a function (which would otherwise re-associate ambiguously).
struct ArrowDomain<'a>(&'a Ty);

impl fmt::Display for ArrowDomain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Ty::Fn(..) | Ty::Linear(..) => write!(f, "({})", self.0),
            other => write!(f, "{other}"),
        }
    }
}
