// Type definitions and kind checker — see SPEC.md §3.1–§3.2
// Canonical runtime representations of Quon types used by the type checker.

use crate::ast::{CliffordClass, Name};
use quon_core::DepthExpr;

/// Fully resolved Quon type (post kind-checking).
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Qubit,
    QReg(u64),
    Bit,
    Bool,
    Int,
    Float,
    Unit,
    List(Box<Ty>),
    Tuple(Vec<Ty>),
    Fn(Box<Ty>, Box<Ty>),
    Linear(Box<Ty>, Box<Ty>),
    Circuit {
        n: u64,
        m: u64,
        d: DepthExpr,
        c: CliffordClass,
    },
    Q(Box<Ty>),
    Matrix(u64, u64, Box<Ty>),
    Var(Name),
}

impl Ty {
    pub fn is_linear(&self) -> bool {
        matches!(
            self,
            Ty::Qubit | Ty::QReg(_) | Ty::Circuit { .. } | Ty::Linear(..)
        )
    }
}
