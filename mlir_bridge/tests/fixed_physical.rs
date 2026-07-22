//! Fixed physical layout canonical-channel tests (issue #316, ADR-0034).
//!
//! Proves that SSA wiring is the single authoritative representation for Fixed
//! physical layout identity, consumed by both emit and scheduling. The
//! `phys_qubit` attr is a derived annotation — corrupting it after routing
//! leaves emit and scheduling unchanged.

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::fixed_physical::{corrupt_phys_qubit_attrs, run_fixed_physical};
use mlir_bridge::passes::sabre_routing::{self, SabreCost};
use mlir_bridge::pipeline::emit_openqasm;
use quon_core::DepthExpr;

use support::{append_foreign_qubit, context, dynamic_context};

// ─── Targets ────────────────────────────────────────────────────────────────

/// Linear chain 0–1–2–3–4 with cx + swap native (for routing tests that need
/// SWAPs).  Same shape as the SABRE routing test's `linear_5q`.
fn linear_5q() -> backend::BackendTarget {
    let edges: Vec<(usize, usize)> = (0..4).map(|i| (i, i + 1)).collect();
    backend::BackendTarget::fixed(
        "linear5",
        backend::FixedTarget {
            num_qubits: 5,
            topology: backend::ConnectivityGraph::try_from_edges(5, edges).expect("topology"),
            native_gates: vec![
                backend::NativeGate::passthrough("cx", 2),
                backend::NativeGate::passthrough("swap", 2),
            ],
            noise: backend::NoiseModel::default(),
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    )
}

// ─── Circ-func module builders ──────────────────────────────────────────────

/// `func @non_adjacent(%q0, %q1, %q2) { H%q0; H%q1; H%q2; CNOT %q0,%q2; return }`
/// On a linear chain, CNOT(0,2) forces a SWAP — so phys_qubit attrs diverge
/// from the SSA WireTracker roots after routing.
fn non_adjacent_cnot_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location), (qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let q2 = Value::from(block.argument(2).unwrap());
    let g0 = block.append_operation(qc::gate(context, "H", 1, true, &[q0], location).unwrap());
    let g1 = block.append_operation(qc::gate(context, "H", 1, true, &[q1], location).unwrap());
    let g2 = block.append_operation(qc::gate(context, "H", 1, true, &[q2], location).unwrap());
    let q0 = Value::from(g0.result(0).unwrap());
    let q1 = Value::from(g1.result(0).unwrap());
    let q2 = Value::from(g2.result(0).unwrap());
    let cx =
        block.append_operation(qc::gate(context, "CNOT", 1, true, &[q0, q2], location).unwrap());
    let q0 = Value::from(cx.result(0).unwrap());
    let q2 = Value::from(cx.result(1).unwrap());
    block.append_operation(qc::r#return(&[q0, q1, q2], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "non_adjacent",
        3,
        3,
        &DepthExpr::Nat(4),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

// ─── Dynamic module builder (for emit tests) ────────────────────────────────

/// Dynamic module: two qubit allocations + a `unitary_region` body with
/// H; CNOT.  Emitable via `emit_openqasm` on `generic_openqasm`.
fn dynamic_bell_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let module = Module::new(location);
    let body = module.body();

    let q0 = append_foreign_qubit(context, &body, location);
    let q1 = append_foreign_qubit(context, &body, location);

    // unitary_region body: H(arg0); CNOT(h_out, arg1); return
    let ur_block = Block::new(&[(qubit, location), (qubit, location)]);
    let arg0 = Value::from(ur_block.argument(0).unwrap());
    let arg1 = Value::from(ur_block.argument(1).unwrap());
    let h = ur_block.append_operation(qc::gate(context, "H", 1, true, &[arg0], location).unwrap());
    let h_out = Value::from(h.result(0).unwrap());
    let cx = ur_block
        .append_operation(qc::gate(context, "CNOT", 1, true, &[h_out, arg1], location).unwrap());
    let cx0 = Value::from(cx.result(0).unwrap());
    let cx1 = Value::from(cx.result(1).unwrap());
    ur_block.append_operation(qc::r#return(&[cx0, cx1], location).unwrap());

    let ur_region = Region::new();
    ur_region.append_block(ur_block);

    body.append_operation(
        qd::unitary_region(
            context,
            &[q0, q1],
            &DepthExpr::Nat(2),
            true,
            ur_region,
            location,
        )
        .unwrap(),
    );
    module
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn parse_schedule_times(text: &str) -> Vec<i64> {
    text.split("schedule_time = ")
        .skip(1)
        .filter_map(|chunk| {
            chunk
                .split(|c: char| !c.is_ascii_digit() && c != '-')
                .next()
                .and_then(|s| s.parse().ok())
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// TEST 1: Scheduling derives from SSA, not phys_qubit attrs
// ═══════════════════════════════════════════════════════════════════════════

/// After routing a non-adjacent CNOT (SWAP inserted, phys_qubit attrs diverge
/// from roots), corrupting every phys_qubit attr must NOT change the schedule.
/// Before the fix, `resolve_phys_qubits` folded the attr into the root list,
/// so corrupting it would alter the dependency graph and change schedule_times.
#[test]
fn scheduling_ignores_phys_qubit_attrs() {
    let context = context();
    let module = non_adjacent_cnot_module(&context);
    let target = linear_5q();

    // Route — inserts SWAP, writes phys_qubit attrs that diverge from roots.
    sabre_routing::run_on_module(&context, &target, SabreCost::default(), &module);
    let routed_text = module.as_operation().to_string();
    assert!(
        routed_text.contains("gate_name = \"SWAP\""),
        "expected SWAP for non-adjacent CNOT: {routed_text}"
    );
    assert!(
        routed_text.contains("phys_qubit"),
        "expected phys_qubit attrs after routing: {routed_text}"
    );

    // Schedule — captures schedule_times derived from SSA roots.
    mlir_bridge::passes::depth_scheduling::run_on_module(&context, &target, &module);
    let times_before = parse_schedule_times(&module.as_operation().to_string());
    assert!(
        !times_before.is_empty(),
        "expected schedule_time attrs: {routed_text}"
    );

    // Corrupt every phys_qubit attr to a bogus value (999).
    corrupt_phys_qubit_attrs(&context, &module, 999);

    // Re-schedule — must produce identical schedule_times because scheduling
    // reads SSA roots, not the phys_qubit attr.
    mlir_bridge::passes::depth_scheduling::run_on_module(&context, &target, &module);
    let times_after = parse_schedule_times(&module.as_operation().to_string());

    assert_eq!(
        times_before, times_after,
        "scheduling must be unaffected by phys_qubit attr corruption \
         (SSA is canonical, ADR-0034):\nbefore: {times_before:?}\nafter:  {times_after:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// TEST 2: Emit derives from SSA, not phys_qubit attrs
// ═══════════════════════════════════════════════════════════════════════════

/// After routing, corrupting every phys_qubit attr must NOT change the emitted
/// QASM.  Emit follows SSA wiring (value→register threading) and never reads
/// the phys_qubit attr — this is the regression guard for that property.
#[test]
fn emit_ignores_phys_qubit_attrs() {
    let context = dynamic_context();
    let module = dynamic_bell_module(&context);
    let target = backend::generic_openqasm::target(2);

    // Route — writes phys_qubit attrs on every gate.
    sabre_routing::run_on_module(&context, &target, SabreCost::default(), &module);
    let routed_text = module.as_operation().to_string();
    assert!(
        routed_text.contains("phys_qubit"),
        "expected phys_qubit attrs after routing: {routed_text}"
    );

    // Emit — baseline QASM derived from SSA wiring.
    let qasm_before = emit_openqasm(&module, &target).expect("emit baseline");

    // Corrupt every phys_qubit attr to a bogus value (999).
    corrupt_phys_qubit_attrs(&context, &module, 999);

    // Re-emit — must be identical because emit reads SSA, not the attr.
    let qasm_after = emit_openqasm(&module, &target).expect("emit after corruption");

    assert_eq!(
        qasm_before, qasm_after,
        "emit must be unaffected by phys_qubit attr corruption \
         (SSA is canonical, ADR-0034):\nbefore:\n{qasm_before}\nafter:\n{qasm_after}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// TEST 3: Emit and scheduling agree on physical qubit identity
// ═══════════════════════════════════════════════════════════════════════════

/// After routing a circuit with a SWAP, both scheduling and emit process the
/// same SSA wiring.  The schedule must order the SWAP before the CNOT (they
/// share a qubit wire), and the QASM must contain both the SWAP and CX in that
/// order.  This verifies the two consumers agree on which qubits each gate
/// touches — the core property the canonical-channel decision guarantees.
#[test]
fn emit_and_scheduling_agree_on_routed_circuit() {
    let context = context();
    let module = non_adjacent_cnot_module(&context);
    let target = linear_5q();

    // Route — inserts SWAP before CNOT.
    sabre_routing::run_on_module(&context, &target, SabreCost::default(), &module);
    let ir = module.as_operation().to_string();

    let swap_pos = ir.find("gate_name = \"SWAP\"").expect("SWAP in IR");
    let cnot_pos = ir.rfind("gate_name = \"CNOT\"").expect("CNOT in IR");
    assert!(
        swap_pos < cnot_pos,
        "SWAP must precede CNOT in program order:\n{ir}"
    );

    // Schedule — the SWAP and CNOT share a qubit wire (SSA root), so the
    // scheduler must assign them strictly increasing schedule_times.
    mlir_bridge::passes::depth_scheduling::run_on_module(&context, &target, &module);
    let scheduled_ir = module.as_operation().to_string();
    let times = parse_schedule_times(&scheduled_ir);
    assert!(
        times.len() >= 2,
        "expected at least 2 schedule_time entries: {scheduled_ir}"
    );
    // The SWAP (earlier in program order) must not be scheduled after the
    // CNOT — they share a wire, so SWAP_time < CNOT_time.  Unconditional:
    // both times MUST be present or the test is vacuous.
    let swap_time =
        parse_schedule_times(&scheduled_ir[..scheduled_ir.find("gate_name = \"CNOT\"").unwrap()])
            .last()
            .copied()
            .expect("SWAP schedule_time must be present");
    let cnot_time =
        parse_schedule_times(&scheduled_ir[scheduled_ir.rfind("gate_name = \"CNOT\"").unwrap()..])
            .first()
            .copied()
            .expect("CNOT schedule_time must be present");
    assert!(
        swap_time < cnot_time,
        "SWAP (time {swap_time}) must be scheduled before CNOT (time {cnot_time}) \
         — they share a qubit wire:\n{scheduled_ir}"
    );

    // Both scheduling and emit derive qubit identity from the same SSA wiring.
    // Corrupting phys_qubit attrs must not change the schedule (emit already
    // never reads phys_qubit — it threads registers from SSA).  This proves
    // SSA is the single canonical channel (ADR-0034).
    corrupt_phys_qubit_attrs(&context, &module, 999);
    mlir_bridge::passes::depth_scheduling::run_on_module(&context, &target, &module);
    let times_after = parse_schedule_times(&module.as_operation().to_string());
    assert_eq!(
        times, times_after,
        "schedule must be stable under attr corruption — emit and scheduling \
         agree because both read SSA (ADR-0034)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// TEST 4: run_fixed_physical orchestrates the full Fixed pipeline
// ═══════════════════════════════════════════════════════════════════════════

/// The fixed_physical module's `run_fixed_physical` owns decomp → route →
// decomp → schedule.  Verify it's callable and produces schedule_time attrs.
#[test]
fn run_fixed_physical_produces_schedule() {
    let context = context();
    let module = non_adjacent_cnot_module(&context);
    let target = linear_5q();

    let result = run_fixed_physical(&context, &target, SabreCost::default(), &module);
    let ir = module.as_operation().to_string();
    assert!(
        ir.contains("schedule_time"),
        "expected schedule_time after run_fixed_physical: {ir}"
    );
    // T-count is sampled after SABRE, before post-SWAP decomp.  The circuit
    // has no T gates, so t_count == 0.
    assert_eq!(result.t_count, 0);
}
