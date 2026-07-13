//! The circuit fragment: gate primitives and the composition algebra (issue #11, SPEC
//! §3.3, §5.4–§5.8).
//!
//! Gates are *values* of `Circuit<n, m, d, C>` type — `H : Circuit<1,1,1,Clifford>`,
//! `CNOT : Circuit<2,2,1,Clifford>`. Parametric gates are functions that return a circuit
//! (`Rz : Float -> Circuit<1,1,1,C>`). This module is the static data: the gate signatures
//! and their hardcoded Clifford class. The composition *rules* (`|>`, `par`, `adjoint`,
//! `controlled`, `repeat`, gate placement, `for`) live in the checker
//! ([`super::TypeChecker`]); the rotation-angle class refinement lands in issue #12.
//!
//! Gate metadata (arity, class, aliases) comes from [`quon_core::gates`] — the single
//! registry shared with backend / cancellation / emit (issue #209).

use crate::ast::CliffordClass;
use crate::types::Ty;
use quon_core::DepthExpr;
use quon_core::gates::{GateClass, surface_gate};

/// A circuit type with literal dimensions and unit depth — the shape of every gate.
fn gate_ty(n: u64, depth: u64, class: CliffordClass) -> Ty {
    Ty::Circuit {
        n: DepthExpr::Nat(n),
        m: DepthExpr::Nat(n),
        d: DepthExpr::Nat(depth),
        c: class,
    }
}

fn to_clifford_class(class: GateClass) -> CliffordClass {
    match class {
        GateClass::Clifford => CliffordClass::Clifford,
        GateClass::Universal => CliffordClass::Universal,
    }
}

/// The type of a gate primitive `name`, if it is one.
///
/// Non-parametric gates synthesize directly to a `Circuit` value; parametric rotations
/// synthesize to a function `Float -> Circuit<…>` (applied to their angle at the use site).
/// The class here is the gate's *intrinsic* classification; [`rotation_arity`] marks the
/// rotations whose class issue #12 specialises from the static angle.
pub fn gate_type(name: &str) -> Option<Ty> {
    let info = surface_gate(name)?;
    let class = to_clifford_class(info.class);
    let arity = info.arity as u64;
    if info.parametric {
        Some(Ty::func(Ty::Float, gate_ty(arity, 1, class)))
    } else {
        Some(gate_ty(arity, 1, class))
    }
}

/// For a parametric gate, the qubit arity of the circuit it produces (so the checker can
/// place it), or `None` if `name` is not a parametric gate. Single-qubit rotations have
/// arity 1; two-qubit ones arity 2.
pub fn rotation_arity(name: &str) -> Option<u64> {
    surface_gate(name)
        .filter(|g| g.parametric)
        .map(|g| g.arity as u64)
}

/// Whether `name` is a single-qubit rotation whose class issue #12 specialises to `Clifford`
/// at static multiples of `π/2`.
pub fn is_specialisable_rotation(name: &str) -> bool {
    surface_gate(name)
        .is_some_and(|g| g.parametric && g.arity == 1 && matches!(g.id, "Rx" | "Ry" | "Rz"))
}
