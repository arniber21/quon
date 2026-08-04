//! Quon frontend — lexer, parser, type checker, and AST→IR lowering.

#![allow(
    clippy::new_without_default,
    clippy::large_enum_variant,
    // `TypeError` carries resolved `Ty`s (with symbolic `Circuit`/`QReg` dimensions) for
    // span-accurate diagnostics, so `Result<_, TypeError>` has a large `Err` variant. The
    // error path is cold; boxing every `?` site would add churn for no real benefit.
    clippy::result_large_err,
    clippy::arc_with_non_send_sync // Z3 Context is !Send; Arc retained per PRD
)]

pub mod ast;
pub mod diagnostics;
pub mod lexer;
pub mod parser;
pub mod pretty;
pub mod visitor;

// The Melior-free frontend pipeline (parse → desugar → typecheck → elaborate →
// specialize) gates on `analyze`, which pulls in only `quon_core` / `z3` — no
// `mlir_bridge` / `melior`. `specialized_circuit` (issue #206) is the typed
// boundary between `elaborate` and `lower`. `lower` — the `quantum.circ` MLIR
// adapter — is the only module that needs `full` (Melior). Only `quonc` opts
// into `full`; `quonfmt` (parser only), `quonlint`, and `quon_lsp` (analysis
// only) all use `default-features = false` so a tooling build never links
// LLVM/MLIR, and specialization is buildable/testable under `--features
// analyze` without linking LLVM/MLIR (ADR-0038, issue #407).
#[cfg(feature = "analyze")]
pub mod analysis;
#[cfg(feature = "analyze")]
pub mod desugar;
#[cfg(feature = "analyze")]
pub mod elaborate;
#[cfg(feature = "full")]
pub mod lower;
#[cfg(feature = "analyze")]
pub mod refinement;
#[cfg(feature = "analyze")]
pub mod specialized_circuit;
#[cfg(feature = "analyze")]
pub mod typecheck;
#[cfg(feature = "analyze")]
pub mod types;

#[cfg(feature = "analyze")]
pub use analysis::DocumentAnalysis;
#[cfg(feature = "analyze")]
pub use analysis::TypedProgram;
#[cfg(feature = "analyze")]
pub use analysis::analyze_program;
#[cfg(feature = "analyze")]
pub use analysis::analyze_with_rich;

use crate::ast::Decl;
use crate::diagnostics::Diagnostic;
use crate::lexer::Sp;

#[cfg(feature = "analyze")]
pub use crate::diagnostics::fixes::apply_fixes;
#[cfg(feature = "analyze")]
pub use crate::diagnostics::{
    AnalysisResult, DiagnosticCode, DiagnosticSeverity, QuickFix, QuickFixKind, RelatedInfo,
    RichDiagnostic, TextEdit,
};

/// IDE-oriented analysis: lex → parse → desugar → typecheck. Does not lower to MLIR.
/// Accumulates errors from the first failing stage; never panics on partial source.
#[cfg(feature = "analyze")]
pub fn analyze(source: &str) -> AnalysisResult {
    analyze_with_rich(source)
}

/// The frontend's single entry point for turning source text into an AST: it
/// runs the lexer then the parser, folding both stages' errors into one
/// [`Diagnostic`] stream so callers never re-thread the pipeline themselves.
///
/// The individual stages ([`lexer::lex`], [`parser::parse`]) remain public as
/// internal seams for stage-level tests. Once desugaring, type checking, and
/// lowering land, this grows into a full `compile`; today the implemented
/// surface stops at a parsed declaration list.
pub fn parse_program(src: &str) -> Result<Vec<Sp<Decl>>, Vec<Diagnostic>> {
    let tokens = lexer::lex(src).map_err(diagnostics::from_stage)?;
    parser::parse(&tokens).map_err(diagnostics::from_stage)
}

/// Parse `src` and run the `run { }` desugaring pass (issue #8).
#[cfg(feature = "analyze")]
pub fn desugar_program(src: &str) -> Result<Vec<Sp<Decl>>, Vec<Diagnostic>> {
    let decls = parse_program(src)?;
    desugar::desugar_decls(decls)
}

/// Parse, desugar, and type-check a program (issues #9–#14).
#[cfg(feature = "analyze")]
pub fn check_program(src: &str) -> Result<(), Vec<Diagnostic>> {
    let result = analyze(src);
    if result.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(result.diagnostics.iter().map(Diagnostic::from).collect())
    }
}

/// Parse, desugar, type-check, and lower circuit functions to `quantum.circ` MLIR (issue #16).
#[cfg(feature = "full")]
pub fn lower_program_to_mlir(src: &str) -> Result<String, Vec<Diagnostic>> {
    let context = melior::Context::new();
    let module = lower::lower_program(&context, src)?;
    Ok(module.as_operation().to_string())
}
