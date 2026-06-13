// Bidirectional type checker with split linear context — see issues #9–#15, SPEC.md §3
//
// Judgment form:  Γ ; Δ ⊢ e : τ
//   Γ  — unrestricted context: HashMap<Name, Ty>
//   Δ  — linear context: HashMap<Name, Ty>, physically removed on use

use std::collections::HashMap;
use crate::ast::{Decl, Expr, Name};
use crate::lexer::Sp;
use crate::types::Ty;

pub type Ctx = HashMap<Name, Ty>;   // unrestricted Γ
pub type LinCtx = HashMap<Name, Ty>; // linear Δ

#[derive(Debug, thiserror::Error)]
pub enum TypeError {
    #[error("{0}: variable `{1}` used twice (no-cloning)")]
    UsedTwice(String, Name),

    #[error("{0}: linear resource `{1}` dropped (must be consumed)")]
    Dropped(String, Name),

    #[error("{0}: type mismatch — expected {1:?}, got {2:?}")]
    Mismatch(String, Ty, Ty),

    #[error("{0}: circuit qubit count mismatch at `|>` — {1} out vs {2} in")]
    QubitMismatch(String, u64, u64),

    #[error("{0}: {1}")]
    Other(String, String),
}

pub struct TypeChecker {
    gamma: Ctx,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self { gamma: Ctx::new() }
    }

    pub fn check_decls(&mut self, _decls: &[Sp<Decl>]) -> Result<(), Vec<TypeError>> {
        todo!("type checker — see issues #9–#15")
    }

    pub fn synth(&mut self, _delta: &mut LinCtx, _expr: &Sp<Expr>) -> Result<Ty, TypeError> {
        todo!()
    }

    pub fn check(&mut self, _delta: &mut LinCtx, _expr: &Sp<Expr>, _ty: &Ty) -> Result<(), TypeError> {
        todo!()
    }
}
