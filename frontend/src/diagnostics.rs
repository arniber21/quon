//! Unified frontend diagnostics.
//!
//! Each pipeline stage (lexer, parser, …) reports errors in its own shape. The
//! [`Diagnostic`] type is the one currency the frontend facade ([`crate::parse_program`])
//! folds them all into, so callers learn a single error type rather than one per
//! stage.

use crate::lexer::{SimpleSpan, Sp};

/// A single frontend error: a message anchored at a source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Human-readable description of the problem.
    pub message: String,
    /// The source span the diagnostic points at.
    pub span: SimpleSpan,
}

impl Diagnostic {
    /// Creates a diagnostic at `span`.
    pub fn new(message: impl Into<String>, span: SimpleSpan) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// Lexer and parser both report `(message, span)` pairs; this is the bind that
/// lifts a stage's errors into the unified [`Diagnostic`] stream.
impl From<Sp<String>> for Diagnostic {
    fn from((message, span): Sp<String>) -> Self {
        Diagnostic { message, span }
    }
}

/// Converts a stage's error list into diagnostics.
pub(crate) fn from_stage(errors: Vec<Sp<String>>) -> Vec<Diagnostic> {
    errors.into_iter().map(Diagnostic::from).collect()
}
