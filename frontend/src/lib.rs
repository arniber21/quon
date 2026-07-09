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
pub mod diagnostics;
pub mod lexer;
pub mod parser;
pub mod pretty;

#[cfg(not(feature = "parser-only"))]
pub mod analysis;
#[cfg(not(feature = "parser-only"))]
pub mod desugar;
#[cfg(not(feature = "parser-only"))]
pub mod elaborate;
#[cfg(not(feature = "parser-only"))]
pub mod lower;
#[cfg(not(feature = "parser-only"))]
pub mod refinement;
#[cfg(not(feature = "parser-only"))]
pub mod typecheck;
#[cfg(not(feature = "parser-only"))]
pub mod types;

#[cfg(not(feature = "parser-only"))]
pub use analysis::DocumentAnalysis;
#[cfg(not(feature = "parser-only"))]
pub use analysis::analyze_program;

use crate::ast::Decl;
use crate::diagnostics::Diagnostic;
use crate::lexer::Sp;

#[cfg(not(feature = "parser-only"))]
pub use crate::diagnostics::fixes::apply_fixes;
#[cfg(not(feature = "parser-only"))]
pub use crate::diagnostics::{
    AnalysisResult, DiagnosticCode, DiagnosticSeverity, QuickFix, QuickFixKind, RelatedInfo,
    RichDiagnostic, TextEdit,
};

#[cfg(not(feature = "parser-only"))]
use crate::typecheck::TypeChecker;

/// IDE-oriented analysis: lex → parse → desugar → typecheck. Does not lower to MLIR.
#[cfg(not(feature = "parser-only"))]
pub fn analyze(source: &str) -> AnalysisResult {
    let intelligence = analyze_program(source);
    let tokens = match crate::lexer::lex_rich(source) {
        Ok(t) => t,
        Err(diags) => {
            return AnalysisResult {
                diagnostics: diags,
                intelligence,
            };
        }
    };
    let decls = match crate::parser::parse_rich(&tokens) {
        Ok(d) => d,
        Err(diags) => {
            return AnalysisResult {
                diagnostics: diags,
                intelligence,
            };
        }
    };
    let decls = match crate::desugar::desugar_decls_rich(decls) {
        Ok(d) => d,
        Err(diags) => {
            return AnalysisResult {
                diagnostics: diags,
                intelligence,
            };
        }
    };
    match TypeChecker::new().check_decls(&decls) {
        Ok(()) => AnalysisResult {
            diagnostics: Vec::new(),
            intelligence,
        },
        Err(errs) => AnalysisResult {
            diagnostics: errs.iter().map(|e| e.to_rich_diagnostic(source)).collect(),
            intelligence,
        },
    }
}

/// The frontend's single entry point for turning source text into an AST.
pub fn parse_program(src: &str) -> Result<Vec<Sp<Decl>>, Vec<Diagnostic>> {
    let tokens = lexer::lex(src).map_err(diagnostics::from_stage)?;
    parser::parse(&tokens).map_err(diagnostics::from_stage)
}

/// Parse `src` and run the `run { }` desugaring pass (issue #8).
#[cfg(not(feature = "parser-only"))]
pub fn desugar_program(src: &str) -> Result<Vec<Sp<Decl>>, Vec<Diagnostic>> {
    let decls = parse_program(src)?;
    desugar::desugar_decls(decls)
}

/// Parse, desugar, and type-check a program (issues #9–#14).
#[cfg(not(feature = "parser-only"))]
pub fn check_program(src: &str) -> Result<(), Vec<Diagnostic>> {
    let result = analyze(source);
    if result.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(result.diagnostics.iter().map(Diagnostic::from).collect())
    }
}

/// Parse, desugar, type-check, and lower circuit functions to `quantum.circ` MLIR (issue #16).
#[cfg(not(feature = "parser-only"))]
pub fn lower_program_to_mlir(src: &str) -> Result<String, Vec<Diagnostic>> {
    let context = melior::Context::new();
    let module = lower::lower_program(&context, src)?;
    Ok(module.as_operation().to_string())
}
