//! Unified frontend diagnostics.
//!
//! Each pipeline stage (lexer, parser, …) reports errors in its own shape. The
//! [`Diagnostic`] type is the legacy currency for `quonc` / ariadne; [`RichDiagnostic`]
//! preserves stable codes, severity, related spans, and quick fixes for IDE use.

pub mod fixes;

use crate::lexer::{SimpleSpan, Sp};

/// Stable LSP diagnostic code slug (canonical string form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagnosticCode(pub &'static str);

impl DiagnosticCode {
    // Lexer
    pub const LEX_INVALID_CHAR: Self = Self("quon.lex.invalid-char");
    pub const LEX_UNTERMINATED_COMMENT: Self = Self("quon.lex.unterminated-comment");

    // Parser (v1: default bucket only)
    pub const PARSE_UNEXPECTED_TOKEN: Self = Self("quon.parse.unexpected-token");

    // Desugar
    pub const DESUGAR_RUN_TRAILING_BIND: Self = Self("quon.desugar.run-trailing-bind");
    pub const DESUGAR_RUN_TRAILING_LET: Self = Self("quon.desugar.run-trailing-let");
    pub const DESUGAR_REFUTABLE_BIND: Self = Self("quon.desugar.refutable-bind-pattern");

    // Classical type
    pub const TYPE_MISMATCH: Self = Self("quon.type.mismatch");
    pub const TYPE_UNBOUND_VARIABLE: Self = Self("quon.type.unbound-variable");
    pub const TYPE_NOT_A_FUNCTION: Self = Self("quon.type.not-a-function");
    pub const TYPE_NOT_NUMERIC: Self = Self("quon.type.not-numeric");
    pub const TYPE_ARITY_MISMATCH: Self = Self("quon.type.arity-mismatch");
    pub const TYPE_NON_EXHAUSTIVE: Self = Self("quon.type.non-exhaustive-match");
    pub const TYPE_UNREACHABLE_ARM: Self = Self("quon.type.unreachable-arm");
    pub const TYPE_AMBIGUOUS_LAMBDA: Self = Self("quon.type.ambiguous-lambda");
    pub const TYPE_INFINITE: Self = Self("quon.type.infinite-type");
    pub const TYPE_ALIAS_ARITY: Self = Self("quon.type.alias-arity");

    // Linearity
    pub const LINEARITY_USED_TWICE: Self = Self("quon.linearity.used-twice");
    pub const LINEARITY_UNCONSUMED: Self = Self("quon.linearity.unconsumed");
    pub const LINEARITY_BRANCH_MISMATCH: Self = Self("quon.linearity.branch-mismatch");
    pub const LINEARITY_DISCARD: Self = Self("quon.linearity.discard");
    pub const LINEARITY_CAPTURE: Self = Self("quon.linearity.capture");

    // Circuit
    pub const CIRCUIT_NOT_A_CIRCUIT: Self = Self("quon.circuit.not-a-circuit");
    pub const CIRCUIT_QUBIT_COUNT: Self = Self("quon.circuit.qubit-count-mismatch");
    pub const CIRCUIT_GATE_TARGET: Self = Self("quon.circuit.gate-target-arity");
    pub const CIRCUIT_INDEX_OOB: Self = Self("quon.circuit.index-out-of-bounds");

    // Refinement
    pub const REFINEMENT_CLIFFORD: Self = Self("quon.refinement.clifford-mismatch");
    pub const REFINEMENT_DEPTH: Self = Self("quon.refinement.depth-mismatch");
    pub const REFINEMENT_DEPTH_INTRACTABLE: Self = Self("quon.refinement.depth-intractable");

    // Monad / borrow
    pub const MONAD_EXPECTED: Self = Self("quon.monad.expected-monad");
    pub const BORROW_ESCAPE: Self = Self("quon.borrow.escape");

    // Dependent / recursion
    pub const DEPENDENT_NON_DEPENDENT: Self = Self("quon.dependent.non-dependent-arg");
    pub const RECURSION_ILL_FOUNDED: Self = Self("quon.recursion.ill-founded");
    pub const RECURSION_MUTUAL: Self = Self("quon.recursion.mutual");

    // Other
    pub const UNSUPPORTED_QUANTUM: Self = Self("quon.unsupported.quantum-fragment");
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Diagnostic severity for IDE consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Secondary location attached to a primary diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedInfo {
    pub message: String,
    pub span: SimpleSpan,
}

/// Kind of quick fix for LSP code action mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickFixKind {
    QuickFix,
    RefactorRewrite,
}

/// A single text replacement in UTF-8 byte offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct TextEdit {
    pub span: SimpleSpan,
    pub replacement: String,
}

/// A suggested automated edit for a diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct QuickFix {
    pub title: String,
    pub kind: QuickFixKind,
    pub edits: Vec<TextEdit>,
    /// When true, the LSP layer may mark this action as preferred.
    pub preferred: bool,
}

/// Structured diagnostic with stable code and optional quick fixes.
#[derive(Debug, Clone, PartialEq)]
pub struct RichDiagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub span: SimpleSpan,
    pub related: Vec<RelatedInfo>,
    pub fixes: Vec<QuickFix>,
}

impl RichDiagnostic {
    pub fn new(
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        message: impl Into<String>,
        span: SimpleSpan,
    ) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            span,
            related: Vec::new(),
            fixes: Vec::new(),
        }
    }

    pub fn with_related(mut self, related: Vec<RelatedInfo>) -> Self {
        self.related = related;
        self
    }

    pub fn with_fixes(mut self, fixes: Vec<QuickFix>) -> Self {
        self.fixes = fixes;
        self
    }

    pub fn primary_label(&self) -> &str {
        &self.message
    }
}

/// Result of IDE-oriented analysis (parse + desugar + typecheck; no lowering).
#[derive(Debug, Clone, Default)]
pub struct AnalysisResult {
    pub diagnostics: Vec<RichDiagnostic>,
    /// Language intelligence snapshot (hover, completion, go-to-definition, semantic tokens).
    pub intelligence: crate::analysis::DocumentAnalysis,
}

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

impl From<&RichDiagnostic> for Diagnostic {
    fn from(d: &RichDiagnostic) -> Self {
        Diagnostic::new(d.message.clone(), d.span)
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

/// Classify a lexer error message into a stable code.
pub(crate) fn classify_lex_error(message: &str, span: SimpleSpan) -> RichDiagnostic {
    let code = if message.contains("comment") && message.contains("unclosed") {
        DiagnosticCode::LEX_UNTERMINATED_COMMENT
    } else {
        DiagnosticCode::LEX_INVALID_CHAR
    };
    RichDiagnostic::new(code, DiagnosticSeverity::Error, message.to_owned(), span)
}

/// Classify a parser error message (v1: single default bucket).
pub(crate) fn classify_parse_error(message: &str, span: SimpleSpan) -> RichDiagnostic {
    RichDiagnostic::new(
        DiagnosticCode::PARSE_UNEXPECTED_TOKEN,
        DiagnosticSeverity::Error,
        message.to_owned(),
        span,
    )
}

/// Map a desugar diagnostic message to a stable code.
pub(crate) fn classify_desugar_error(message: &str, span: SimpleSpan) -> RichDiagnostic {
    let code = if message.contains("`<-` bind") {
        DiagnosticCode::DESUGAR_RUN_TRAILING_BIND
    } else if message.contains("`let` binding") {
        DiagnosticCode::DESUGAR_RUN_TRAILING_LET
    } else if message.contains("literal pattern") {
        DiagnosticCode::DESUGAR_REFUTABLE_BIND
    } else {
        DiagnosticCode::PARSE_UNEXPECTED_TOKEN
    };
    RichDiagnostic::new(code, DiagnosticSeverity::Error, message.to_owned(), span)
}
