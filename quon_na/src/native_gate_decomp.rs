//! NA-path native-gate decomposition for single-qubit gates (issue #298).
//!
//! Mirrors `mlir_bridge::passes::native_gate_decomp`'s fixed-target pass, but
//! targets [`backend::target::NeutralAtomTarget::native_gates`] and produces
//! [`crate::schedule::NeutralAtomAction`]s (local `rz`, global `ry`, or a
//! `u3` escape hatch) instead of rewriting MLIR `quantum.circ.gate` ops.
//!
//! Reuses `backend::decompose`'s ZYZ math wholesale — no new decomposition
//! logic here — so results agree with the fixed-target decomposer for gate
//! names both paths recognize (e.g. `rz`), and a target whose native set
//! includes `ry` always gets the same `Rz(gamma) · Ry(beta) · Rz(alpha)`
//! Euler form mqt-qmap's zoned compiler uses for its global-`ry` raster.
//!
//! Any decomposed step whose name isn't one of `h`/`rz`/`ry` (the kinds
//! [`crate::schedule::LocalGateKind`] models today, plus the whole-plane `ry`
//! raster) — e.g. a target that natively lists `sx`/`rx`/`x` — falls through
//! to the `u3(theta, phi, lambda)` escape hatch computed from the gate's own
//! unitary, with a warning to stderr. This mirrors qmap's
//! `warnUnsupportedGates` policy: never silently drop a gate.

use backend::decompose::decompose_named_single;
use backend::unitary::{gate_unitary, rotation_unitary, zyz_angles};
use thiserror::Error;

use crate::extract::LocalGateExtract;
use crate::graph::{InteractionId, LogicalQubitId};
use crate::schedule::LocalGateKind;

/// Matches `backend::decompose`'s private `EPS`; used only to distinguish a
/// legitimate zero-gate decomposition (e.g. the identity) from an
/// unsupported gate for which `decompose_single_qubit` returned no ops
/// because the target's native set matches none of its known combinations.
const EPS: f64 = 1e-8;

/// One decomposed step, ready to become a schedule action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NaLocalOp {
    /// Per-atom local gate.
    Local(LocalGateKind),
    /// Whole-plane `ry(theta)` raster.
    GlobalRy(f64),
}

/// A captured 1-qubit gate, decomposed to `native_gates` and still carrying
/// its extraction-time schedule anchor.
#[derive(Clone, Debug, PartialEq)]
pub struct DecomposedLocalGate {
    pub qubit: LogicalQubitId,
    pub after: Option<InteractionId>,
    pub ops: Vec<NaLocalOp>,
}

/// Errors decomposing a captured 1-qubit gate.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum NaDecompError {
    #[error("no decomposition for single-qubit gate `{0}` (unrecognized name/angle)")]
    Unrecognized(String),
}

/// Decompose every captured 1-qubit gate to `native_gates`, preserving order.
pub fn decompose_local_gates(
    extracted: &[LocalGateExtract],
    native_gates: &[String],
) -> Result<Vec<DecomposedLocalGate>, NaDecompError> {
    extracted
        .iter()
        .map(|gate| decompose_one(gate, native_gates))
        .collect()
}

fn decompose_one(
    gate: &LocalGateExtract,
    native_gates: &[String],
) -> Result<DecomposedLocalGate, NaDecompError> {
    let Some(named_ops) = decompose_named_single(&gate.gate_name, gate.angle, native_gates, 0)
    else {
        return Err(NaDecompError::Unrecognized(gate.gate_name.clone()));
    };

    if named_ops.is_empty() {
        // Ambiguous from `decompose_named_single`'s return value alone: this
        // is either a legitimate zero-gate decomposition (identity, or a
        // diagonal rotation whose total angle is ~0), or — only reachable
        // when `native_gates` has none of the basis combinations
        // `decompose_single_qubit` knows (`rz` alone, `rz`+`sx`, `rz`+`rx`,
        // or `ry`; not the case for `generic_rna_v0.json`, which lists
        // `ry`) — a real gate with no realizable native form. Recompute the
        // gate's own unitary to tell the two apart.
        let matrix = gate
            .angle
            .and_then(|a| rotation_unitary(&gate.gate_name, a))
            .or_else(|| gate_unitary(&gate.gate_name));
        let Some(u) = matrix else {
            return Err(NaDecompError::Unrecognized(gate.gate_name.clone()));
        };
        let (alpha, beta, gamma) = zyz_angles(u);
        if beta.abs() < EPS && (alpha + gamma).abs() < EPS {
            return Ok(DecomposedLocalGate {
                qubit: gate.qubit,
                after: gate.after,
                ops: Vec::new(),
            });
        }
        warn_unsupported(&gate.gate_name, native_gates, "no rz/ry/rx/sx realization");
        return Ok(DecomposedLocalGate {
            qubit: gate.qubit,
            after: gate.after,
            ops: vec![NaLocalOp::Local(LocalGateKind::U3 {
                theta: beta,
                phi: alpha,
                lambda: gamma,
            })],
        });
    }

    let mut ops = Vec::with_capacity(named_ops.len());
    for op in named_ops {
        match op.name.as_str() {
            "rz" => ops.push(NaLocalOp::Local(LocalGateKind::Rz(
                op.params.first().copied().unwrap_or(0.0),
            ))),
            "ry" => ops.push(NaLocalOp::GlobalRy(
                op.params.first().copied().unwrap_or(0.0),
            )),
            "h" => ops.push(NaLocalOp::Local(LocalGateKind::H)),
            other => {
                // `decompose_single_qubit`'s sx/rx/x combinations, or a
                // native-passthrough of a gate literally named e.g. `t`:
                // none of these have a `LocalGateKind` today (issue #298
                // scope is h/rz/ry/u3). Escape the *whole* gate to u3 rather
                // than emit a step type the schedule can't represent.
                warn_unsupported(
                    &gate.gate_name,
                    native_gates,
                    &format!("decomposed step `{other}` has no LocalGateKind representation"),
                );
                let matrix = gate
                    .angle
                    .and_then(|a| rotation_unitary(&gate.gate_name, a))
                    .or_else(|| gate_unitary(&gate.gate_name));
                let Some(u) = matrix else {
                    return Err(NaDecompError::Unrecognized(gate.gate_name.clone()));
                };
                let (alpha, beta, gamma) = zyz_angles(u);
                return Ok(DecomposedLocalGate {
                    qubit: gate.qubit,
                    after: gate.after,
                    ops: vec![NaLocalOp::Local(LocalGateKind::U3 {
                        theta: beta,
                        phi: alpha,
                        lambda: gamma,
                    })],
                });
            }
        }
    }

    Ok(DecomposedLocalGate {
        qubit: gate.qubit,
        after: gate.after,
        ops,
    })
}

fn warn_unsupported(gate_name: &str, native_gates: &[String], reason: &str) {
    eprintln!(
        "quon_na: native-gate-decomp: gate `{gate_name}` on native set {native_gates:?}: \
         {reason}; using u3(theta, phi, lambda) escape hatch"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const NA_NATIVE: &[&str] = &["cz", "rz", "rx", "ry", "measure_z", "reset"];

    fn native_gates() -> Vec<String> {
        NA_NATIVE.iter().map(|s| s.to_string()).collect()
    }

    fn extract(qubit: u32, gate_name: &str, angle: Option<f64>) -> LocalGateExtract {
        LocalGateExtract {
            qubit: LogicalQubitId(qubit),
            gate_name: gate_name.to_string(),
            angle,
            after: None,
        }
    }

    #[test]
    fn rz_passes_through_as_local_rz() {
        let decomposed = decompose_one(&extract(0, "rz", Some(1.25)), &native_gates()).unwrap();
        assert_eq!(
            decomposed.ops,
            vec![NaLocalOp::Local(LocalGateKind::Rz(1.25))]
        );
    }

    #[test]
    fn h_decomposes_to_rz_ry_rz_via_zyz() {
        let decomposed = decompose_one(&extract(0, "H", None), &native_gates()).unwrap();
        assert!(
            decomposed
                .ops
                .iter()
                .any(|op| matches!(op, NaLocalOp::GlobalRy(_)))
        );
        assert!(
            decomposed
                .ops
                .iter()
                .any(|op| matches!(op, NaLocalOp::Local(LocalGateKind::Rz(_))))
        );
    }

    #[test]
    fn identity_decomposes_to_zero_ops() {
        let decomposed = decompose_one(&extract(0, "I", None), &native_gates()).unwrap();
        assert!(decomposed.ops.is_empty());
    }

    #[test]
    fn unrecognized_gate_errors() {
        let err = decompose_one(&extract(0, "not_a_real_gate", None), &native_gates());
        assert!(err.is_err());
    }

    #[test]
    fn ry_target_gate_becomes_global_raster() {
        let decomposed = decompose_one(&extract(0, "ry", Some(0.5)), &native_gates()).unwrap();
        assert_eq!(decomposed.ops, vec![NaLocalOp::GlobalRy(0.5)]);
    }

    #[test]
    fn unsupported_native_set_falls_back_to_u3() {
        // A native set with none of rz-alone/rz+sx/rz+rx/ry: any non-trivial
        // gate has no decomposition and must escape to u3.
        let native: Vec<String> = vec!["cz".to_string()];
        let decomposed = decompose_one(&extract(0, "H", None), &native).unwrap();
        assert_eq!(decomposed.ops.len(), 1);
        assert!(matches!(
            decomposed.ops[0],
            NaLocalOp::Local(LocalGateKind::U3 { .. })
        ));
    }
}
