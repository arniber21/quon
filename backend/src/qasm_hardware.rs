//! Hardware-constraint check for emitted OpenQASM (issue #499, #82 phase G2b).
//!
//! [`validate_hardware_qasm`] inspects a [`quon_core::qasm::Program`] against a
//! fixed [`FixedTarget`] after reification and before text is printed:
//!
//! - every gate keyword is in the target native set
//! - every two-qubit gate sits on one undirected coupling edge
//! - user-defined `gate` blocks are rejected (they are not native hardware ops)
//! - an `if` requires `supports_feed_forward`
//! - a measure that is not in the trailing end-of-circuit measure run requires
//!   `supports_mid_circuit_meas`
//!
//! Three-qubit gates are checked for native-set membership and in-range qubits
//! only. Coupling is a two-qubit constraint; a native three-qubit gate is not
//! rewritten into edges here.

use quon_core::qasm::{Program, QasmGate, Stmt};
use thiserror::Error;

use crate::target::FixedTarget;

/// A reified OpenQASM program that the fixed target cannot execute.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HardwareQasmError {
    /// The gate's OpenQASM keyword is not in `native_gates`.
    #[error("gate `{keyword}` is not in the target native set")]
    NonNativeGate { keyword: String },

    /// A two-qubit gate's operands are not one coupling-map edge.
    #[error("two-qubit gate `{keyword}` on q[{a}], q[{b}] is not a coupling-map edge")]
    OffCoupling { keyword: String, a: usize, b: usize },

    /// A qubit index is outside the target's physical register.
    #[error("qubit q[{index}] is outside the target ({num_qubits} qubits)")]
    QubitOutOfRange { index: usize, num_qubits: usize },

    /// A user-defined gate declaration is not a native hardware operation.
    #[error("user-defined gate `{name}` is not a native hardware gate")]
    CustomGate { name: String },

    /// The program contains feed-forward `if` and the target disallows it.
    #[error("feed-forward `if` is not supported by this target (`supports_feed_forward` is false)")]
    FeedForwardDisabled,

    /// A measure is not an end-of-circuit measure and the target disallows that.
    #[error(
        "mid-circuit measurement is not supported by this target (`supports_mid_circuit_meas` is false)"
    )]
    MidCircuitMeasure,
}

/// Check `program` against `target`.
///
/// End-of-circuit measurement is the longest trailing run of top-level
/// `measure` statements. Measures inside an `if`, or any measure before a
/// later gate, reset, barrier, or `if`, are mid-circuit.
pub fn validate_hardware_qasm(
    program: &Program,
    target: &FixedTarget,
) -> Result<(), HardwareQasmError> {
    if let Some(def) = program.gate_defs().first() {
        return Err(HardwareQasmError::CustomGate {
            name: def.name.clone(),
        });
    }
    check_stmts(program.body(), target, false)
}

fn check_stmts(
    stmts: &[Stmt],
    target: &FixedTarget,
    in_branch: bool,
) -> Result<(), HardwareQasmError> {
    let terminal_from = if in_branch {
        stmts.len()
    } else {
        trailing_measure_start(stmts)
    };
    for (index, stmt) in stmts.iter().enumerate() {
        match stmt {
            Stmt::Gate(gate) => check_gate(*gate, target)?,
            Stmt::Measure { qubit, .. } => {
                check_qubit(qubit.index(), target)?;
                if index < terminal_from && !target.supports_mid_circuit_meas {
                    return Err(HardwareQasmError::MidCircuitMeasure);
                }
            }
            Stmt::Reset(qubit) => check_qubit(qubit.index(), target)?,
            Stmt::Barrier(qubits) => {
                for qubit in qubits {
                    check_qubit(qubit.index(), target)?;
                }
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                if !target.supports_feed_forward {
                    return Err(HardwareQasmError::FeedForwardDisabled);
                }
                check_stmts(then_body, target, true)?;
                check_stmts(else_body, target, true)?;
            }
        }
    }
    Ok(())
}

/// Index of the first statement in the trailing top-level measure run.
fn trailing_measure_start(stmts: &[Stmt]) -> usize {
    let mut index = stmts.len();
    while index > 0 && matches!(stmts[index - 1], Stmt::Measure { .. }) {
        index -= 1;
    }
    index
}

fn check_gate(gate: QasmGate, target: &FixedTarget) -> Result<(), HardwareQasmError> {
    let keyword = gate.keyword();
    if !target.is_native(keyword) {
        return Err(HardwareQasmError::NonNativeGate {
            keyword: keyword.to_string(),
        });
    }
    let qubits = gate_qubits(gate);
    for qubit in &qubits {
        check_qubit(qubit.index(), target)?;
    }
    if let [a, b] = qubits.as_slice() {
        let a = a.index();
        let b = b.index();
        if !target.topology.is_edge(a, b) {
            return Err(HardwareQasmError::OffCoupling {
                keyword: keyword.to_string(),
                a,
                b,
            });
        }
    }
    Ok(())
}

fn check_qubit(index: usize, target: &FixedTarget) -> Result<(), HardwareQasmError> {
    if index >= target.num_qubits {
        Err(HardwareQasmError::QubitOutOfRange {
            index,
            num_qubits: target.num_qubits,
        })
    } else {
        Ok(())
    }
}

fn gate_qubits(gate: QasmGate) -> Vec<quon_core::qasm::QubitId> {
    match gate {
        QasmGate::One(_, q)
        | QasmGate::Rotation(_, _, q)
        | QasmGate::U2 { q, .. }
        | QasmGate::U3 { q, .. }
        | QasmGate::Std1 { q, .. } => vec![q],
        QasmGate::Two(_, a, b) | QasmGate::Std2 { a, b, .. } => vec![a, b],
        QasmGate::Ccx(a, b, c) | QasmGate::Std3 { a, b, c, .. } => vec![a, b, c],
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use quon_core::qasm::{
        Expr, GateDef, OneQubitGate, Program, QasmGate, RotationGate, TwoQubitGate,
    };

    use proptest::prelude::*;

    use super::*;
    use crate::json;
    use crate::target::{ConnectivityGraph, FixedTarget, NativeGate, NoiseModel};

    fn line(num_qubits: usize, mid: bool, feed_forward: bool) -> FixedTarget {
        let edges: Vec<(usize, usize)> = (0..num_qubits.saturating_sub(1))
            .map(|i| (i, i + 1))
            .collect();
        let topology = ConnectivityGraph::try_from_edges(num_qubits, edges)
            .expect("line topology is a valid graph");
        FixedTarget::new(
            topology,
            vec![
                NativeGate::passthrough("cx", 2),
                NativeGate::passthrough("x", 1),
                NativeGate::passthrough("rz", 1),
            ],
            NoiseModel::default(),
            0.0,
            mid,
            feed_forward,
        )
    }

    fn cx(program: &Program, a: usize, b: usize) -> QasmGate {
        QasmGate::Two(
            TwoQubitGate::Cx,
            program.qubit(a).expect("qubit in program"),
            program.qubit(b).expect("qubit in program"),
        )
    }

    #[test]
    fn adjacent_cx_is_legal() {
        let target = line(4, false, false);
        let mut program = Program::new(4, 0);
        program.push_gate(cx(&program, 1, 2)).expect("cx statement");
        validate_hardware_qasm(&program, &target).expect("adjacent cx");
    }

    #[test]
    fn skipped_edge_cx_is_rejected() {
        let target = line(4, false, false);
        let mut program = Program::new(4, 0);
        program.push_gate(cx(&program, 0, 2)).expect("cx statement");
        let err = validate_hardware_qasm(&program, &target).expect_err("off-edge cx");
        assert_eq!(
            err,
            HardwareQasmError::OffCoupling {
                keyword: "cx".to_string(),
                a: 0,
                b: 2,
            }
        );
    }

    #[test]
    fn non_native_keyword_is_rejected() {
        let target = line(2, false, false);
        let mut program = Program::new(2, 0);
        let q = program.qubit(0).expect("qubit");
        program
            .push_gate(QasmGate::One(OneQubitGate::H, q))
            .expect("h statement");
        let err = validate_hardware_qasm(&program, &target).expect_err("h is not native");
        assert_eq!(
            err,
            HardwareQasmError::NonNativeGate {
                keyword: "h".to_string(),
            }
        );
    }

    #[test]
    fn terminal_measures_are_legal_without_mid_circuit_support() {
        let target = line(2, false, false);
        let mut program = Program::new(2, 2);
        program.push_gate(cx(&program, 0, 1)).expect("cx statement");
        let q0 = program.qubit(0).expect("q0");
        let q1 = program.qubit(1).expect("q1");
        let c0 = program.bit(0).expect("c0");
        let c1 = program.bit(1).expect("c1");
        program.push_measure(q0, c0).expect("measure");
        program.push_measure(q1, c1).expect("measure");
        validate_hardware_qasm(&program, &target).expect("terminal measures");
    }

    #[test]
    fn measure_followed_by_a_gate_is_mid_circuit() {
        let target = line(2, false, false);
        let mut program = Program::new(2, 1);
        let q0 = program.qubit(0).expect("q0");
        let c0 = program.bit(0).expect("c0");
        program.push_measure(q0, c0).expect("measure");
        program
            .push_gate(QasmGate::One(
                OneQubitGate::X,
                program.qubit(1).expect("q1"),
            ))
            .expect("x statement");
        let err = validate_hardware_qasm(&program, &target).expect_err("mid-circuit");
        assert_eq!(err, HardwareQasmError::MidCircuitMeasure);
    }

    #[test]
    fn feed_forward_requires_the_capability_flag() {
        let restricted = line(2, true, false);
        let mut program = Program::new(2, 1);
        let bit = program.bit(0).expect("bit");
        let q = program.qubit(0).expect("qubit");
        program
            .push_if(
                Expr::bit_is_set(bit),
                vec![quon_core::qasm::Stmt::Gate(QasmGate::One(
                    OneQubitGate::X,
                    q,
                ))],
                Vec::new(),
            )
            .expect("if statement");
        let err = validate_hardware_qasm(&program, &restricted).expect_err("no feed-forward");
        assert_eq!(err, HardwareQasmError::FeedForwardDisabled);

        let allowed = line(2, true, true);
        validate_hardware_qasm(&program, &allowed).expect("feed-forward allowed");
    }

    #[test]
    fn measure_inside_a_branch_is_mid_circuit() {
        let target = line(1, false, true);
        let mut program = Program::new(1, 1);
        let bit = program.bit(0).expect("bit");
        let q = program.qubit(0).expect("qubit");
        program
            .push_if(
                Expr::bit_is_set(bit),
                vec![quon_core::qasm::Stmt::Measure { qubit: q, bit }],
                Vec::new(),
            )
            .expect("if statement");
        let err = validate_hardware_qasm(&program, &target).expect_err("conditional measure");
        assert_eq!(err, HardwareQasmError::MidCircuitMeasure);
    }

    #[test]
    fn custom_gate_def_is_rejected() {
        let target = line(1, false, false);
        let mut program = Program::new(1, 0);
        program.push_gate_def(GateDef {
            name: "my_h".to_string(),
            params: vec!["a".to_string()],
            body: "h a;".to_string(),
        });
        let err = validate_hardware_qasm(&program, &target).expect_err("custom gate");
        assert_eq!(
            err,
            HardwareQasmError::CustomGate {
                name: "my_h".to_string(),
            }
        );
    }

    #[test]
    fn qubit_past_the_device_is_rejected() {
        let target = line(2, false, false);
        let mut program = Program::new(3, 0);
        program
            .push_gate(QasmGate::Rotation(
                RotationGate::Rz,
                0.1,
                program.qubit(2).expect("q2"),
            ))
            .expect("rz statement");
        let err = validate_hardware_qasm(&program, &target).expect_err("out of range");
        assert_eq!(
            err,
            HardwareQasmError::QubitOutOfRange {
                index: 2,
                num_qubits: 2,
            }
        );
    }

    #[test]
    fn fake_manila_snapshot_accepts_adjacent_native_gates() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../targets/ibm/fake_manila_v2.json");
        let loaded = json::load(&path).expect("manila snapshot");
        let target = loaded.fixed_target().expect("fixed target");
        let mut program = Program::new(5, 1);
        program.push_gate(cx(&program, 0, 1)).expect("cx");
        program
            .push_gate(QasmGate::Rotation(
                RotationGate::Rz,
                0.25,
                program.qubit(2).expect("q2"),
            ))
            .expect("rz");
        program
            .push_gate(QasmGate::Std1 {
                keyword: "sx",
                angle: None,
                q: program.qubit(4).expect("q4"),
            })
            .expect("sx");
        program
            .push_gate(QasmGate::One(
                OneQubitGate::X,
                program.qubit(3).expect("q3"),
            ))
            .expect("x");
        program
            .push_measure(program.qubit(0).expect("q0"), program.bit(0).expect("c0"))
            .expect("measure");
        validate_hardware_qasm(&program, target).expect("manila-legal program");

        let mut skipped = Program::new(5, 0);
        skipped.push_gate(cx(&skipped, 0, 2)).expect("cx");
        let err = validate_hardware_qasm(&skipped, target).expect_err("manila non-edge");
        assert_eq!(
            err,
            HardwareQasmError::OffCoupling {
                keyword: "cx".to_string(),
                a: 0,
                b: 2,
            }
        );
    }

    proptest! {
        #[test]
        fn line_cx_agrees_with_adjacency(n in 2usize..8, a in 0usize..8, b in 0usize..8) {
            let a = a % n;
            let b = b % n;
            prop_assume!(a != b);
            let target = line(n, true, true);
            let mut program = Program::new(n, 0);
            program.push_gate(cx(&program, a, b)).expect("cx statement");
            let result = validate_hardware_qasm(&program, &target);
            if target.topology.is_edge(a, b) {
                prop_assert!(result.is_ok(), "expected Ok, got {result:?}");
            } else {
                let off_coupling = matches!(result, Err(HardwareQasmError::OffCoupling { .. }));
                prop_assert!(off_coupling, "expected OffCoupling, got {result:?}");
            }
        }
    }
}
