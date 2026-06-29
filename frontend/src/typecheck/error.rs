//! Type-checker diagnostics.
//!
//! Every error carries the source [`SimpleSpan`] of the offending sub-term so the
//! frontend can point a caret at the exact token (issue #9: "span-accurate errors").
//! [`TypeError`] lowers into the unified [`Diagnostic`] currency via [`TypeError::span`]
//! and its `Display` impl, keeping the renderer (ariadne) decoupled from the checker.

use crate::diagnostics::Diagnostic;
use crate::lexer::SimpleSpan;
use crate::types::Ty;
use std::fmt;

/// A single type error, anchored at the span of the sub-expression that triggered it.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    /// Two types failed to unify (the workhorse mismatch).
    Mismatch {
        expected: Ty,
        found: Ty,
        span: SimpleSpan,
    },
    /// A name was used but never bound in Γ, the prelude, or the constants table.
    UnboundVariable { name: String, span: SimpleSpan },
    /// The callee of an application did not have a function type.
    NotAFunction { found: Ty, span: SimpleSpan },
    /// A value was used arithmetically (or negated) but is not `Int`/`Float`.
    NotNumeric { found: Ty, span: SimpleSpan },
    /// A tuple pattern/expression had the wrong number of components for its type.
    ArityMismatch {
        expected: usize,
        found: usize,
        span: SimpleSpan,
    },
    /// A `match` does not cover every value of the scrutinee's type.
    NonExhaustive {
        /// A human-readable witness pattern that is not matched, e.g. `(true, _)`.
        witness: String,
        span: SimpleSpan,
    },
    /// A `match` arm can never be reached because earlier arms already cover it.
    UnreachableArm { span: SimpleSpan },
    /// A lambda with unannotated parameters appeared where its type could not be
    /// inferred top-down. Give it an expected type (an ascription or a known callee).
    AmbiguousLambda { span: SimpleSpan },
    /// An infinite type would result from unification (e.g. `?0 = List<?0>`).
    OccursCheck { span: SimpleSpan },
    /// A type alias was referenced with the wrong number of arguments.
    AliasArity {
        name: String,
        expected: usize,
        found: usize,
        span: SimpleSpan,
    },
    /// A linear resource (`Qubit`/`QReg`/`Circuit` value) was used more than once —
    /// contraction is absent from the linear context, so no-cloning is a type error.
    /// `first` is the consuming use; `span` (== the second use) is where the caret lands.
    LinearUsedTwice {
        name: String,
        first: SimpleSpan,
        span: SimpleSpan,
    },
    /// A linear resource went out of scope without being consumed — weakening is absent,
    /// so no-dropping is a type error. `span` points at the binding that introduced it.
    LinearUnconsumed { name: String, span: SimpleSpan },
    /// The branches of an `if`/`match` disagree on which linear resources they consume:
    /// `name` is spent on one path but not all. `span` points at the offending branch.
    LinearBranchMismatch { name: String, span: SimpleSpan },
    /// A linear resource was bound to a wildcard `_`, i.e. silently discarded. Permitted
    /// discards (a measured qubit, a borrow-reset qubit) arrive with issues #14/#15.
    LinearDiscard { name: String, span: SimpleSpan },
    /// A lambda body referred to a linear resource from the enclosing scope. A function
    /// value may run zero or many times, so it cannot consume a resource exactly once.
    LinearCapture { name: String, span: SimpleSpan },
    /// A value was used as a circuit (composed, placed, adjointed, …) but is not one.
    NotACircuit { found: Ty, span: SimpleSpan },
    /// Sequential composition `f |> g` requires `f`'s output width to equal `g`'s input
    /// width. `expected`/`found` are the two register sizes, rendered for the message.
    QubitCountMismatch {
        expected: String,
        found: String,
        span: SimpleSpan,
    },
    /// A gate was placed on the wrong number of qubit targets (`H @ (0,1)`, `CNOT @ 0`).
    GateTargetArity {
        expected: u64,
        found: usize,
        span: SimpleSpan,
    },
    /// A gate was targeted at a qubit index outside the ambient register `0..width`.
    IndexOutOfBounds {
        index: u64,
        width: u64,
        span: SimpleSpan,
    },
    /// A user-supplied Clifford annotation disagrees with the inferred classification
    /// (issue #12). `expected` is the annotation, `found` the inferred class.
    CliffordMismatch {
        expected: crate::ast::CliffordClass,
        found: crate::ast::CliffordClass,
        span: SimpleSpan,
    },
    /// A user-supplied depth annotation could not be shown equal to the inferred symbolic
    /// depth (issue #13). `expected`/`found` are the two depth expressions, rendered for the
    /// message; Z3 found them unequal (or the constant fast-path did).
    DepthMismatch {
        expected: String,
        found: String,
        span: SimpleSpan,
    },
    /// A symbolic depth constraint was beyond what the refinement solver could decide
    /// (issue #13) — e.g. an intractable nonlinear term. The user must supply a static bound.
    DepthIntractable { expr: String, span: SimpleSpan },
    /// The right-hand side of a monadic `<-` bind was expected to be a quantum computation
    /// `Q<_>` (or a pure value, auto-lifted) but was something else (issue #14).
    ExpectedMonad { found: Ty, span: SimpleSpan },
    /// A borrowed ancilla (`borrow q: Qubit in { … }`) appears in the block's result value, so
    /// it would escape the borrow scope (issue #15, SPEC §3.4). An ancilla must be cleaned up
    /// (measured, `reset`, or `discard`ed) inside the block, never returned. `span` points at
    /// the returned value that mentions it.
    BorrowEscape { name: String, span: SimpleSpan },
    /// A `Nat` value argument at a value-dependent call site (issue #57) could not be lowered to
    /// a symbolic depth — only `Int` literals, variables, and `+ - * / ^` over them specialize a
    /// dependent parameter. `func`/`param` name the callee and the offending parameter.
    NonDependentArg {
        func: String,
        param: String,
        span: SimpleSpan,
    },
    /// A recursive circuit function whose recursion could not be shown to terminate (issue #60):
    /// no `Nat` parameter strictly decreases (and stays `≥ 0`) at every recursive call. Without a
    /// well-founded measure the depth index is not a bound on any finite circuit, so the function
    /// is rejected — and reported, never looped on.
    IllFoundedRecursion { name: String, span: SimpleSpan },
    /// Mutual recursion among circuit functions (issue #60). v1 supports only direct
    /// self-recursion with an inferred decreasing measure; a cycle through two or more distinct
    /// functions is rejected rather than accepted without a termination witness.
    MutualRecursion { name: String, span: SimpleSpan },
    /// A construct that belongs to the linear/quantum fragment (issues #10–#15) was
    /// encountered while type-checking the classical fragment.
    Unsupported {
        construct: &'static str,
        span: SimpleSpan,
    },
}

impl TypeError {
    /// The source span this error points at.
    pub fn span(&self) -> SimpleSpan {
        match self {
            TypeError::Mismatch { span, .. }
            | TypeError::UnboundVariable { span, .. }
            | TypeError::NotAFunction { span, .. }
            | TypeError::NotNumeric { span, .. }
            | TypeError::ArityMismatch { span, .. }
            | TypeError::NonExhaustive { span, .. }
            | TypeError::UnreachableArm { span }
            | TypeError::AmbiguousLambda { span }
            | TypeError::OccursCheck { span }
            | TypeError::AliasArity { span, .. }
            | TypeError::LinearUsedTwice { span, .. }
            | TypeError::LinearUnconsumed { span, .. }
            | TypeError::LinearBranchMismatch { span, .. }
            | TypeError::LinearDiscard { span, .. }
            | TypeError::LinearCapture { span, .. }
            | TypeError::NotACircuit { span, .. }
            | TypeError::QubitCountMismatch { span, .. }
            | TypeError::GateTargetArity { span, .. }
            | TypeError::IndexOutOfBounds { span, .. }
            | TypeError::CliffordMismatch { span, .. }
            | TypeError::DepthMismatch { span, .. }
            | TypeError::DepthIntractable { span, .. }
            | TypeError::ExpectedMonad { span, .. }
            | TypeError::BorrowEscape { span, .. }
            | TypeError::NonDependentArg { span, .. }
            | TypeError::IllFoundedRecursion { span, .. }
            | TypeError::MutualRecursion { span, .. }
            | TypeError::Unsupported { span, .. } => *span,
        }
    }

    /// Lowers this error into the frontend's unified diagnostic type.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::new(self.to_string(), self.span())
    }
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::Mismatch {
                expected, found, ..
            } => write!(f, "type mismatch: expected `{expected}`, found `{found}`"),
            TypeError::UnboundVariable { name, .. } => write!(f, "unbound variable `{name}`"),
            TypeError::NotAFunction { found, .. } => {
                write!(f, "cannot apply a value of non-function type `{found}`")
            }
            TypeError::NotNumeric { found, .. } => {
                write!(f, "arithmetic requires `Int` or `Float`, found `{found}`")
            }
            TypeError::ArityMismatch {
                expected, found, ..
            } => write!(f, "expected a {expected}-tuple, found {found} components"),
            TypeError::NonExhaustive { witness, .. } => {
                write!(f, "non-exhaustive `match`: pattern `{witness}` not covered")
            }
            TypeError::UnreachableArm { .. } => write!(f, "unreachable `match` arm"),
            TypeError::AmbiguousLambda { .. } => write!(
                f,
                "cannot infer the type of this lambda; add parameter annotations or an expected type"
            ),
            TypeError::OccursCheck { .. } => {
                write!(f, "cannot construct an infinite type")
            }
            TypeError::AliasArity {
                name,
                expected,
                found,
                ..
            } => write!(
                f,
                "type alias `{name}` expects {expected} argument(s), found {found}"
            ),
            TypeError::LinearUsedTwice { name, .. } => write!(
                f,
                "linear resource `{name}` is used more than once (no-cloning)"
            ),
            TypeError::LinearUnconsumed { name, .. } => write!(
                f,
                "linear resource `{name}` is never consumed (no-dropping)"
            ),
            TypeError::LinearBranchMismatch { name, .. } => write!(
                f,
                "linear resource `{name}` is consumed in some branches but not all"
            ),
            TypeError::LinearDiscard { name, .. } => {
                write!(f, "cannot discard linear resource `{name}` with `_`")
            }
            TypeError::LinearCapture { name, .. } => {
                write!(f, "cannot capture linear resource `{name}` in a closure")
            }
            TypeError::NotACircuit { found, .. } => {
                write!(f, "expected a circuit, found `{found}`")
            }
            TypeError::QubitCountMismatch {
                expected, found, ..
            } => write!(
                f,
                "circuit composition requires matching qubit counts: \
                 left produces {expected}, right consumes {found}"
            ),
            TypeError::GateTargetArity {
                expected, found, ..
            } => write!(
                f,
                "this gate acts on {expected} qubit(s), but {found} target(s) were given"
            ),
            TypeError::IndexOutOfBounds { index, width, .. } => write!(
                f,
                "qubit index {index} is out of bounds for a register of width {width}"
            ),
            TypeError::CliffordMismatch {
                expected, found, ..
            } => write!(
                f,
                "Clifford classification mismatch: annotated `{expected:?}`, inferred `{found:?}`"
            ),
            TypeError::DepthMismatch {
                expected, found, ..
            } => write!(
                f,
                "circuit depth mismatch: annotated `{expected}`, inferred `{found}`"
            ),
            TypeError::DepthIntractable { expr, .. } => write!(
                f,
                "depth constraint `{expr}` is too complex for the solver to verify; \
                 supply a static depth bound"
            ),
            TypeError::ExpectedMonad { found, .. } => write!(
                f,
                "the right-hand side of `<-` must be a quantum computation `Q<_>`, found `{found}`"
            ),
            TypeError::BorrowEscape { name, .. } => write!(
                f,
                "borrowed ancilla `{name}` escapes its borrow scope; it must be measured, \
                 `reset`, or `discard`ed inside the block, not returned"
            ),
            TypeError::NonDependentArg { func, param, .. } => write!(
                f,
                "argument for the `Nat` parameter `{param}` of `{func}` is not a static depth \
                 expression; use an `Int` literal, variable, or `+ - * / ^` over them"
            ),
            TypeError::IllFoundedRecursion { name, .. } => write!(
                f,
                "cannot prove that the recursive function `{name}` terminates; some `Nat` \
                 parameter must strictly decrease (and stay non-negative) at every recursive call \
                 — add or adjust a base case so the recursion is well-founded"
            ),
            TypeError::MutualRecursion { name, .. } => write!(
                f,
                "`{name}` is part of a mutually-recursive cycle; only direct self-recursion with a \
                 decreasing measure is supported"
            ),
            TypeError::Unsupported { construct, .. } => write!(
                f,
                "`{construct}` is part of the linear/quantum fragment and is not yet type-checked"
            ),
        }
    }
}

impl std::error::Error for TypeError {}

impl From<&TypeError> for Diagnostic {
    fn from(e: &TypeError) -> Self {
        e.to_diagnostic()
    }
}
