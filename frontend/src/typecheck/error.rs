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
