//! The circuit fragment: gate primitives and the composition algebra (issue #11, SPEC
//! §3.3, §5.4–§5.8).
//!
//! Gates are *values* of `Circuit<n, m, d, C>` type — `H : Circuit<1,1,1,Clifford>`,
//! `CNOT : Circuit<2,2,1,Clifford>`. Parametric gates are functions that return a circuit
//! (`Rz : Float -> Circuit<1,1,1,C>`). This module is the static data: the gate signatures
//! and their hardcoded Clifford class. The composition *rules* (`|>`, `par`, `adjoint`,
//! `controlled`, `repeat`, gate placement, `for`) live in the checker
//! ([`super::TypeChecker`]); the rotation-angle class refinement lands in issue #12.

use crate::ast::CliffordClass;
use crate::types::Ty;
use quon_core::DepthExpr;

/// A circuit type with literal dimensions and unit depth — the shape of every gate.
fn gate_ty(n: u64, depth: u64, class: CliffordClass) -> Ty {
    Ty::Circuit {
        n: DepthExpr::Nat(n),
        m: DepthExpr::Nat(n),
        d: DepthExpr::Nat(depth),
        c: class,
    }
}

/// The type of a gate primitive `name`, if it is one.
///
/// Non-parametric gates synthesize directly to a `Circuit` value; parametric rotations
/// synthesize to a function `Float -> Circuit<…>` (applied to their angle at the use site).
/// The class here is the gate's *intrinsic* classification; [`rotation_arity`] marks the
/// rotations whose class issue #12 specialises from the static angle.
pub fn gate_type(name: &str) -> Option<Ty> {
    use CliffordClass::{Clifford, Universal};
    let ty = match name {
        // ── §5.4 single-qubit, Clifford ─────────────────────────────────────────
        "I" | "X" | "Y" | "Z" | "H" | "S" | "S_dag" | "SX" | "SX_dag" => gate_ty(1, 1, Clifford),
        // ── §5.4 single-qubit, Universal ────────────────────────────────────────
        "T" | "T_dag" => gate_ty(1, 1, Universal),
        // ── §5.5 two-qubit, Clifford ────────────────────────────────────────────
        "CNOT" | "CX" | "CY" | "CZ" | "SWAP" | "iSWAP" | "ECR" => gate_ty(2, 1, Clifford),
        // ── §5.4/§5.5 parametric rotations: `Float -> Circuit<arity,arity,1,C>` ──
        "Rx" | "Ry" | "Rz" => Ty::func(Ty::Float, gate_ty(1, 1, Universal)),
        "Rzz" | "Rxx" | "Ryy" | "CRz" | "CRx" | "CP" => {
            Ty::func(Ty::Float, gate_ty(2, 1, Universal))
        }
        _ => return None,
    };
    Some(ty)
}

/// For a parametric gate, the qubit arity of the circuit it produces (so the checker can
/// place it), or `None` if `name` is not a parametric gate. Single-qubit rotations have
/// arity 1; two-qubit ones arity 2.
pub fn rotation_arity(name: &str) -> Option<u64> {
    match name {
        "Rx" | "Ry" | "Rz" => Some(1),
        "Rzz" | "Rxx" | "Ryy" | "CRz" | "CRx" | "CP" => Some(2),
        _ => None,
    }
}

/// Whether `name` is a single-qubit rotation whose class issue #12 specialises to `Clifford`
/// at static multiples of `π/2`.
pub fn is_specialisable_rotation(name: &str) -> bool {
    matches!(name, "Rx" | "Ry" | "Rz")
}
