//! AST → `quantum.circ` lowering tests (issue #16).

use melior::Context;
use melior::ir::BlockLike;
use melior::ir::operation::OperationLike;
use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::linearity_verifier::check_linearity;

use frontend::lower::lower_program;

fn lower_text(src: &str) -> String {
    let context = Context::new();
    let module = lower_program(&context, src).expect("lowering should succeed");
    module.as_operation().to_string()
}

fn module_linearity_ok(module: &melior::ir::Module<'_>) -> bool {
    let body = module.body();
    let mut op = body.first_operation();
    while let Some(current) = op {
        let op_name = current
            .name()
            .as_string_ref()
            .as_str()
            .unwrap_or("")
            .to_string();
        if op_name == qc::op::FUNC && !check_linearity(&current).is_empty() {
            return false;
        }
        op = current.next_in_block();
    }
    true
}

#[test]
fn bell_state_lowers_to_quantum_circ_func() {
    let src = include_str!("fixtures/bell_state.qn");
    let text = lower_text(src);
    assert!(
        text.contains(r#"sym_name = "bell_state""#),
        "missing bell_state func: {text}"
    );
    assert!(text.contains("in_qubits = 2"));
    assert!(text.contains("out_qubits = 2"));
    assert!(text.contains(r#"depth = "2""#));
    assert!(text.contains("clifford = true"));
    assert!(text.contains(r#"gate_name = "H""#));
    assert!(text.contains(r#"gate_name = "CNOT""#));
}

#[test]
fn gate_ops_carry_depth_and_clifford_attributes() {
    let src = "fn f(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }";
    let text = lower_text(src);
    assert!(text.contains("depth_contribution = 1"));
    assert!(text.contains("clifford = true"));
    assert!(text.contains(r#"gate_name = "H""#));
}

#[test]
fn adjoint_of_circuit_call_emits_adjoint_op() {
    let src = "\
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }
fn adjoint_bell(): Circuit<2, 2, 2, Clifford> = adjoint(bell_state())
";
    let text = lower_text(src);
    assert!(text.contains("quantum.circ.adjoint"));
    assert!(text.contains(r#"sym_name = "bell_state""#));
}

#[test]
fn lowered_bell_state_passes_linearity_verifier() {
    let src = "fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }";
    let context = Context::new();
    let module = lower_program(&context, src).expect("lower bell_state");
    assert!(module_linearity_ok(&module), "bell_state should be linear");
}

#[test]
fn bell_state_fixture_passes_linearity_verifier() {
    let src = include_str!("fixtures/bell_state.qn");
    let context = Context::new();
    let module = lower_program(&context, src).expect("lower bell_state fixture");
    assert!(
        module_linearity_ok(&module),
        "bell_state fixture should be linear"
    );
}

/// CNOT operand pairs in a lowered module, as qubit indices.
///
/// Each `quantum.circ.gate` is wire-preserving, so an SSA value stays on the
/// block argument it was derived from. A second arm dropped, or both arms
/// left on the same wire, changes this list.
fn cnot_qubit_pairs(mlir: &str) -> Vec<(usize, usize)> {
    let mut qubits: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut pairs = Vec::new();
    for line in mlir.lines() {
        let line = line.trim();
        if let Some(args) = line.strip_prefix("^bb0(") {
            qubits.clear();
            let mut index = 0usize;
            for part in args.split(',') {
                let name = part
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches(')');
                if let Some(rest) = name.strip_prefix("%arg") {
                    let arg_index: usize = rest
                        .parse()
                        .unwrap_or_else(|_| panic!("bad block arg {name} in {line}"));
                    assert_eq!(arg_index, index, "block args are not dense: {line}");
                    qubits.insert(name.to_string(), index);
                    index += 1;
                }
            }
            continue;
        }
        if !line.contains("\"quantum.circ.gate\"") {
            continue;
        }
        let (result, rest) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("gate has no result: {line}"));
        let open = rest
            .find('(')
            .unwrap_or_else(|| panic!("gate has no operands: {line}"));
        let close = rest
            .find(')')
            .unwrap_or_else(|| panic!("gate operands are unclosed: {line}"));
        let operands: Vec<&str> = rest[open + 1..close]
            .split(',')
            .map(str::trim)
            .filter(|operand| !operand.is_empty())
            .collect();
        let gate_name = rest
            .split("gate_name = \"")
            .nth(1)
            .and_then(|tail| tail.split('"').next())
            .unwrap_or_else(|| panic!("gate has no name: {line}"));
        let operand_qubits: Vec<usize> = operands
            .iter()
            .map(|operand| {
                *qubits
                    .get(*operand)
                    .unwrap_or_else(|| panic!("unknown SSA {operand} in {line}\n{mlir}"))
            })
            .collect();
        if gate_name == "CNOT" {
            assert_eq!(operand_qubits.len(), 2, "CNOT arity in {line}");
            pairs.push((operand_qubits[0], operand_qubits[1]));
        }
        let result_names = expand_gate_results(result.trim());
        assert_eq!(
            result_names.len(),
            operand_qubits.len(),
            "result/operand arity in {line}"
        );
        for (name, qubit) in result_names.into_iter().zip(operand_qubits) {
            qubits.insert(name, qubit);
        }
    }
    pairs
}

/// `%0` is one result; `%1:2` is `%1#0` and `%1#1`.
fn expand_gate_results(result: &str) -> Vec<String> {
    let Some((name, count)) = result.split_once(':') else {
        return vec![result.to_string()];
    };
    let count: usize = count
        .parse()
        .unwrap_or_else(|_| panic!("bad result count in {result}"));
    (0..count).map(|i| format!("{name}#{i}")).collect()
}

#[test]
fn controlled_par_nested_targets_lower() {
    // Surface spellings from the typechecker contract. Each controlled-H
    // contributes one CNOT; the pairs are (control, arm target).
    let programs = [
        (
            "nested",
            "fn ctrl_layer(): Circuit<3, 3, 2, Clifford> = circuit {\n    \
             (controlled(par { H @0, H @0 })) @(0, (1, 2))\n}",
        ),
        (
            "flat",
            "fn ctrl_layer(): Circuit<3, 3, 2, Clifford> = circuit {\n    \
             controlled(par { H @0, H @0 }) @(0, 1, 2)\n}",
        ),
        (
            "start",
            "fn ctrl_layer(): Circuit<3, 3, 2, Clifford> = circuit {\n    \
             controlled(par { H @0 } * 2) @(0, 1)\n}",
        ),
    ];
    for (label, src) in programs {
        let text = lower_text(src);
        assert!(
            text.contains(r#"sym_name = "ctrl_layer""#),
            "{label} missing ctrl_layer: {text}"
        );
        assert!(
            text.contains("in_qubits = 3") && text.contains("out_qubits = 3"),
            "{label} expected a 3-qubit controlled par: {text}"
        );
        assert_eq!(
            cnot_qubit_pairs(&text),
            vec![(0, 1), (0, 2)],
            "{label} CNOT pairs drifted: {text}"
        );
    }
}

#[test]
fn controlled_par_multiqubit_arm_does_not_lower() {
    // Rejected at typecheck with the same diagnostic `controlled_named_gate`
    // uses, so lowering never sees the multi-qubit arm.
    let src = "\
fn f(): Circuit<3, 3, 2, Clifford> = circuit {
    controlled(par { CNOT @(0, 1) }) @(0, (1, 2))
}
";
    let context = Context::new();
    let err = lower_program(&context, src).expect_err("multi-qubit arm should not lower");
    let text = err
        .iter()
        .map(|diag| diag.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("controlled() of a multi-qubit gate"),
        "expected the controlled_named_gate diagnostic, got {text}"
    );
}

#[test]
fn controlled_h_and_compose_lower() {
    // Issue #182: controlled(H) and controlled(H |> T) must elaborate under a
    // generic target (emit path goes through the same lower → circ → QASM).
    let src = "\
fn ch(): Circuit<2, 2, 2, Universal> = circuit { controlled(H) @(0, 1) }
fn cht(): Circuit<2, 2, 3, Universal> = circuit { controlled(H |> T) @(0, 1) }
fn cht_block(): Circuit<2, 2, 3, Universal> = circuit { controlled(circuit { H |> T }) @(0, 1) }
";
    let text = lower_text(src);
    assert!(text.contains(r#"sym_name = "ch""#), "missing ch: {text}");
    assert!(
        text.contains(r#"sym_name = "cht""#) && text.contains(r#"sym_name = "cht_block""#),
        "missing cht forms: {text}"
    );
    assert!(
        text.contains(r#"gate_name = "CNOT""#) || text.contains(r#"gate_name = "CX""#),
        "expected CNOT in CH decomposition: {text}"
    );
}

#[test]
fn controlled_named_parametric_circuit_lowers() {
    // Issue #374: `controlled(trotter_evolve(3))` must elaborate — the named
    // parametric callee's body is unrolled, then control distributes over
    // every gate. The result is a 2-qubit (control-plus-target) circuit
    // whose decomposition is built from CNOT/Rz primitives.
    let src = include_str!("fixtures/controlled_parametric.qn");
    let text = lower_text(src);
    assert!(
        text.contains(r#"sym_name = "controlled_trotter""#),
        "missing controlled_trotter: {text}"
    );
    // The controlled-Rz decomposition is `Rz |> CNOT |> Rz |> CNOT`, so the
    // three unrolled steps contribute CNOTs (the control wire).
    assert!(
        text.contains(r#"gate_name = "CNOT""#),
        "expected CNOT in the controlled decomposition: {text}"
    );
    // Width is preserved: control-plus-target = 2 qubits.
    assert!(
        text.contains("in_qubits = 2") && text.contains("out_qubits = 2"),
        "expected a 2-qubit controlled circuit: {text}"
    );
}

#[test]
fn controlled_unsupported_body_is_diagnostic() {
    // `identity(1)` is a valid 1-qubit circuit value, but not a single-qubit
    // generator the controlled elaborator knows how to decompose.
    let src = "fn bad(): Circuit<2, 2, 1, Clifford> = circuit { controlled(identity(1)) @(0, 1) }";
    let context = Context::new();
    let err = lower_program(&context, src).expect_err("unsupported controlled body");
    let msg = err
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        msg.contains("controlled()") || msg.contains("elaboration is not implemented"),
        "unexpected diagnostic: {msg}"
    );
}

#[test]
fn qec_repetition_lowers_to_dynamic_ops() {
    let src = r#"
fn main(): Q<Bit> = run {
  b <- repetition_code<3>()
  b2 <- memory_round(b)
  measure_logical_z(b2)
}
"#;
    let text = lower_text(src);
    assert!(
        text.contains("quantum.dynamic.qec_construct"),
        "missing construct: {text}"
    );
    assert!(
        text.contains("quantum.dynamic.qec_memory_round"),
        "missing memory_round: {text}"
    );
    assert!(
        text.contains("quantum.dynamic.qec_measure_logical"),
        "missing measure: {text}"
    );
    assert!(text.contains(r#"family = "repetition""#), "{text}");
    assert!(text.contains("distance = 3"), "{text}");
    // No staging ops survive lowering (#213 / ADR-0037).
    assert!(
        !text.contains("quantum.circ.run"),
        "staging run op leaked: {text}"
    );
    assert!(
        !text.contains("quantum.circ.qec_"),
        "staging qec op leaked: {text}"
    );
}

#[test]
fn qec_workload_extracts_after_lowering() {
    use mlir_bridge::collect_qec_workload;
    use quon_qec::{CodeFamily, LogicalBasis, LogicalQubitId, SourceFamily, WorkloadOp};

    let src = r#"
fn main(): Q<Bit> = run {
  b <- repetition_code<3>()
  b2 <- memory_round(b)
  b3 <- memory_round(b2)
  measure_logical_z(b3)
}
"#;
    let context = Context::new();
    let module = lower_program(&context, src).expect("lower");
    let workload = collect_qec_workload(&module).expect("collect");
    assert_eq!(workload.blocks().len(), 1);
    assert_eq!(workload.blocks()[0].family, SourceFamily::Repetition);
    assert_eq!(
        workload.blocks()[0].code_family,
        CodeFamily::RepetitionCodeToy { distance: 3 }
    );
    assert_eq!(workload.memory_round_count(), 2);
    assert_eq!(
        workload.ops().last(),
        Some(&WorkloadOp::MeasureLogical {
            logical_id: LogicalQubitId(0),
            basis: LogicalBasis::Z,
        })
    );
}

#[test]
fn qec_surface_workload_extracts_with_logical_cx() {
    use mlir_bridge::collect_qec_workload;
    use quon_qec::{CodeFamily, LogicalBasis, LogicalQubitId, SourceFamily, WorkloadOp};

    let src = r#"
fn main(): Q<(Bit, Bit)> = run {
  a <- surface_code<3>()
  b <- surface_code_x<3>()
  (a2, b2) <- logical_cx(a, b)
  mz <- measure_logical_z(a2)
  mx <- measure_logical_x(b2)
  return (mz, mx)
}
"#;
    let context = Context::new();
    let module = lower_program(&context, src).expect("lower");
    let workload = collect_qec_workload(&module).expect("collect");
    assert_eq!(workload.blocks().len(), 2);
    let a = &workload.blocks()[0];
    assert_eq!(a.family, SourceFamily::Surface);
    assert_eq!(a.distance, 3);
    assert_eq!(a.init_basis, LogicalBasis::Z);
    assert_eq!(a.code_family, CodeFamily::SurfaceCodeLike { distance: 3 });
    let b = &workload.blocks()[1];
    assert_eq!(b.family, SourceFamily::Surface);
    assert_eq!(b.distance, 3);
    assert_eq!(b.init_basis, LogicalBasis::X);
    assert_eq!(b.code_family, CodeFamily::SurfaceCodeLike { distance: 3 });
    assert_eq!(
        workload.ops(),
        vec![
            WorkloadOp::Construct {
                family: SourceFamily::Surface,
                distance: 3,
                basis: LogicalBasis::Z,
                logical_id: LogicalQubitId(0),
            },
            WorkloadOp::Construct {
                family: SourceFamily::Surface,
                distance: 3,
                basis: LogicalBasis::X,
                logical_id: LogicalQubitId(1),
            },
            WorkloadOp::LogicalCx {
                control: LogicalQubitId(0),
                target: LogicalQubitId(1),
            },
            WorkloadOp::MeasureLogical {
                logical_id: LogicalQubitId(0),
                basis: LogicalBasis::Z,
            },
            WorkloadOp::MeasureLogical {
                logical_id: LogicalQubitId(1),
                basis: LogicalBasis::X,
            },
        ]
    );
}

/// Collapsing the staging dialect (#213 / ADR-0037) means `lower` emits
/// `quantum.dynamic` IR directly — no `quantum.circ.run` / `qreg` / `apply` /
/// `cond_apply` / `yield` staging ops should ever reach the module.
#[test]
fn lowered_ir_has_no_staging_ops() {
    let src = r#"
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }
fn main(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
"#;
    let text = lower_text(src);
    // Dynamic IR is produced directly.
    assert!(text.contains("quantum.dynamic.unitary_region"), "{text}");
    assert_eq!(text.matches("quantum.dynamic.measure").count(), 2, "{text}");
    assert!(text.contains("quantum.dynamic.alloc"), "{text}");
    // No staging ops survive lowering.
    for staging in [
        "quantum.circ.run",
        "quantum.circ.qreg",
        "quantum.circ.apply",
        "quantum.circ.cond_apply",
        "quantum.circ.yield",
        "quantum.circ.measure",
    ] {
        assert!(
            !text.contains(staging),
            "{staging} leaked into lowered IR: {text}"
        );
    }
}

/// Feed-forward (`if cond then C else D @ q`) lowers to a `quantum.dynamic.if`
/// with both branches inlined as circ-only `unitary_region`-style bodies.
#[test]
fn conditional_application_lowers_to_dynamic_if() {
    let src = r#"
fn x_gate(): Circuit<1, 1, 1, Clifford> = circuit { X @0 }
fn id_one(): Circuit<1, 1, 1, Clifford> = circuit { I @0 }
fn main(): Q<Bit> = run {
    (q, r) <- qreg(2)
    b <- measure(q)
    c <- (if b then x_gate() else id_one()) @ r
    measure(c)
}
"#;
    let text = lower_text(src);
    assert!(text.contains("quantum.dynamic.if"), "{text}");
    assert!(!text.contains("quantum.circ.cond_apply"), "{text}");
    assert!(!text.contains("quantum.circ.run"), "{text}");
}
/// `init_plus()` lowers to a fresh `quantum.dynamic.alloc` allocation followed by an H
/// gate on that wire (issue #368). The prep gate threads the allocated qubit,
/// producing a new SSA value returned from the run block.
#[test]
fn init_plus_lowers_to_allocation_then_h() {
    let src = r#"
fn main(): Q<Qubit> = run {
    q <- init_plus()
    return q
}
"#;
    let text = lower_text(src);
    assert!(
        text.contains("quantum.dynamic.alloc"),
        "missing allocation: {text}"
    );
    assert!(
        text.contains(r#"gate_name = "H""#),
        "missing H prep gate: {text}"
    );
    assert!(
        !text.contains(r#"gate_name = "X""#),
        "init_plus must not emit X: {text}"
    );
}

/// `init_one()` lowers to a fresh `quantum.dynamic.alloc` allocation followed by an X
/// gate on that wire (issue #368).
#[test]
fn init_one_lowers_to_allocation_then_x() {
    let src = r#"
fn main(): Q<Qubit> = run {
    q <- init_one()
    return q
}
"#;
    let text = lower_text(src);
    assert!(
        text.contains("quantum.dynamic.alloc"),
        "missing allocation: {text}"
    );
    assert!(
        text.contains(r#"gate_name = "X""#),
        "missing X prep gate: {text}"
    );
    assert!(
        !text.contains(r#"gate_name = "H""#),
        "init_one must not emit H: {text}"
    );
}

/// When allocating multiple qubits, the prep gate from `init_plus()` applies
/// only to its own freshly-allocated wire, not to a sibling `qubit()` wire
/// (issue #368 acceptance criterion).
#[test]
fn init_plus_prep_gate_targets_only_allocated_wire() {
    let src = r#"
fn main(): Q<(Bit, Bit)> = run {
    q1 <- qubit()
    q2 <- init_plus()
    b1 <- measure(q1)
    b2 <- measure(q2)
    return (b1, b2)
}
"#;
    let text = lower_text(src);
    // Two allocations, exactly one H gate (on q2's wire).
    assert_eq!(
        text.matches("quantum.dynamic.alloc").count(),
        2,
        "expected two allocations: {text}"
    );
    assert_eq!(
        text.matches(r#"gate_name = "H""#).count(),
        1,
        "expected exactly one H gate: {text}"
    );
    assert!(
        !text.contains(r#"gate_name = "X""#),
        "no X expected: {text}"
    );
}

/// Bare `qubit()` allocation remains unchanged — no prep gate emitted
/// (issue #368: existing behavior must not regress).
#[test]
fn bare_qubit_allocation_emits_no_prep_gate() {
    let src = r#"
fn main(): Q<Qubit> = run {
    q <- qubit()
    return q
}
"#;
    let text = lower_text(src);
    assert!(
        text.contains("quantum.dynamic.alloc"),
        "missing allocation: {text}"
    );
    assert!(
        !text.contains(r#"gate_name = "H""#),
        "qubit() must not emit H: {text}"
    );
    assert!(
        !text.contains(r#"gate_name = "X""#),
        "qubit() must not emit X: {text}"
    );
}
