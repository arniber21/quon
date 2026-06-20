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
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod refinement;
pub mod typecheck;
pub mod types;
