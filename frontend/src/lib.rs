//! Quon frontend — lexer, parser, type checker, and AST→IR lowering.
//!
//! Stub implementations in this crate are expanded in issues #5–#16.

#![allow(
    dead_code,
    clippy::new_without_default,
    clippy::large_enum_variant,
    // `TypeError` carries resolved `Ty`s (with symbolic `Circuit`/`QReg` dimensions) for
    // span-accurate diagnostics, so `Result<_, TypeError>` has a large `Err` variant. The
    // error path is cold; boxing every `?` site would add churn for no real benefit.
    clippy::result_large_err,
    clippy::arc_with_non_send_sync // Z3 Context is !Send; Arc retained per PRD
)]

pub mod ast;
pub mod desugar;
pub mod diagnostics;
pub mod elaborate;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod pretty;
pub mod refinement;
pub mod typecheck;
pub mod types;

use crate::ast::Decl;
use crate::diagnostics::Diagnostic;
use crate::lexer::Sp;
use crate::typecheck::TypeChecker;

pub use crate::diagnostics::fixes::apply_fixes;
pub use crate::diagnostics::{
    AnalysisResult, DiagnosticCode, DiagnosticSeverity, QuickFix, QuickFixKind, RelatedInfo,
    RichDiagnostic, TextEdit,
};

/// IDE-oriented analysis: lex → parse → desugar → typecheck. Does not lower to MLIR.
/// Accumulates errors from the first failing stage; never panics on partial source.
pub fn analyze(source: &str) -> AnalysisResult {
    let tokens = match crate::lexer::lex_rich(source) {
        Ok(t) => t,
        Err(diags) => return AnalysisResult { diagnostics: diags },
    };
    let decls = match crate::parser::parse_rich(&tokens) {
        Ok(d) => d,
        Err(diags) => return AnalysisResult { diagnostics: diags },
    };
    let decls = match crate::desugar::desugar_decls_rich(decls) {
        Ok(d) => d,
        Err(diags) => return AnalysisResult { diagnostics: diags },
    };
    match TypeChecker::new().check_decls(&decls) {
        Ok(()) => AnalysisResult {
            diagnostics: Vec::new(),
        },
        Err(errs) => AnalysisResult {
            diagnostics: errs.iter().map(|e| e.to_rich_diagnostic(source)).collect(),
        },
    }
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

/// Parse `src` and run the `run { }` desugaring pass (issue #8), folding lexer,
/// parser, and desugaring errors into one [`Diagnostic`] stream. Desugaring runs
/// *before* the type checker, so the declarations this returns have every `run`
/// block already lowered to `Bind`/`Return` nodes — this is the seam the checker
/// and later lowering stages build on.
pub fn desugar_program(src: &str) -> Result<Vec<Sp<Decl>>, Vec<Diagnostic>> {
    let decls = parse_program(src)?;
    desugar::desugar_decls(decls)
}

/// Parse, desugar, and type-check a program (issues #9–#14). `run { }` blocks are lowered to
/// `Bind`/`Return` nodes by [`desugar_program`] *before* the checker runs, so the quantum
/// monad fragment is type-checked on the desugared tree. Lexer, parser, desugaring, and type
/// errors are folded into the one [`Diagnostic`] stream.
pub fn check_program(src: &str) -> Result<(), Vec<Diagnostic>> {
    let result = analyze(src);
    if result.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(result.diagnostics.iter().map(Diagnostic::from).collect())
    }
}

/// Parse, desugar, type-check, and lower circuit functions to `quantum.circ` MLIR (issue #16).
pub fn lower_program_to_mlir(src: &str) -> Result<String, Vec<Diagnostic>> {
    let context = melior::Context::new();
    let module = lower::lower_program(&context, src)?;
    Ok(module.as_operation().to_string())
}
