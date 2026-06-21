//! Quon frontend — lexer, parser, type checker, and AST→IR lowering.
//!
//! Stub implementations in this crate are expanded in issues #5–#16.

#![allow(
    dead_code,
    clippy::new_without_default,
    clippy::large_enum_variant,
    clippy::arc_with_non_send_sync // Z3 Context is !Send; Arc retained per PRD
)]

pub mod ast;
pub mod desugar;
pub mod diagnostics;
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
