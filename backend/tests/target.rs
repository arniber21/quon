// Unit tests for the backend crate — issue #3 acceptance criteria.

use std::path::Path;

use backend::error::BackendError;
use backend::target::{ConnectivityGraph, FixedTarget, UNREACHABLE};
use backend::{generic_openqasm, json};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn workspace_path(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn fixed(target: &backend::BackendTarget) -> &FixedTarget {
    target.fixed_target().expect("expected fixed target")
}

fn neutral_sample_json() -> String {
    std::fs::read_to_string(workspace_path(
        "../targets/neutral_atom/generic_rna_v0.json",
    ))
    .expect("read neutral atom sample")
}

fn neutral_sample_value() -> serde_json::Value {
    serde_json::from_str(&neutral_sample_json()).expect("parse neutral atom sample")
}

fn with_error_model_mutated(
    mutate: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> String {
    let mut value = neutral_sample_value();
    let model = value
        .get_mut("error_model")
        .and_then(|v| v.as_object_mut())
        .expect("error_model object");
    mutate(model);
    value.to_string()
}

// --- generic_openqasm ------------------------------------------------------

#[test]
fn generic_openqasm_is_all_to_all_with_no_noise() {
    let n = 4;
    let target = generic_openqasm::target(n);
    let fixed = fixed(&target);

    assert_eq!(target.id, "generic_openqasm");
    assert_eq!(target.kind_name(), "fixed");
    assert_eq!(fixed.num_qubits, n);
    assert!(fixed.supports_mid_circuit_meas);
    assert!(fixed.supports_feed_forward);
    assert_eq!(fixed.meas_latency_us, 0.0);

    // No noise model.
    assert!(fixed.noise.single_qubit_fidelity.is_empty());
    assert!(fixed.noise.two_qubit_fidelity.is_empty());
    assert!(fixed.noise.t1_us.is_empty());

    // All-to-all distances: 0 on the diagonal, 1 everywhere else.
    for i in 0..n {
        for j in 0..n {
            let expected = if i == j { 0 } else { 1 };
            assert_eq!(fixed.topology.dist(i, j), expected, "dist({i},{j})");
        }
    }
}

#[test]
fn generic_openqasm_has_standard_gates_native() {
    let target = generic_openqasm::target(2);
    for g in ["h", "x", "cx", "rz", "ccx", "swap", "u3"] {
        assert!(target.is_native(g), "expected `{g}` to be native");
    }
    assert!(!target.is_native("totally_made_up_gate"));
}

// --- Floyd-Warshall --------------------------------------------------------

/// Build an undirected ring of `n` qubits: 0-1-2-…-(n-1)-0.
fn ring(n: usize) -> Vec<(usize, usize)> {
    (0..n).map(|i| (i, (i + 1) % n)).collect()
}

#[test]
fn floyd_warshall_on_a_ring_matches_closed_form() {
    let n = 7;
    let graph = ConnectivityGraph::try_from_edges(n, ring(n)).expect("valid ring");
    for i in 0..n {
        for j in 0..n {
            let diff = i.abs_diff(j);
            let expected = diff.min(n - diff);
            assert_eq!(graph.dist(i, j), expected, "dist({i},{j}) on ring of {n}");
        }
    }
}

#[test]
fn floyd_warshall_on_a_line_counts_hops() {
    // Path graph 0-1-2-3-4.
    let edges = vec![(0, 1), (1, 2), (2, 3), (3, 4)];
    let graph = ConnectivityGraph::try_from_edges(5, edges).expect("valid line");
    assert_eq!(graph.dist(0, 4), 4);
    assert_eq!(graph.dist(1, 3), 2);
    assert_eq!(graph.dist(2, 2), 0);
}

#[test]
fn disconnected_components_are_unreachable() {
    // Two components: {0,1} and {2,3}.
    let edges = vec![(0, 1), (2, 3)];
    let graph = ConnectivityGraph::try_from_edges(4, edges).expect("valid graph");
    assert_eq!(graph.dist(0, 1), 1);
    assert_eq!(graph.dist(2, 3), 1);
    assert_eq!(graph.dist(0, 2), UNREACHABLE);
    assert_eq!(graph.dist(1, 3), UNREACHABLE);
}

// --- Connectivity validation ----------------------------------------------

#[test]
fn out_of_range_edge_is_rejected() {
    let err = ConnectivityGraph::try_from_edges(3, vec![(0, 1), (1, 5)]).unwrap_err();
    assert!(
        matches!(
            err,
            BackendError::EdgeOutOfRange {
                a: 1,
                b: 5,
                num_qubits: 3
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn self_loop_is_rejected() {
    let err = ConnectivityGraph::try_from_edges(3, vec![(1, 1)]).unwrap_err();
    assert!(matches!(err, BackendError::SelfLoop(1)), "got {err:?}");
}

// --- JSON loading ----------------------------------------------------------

#[test]
fn spec_5q_device_loads_correctly() {
    let target = json::load(&fixture("device_5q.json")).expect("fixture should load");
    let fixed = fixed(&target);

    assert_eq!(target.id, "my_device");
    assert_eq!(fixed.num_qubits, 5);
    assert_eq!(
        fixed.topology.edges,
        vec![(0, 1), (1, 2), (2, 3), (3, 4), (0, 2)]
    );

    for g in ["cx", "rz", "sx", "x"] {
        assert!(target.is_native(g), "expected `{g}` native");
    }

    // Noise: tuple-keyed lookups.
    assert_eq!(
        fixed.noise.single_qubit_fidelity[&("rz".to_string(), 0)],
        0.999
    );
    assert_eq!(
        fixed.noise.two_qubit_fidelity[&("cx".to_string(), 0, 1)],
        0.995
    );
    assert_eq!(fixed.noise.t1_us[&0], 120.0);
    assert_eq!(fixed.noise.readout_error[&1], 0.015);

    assert_eq!(fixed.meas_latency_us, 0.9);
    assert!(fixed.supports_mid_circuit_meas);
    assert!(fixed.supports_feed_forward);

    // The cross-link 0-2 makes dist(0,2) == 1, shorter than going 0-1-2.
    assert_eq!(fixed.topology.dist(0, 2), 1);
    assert_eq!(fixed.topology.dist(0, 4), 3); // 0-2-3-4
}

#[test]
fn missing_num_qubits_is_rejected_with_clear_error() {
    let err = json::load(&fixture("device_missing_num_qubits.json")).unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
    assert!(
        msg.contains("num_qubits"),
        "error should name the missing field, got: {msg}"
    );
}

#[test]
fn missing_native_gates_is_rejected_with_clear_error() {
    let err = json::load(&fixture("device_missing_native_gates.json")).unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
    assert!(
        msg.contains("native_gates"),
        "error should name the missing field, got: {msg}"
    );
}

#[test]
fn out_of_range_edge_in_json_is_rejected() {
    let err = json::load(&fixture("device_bad_edge.json")).unwrap_err();
    assert!(
        matches!(err, BackendError::EdgeOutOfRange { .. }),
        "got {err:?}"
    );
}

#[test]
fn unknown_gate_is_rejected() {
    let src = r#"{
        "id": "d", "num_qubits": 2,
        "topology": {"edges": [[0,1]]},
        "native_gates": ["cx", "frobnicate"],
        "meas_latency_us": 0.0,
        "supports_mid_circuit_meas": false,
        "supports_feed_forward": false
    }"#;
    let err = json::from_str(src).unwrap_err();
    assert!(
        matches!(&err, BackendError::UnknownGate(g) if g == "frobnicate"),
        "got {err:?}"
    );
}

#[test]
fn unknown_field_is_rejected() {
    let src = r#"{
        "id": "d", "num_qubits": 2,
        "topology": {"edges": [[0,1]]},
        "native_gates": ["cx"],
        "meas_latency_us": 0.0,
        "supports_mid_circuit_meas": false,
        "supports_feed_forward": false,
        "surprise_field": 42
    }"#;
    let err = json::from_str(src).unwrap_err();
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
}

#[test]
fn json_round_trips_through_descriptor() {
    let target = json::load(&fixture("device_5q.json")).expect("load");
    let descriptor = target.to_descriptor();
    let serialized = serde_json::to_string(&descriptor).expect("serialize");
    let reloaded = json::from_str(&serialized).expect("reload");
    let target_fixed = fixed(&target);
    let reloaded_fixed = fixed(&reloaded);

    assert_eq!(reloaded.id, target.id);
    assert_eq!(reloaded_fixed.num_qubits, target_fixed.num_qubits);
    assert_eq!(reloaded_fixed.topology.edges, target_fixed.topology.edges);
    assert_eq!(
        reloaded_fixed.noise.single_qubit_fidelity,
        target_fixed.noise.single_qubit_fidelity
    );
    assert_eq!(
        reloaded_fixed.noise.two_qubit_fidelity,
        target_fixed.noise.two_qubit_fidelity
    );
    assert_eq!(reloaded_fixed.noise.t1_us, target_fixed.noise.t1_us);
    assert_eq!(
        reloaded_fixed.noise.readout_error,
        target_fixed.noise.readout_error
    );
    let mut a: Vec<_> = reloaded_fixed
        .native_gates
        .iter()
        .map(|g| &g.name)
        .collect();
    let mut b: Vec<_> = target_fixed.native_gates.iter().map(|g| &g.name).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
}

#[test]
fn passthrough_decomposition_returns_the_gate_itself() {
    let target = generic_openqasm::target(2);
    let cx = fixed(&target)
        .native_gates
        .iter()
        .find(|g| g.name == "cx")
        .unwrap();
    let ops = (cx.decompose)(&[]);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].name, "cx");
    assert_eq!(ops[0].qubits, vec![0, 1]);
}

// --- Noise-model loader error paths ----------------------------------------

/// Build a 2-qubit descriptor JSON with a custom `noise` object spliced in.
fn descriptor_with_noise(noise: &str) -> String {
    format!(
        r#"{{
            "id": "d", "num_qubits": 2,
            "topology": {{"edges": [[0,1]]}},
            "native_gates": ["cx", "rz"],
            "noise": {noise},
            "meas_latency_us": 0.0,
            "supports_mid_circuit_meas": false,
            "supports_feed_forward": false
        }}"#
    )
}

#[test]
fn malformed_two_qubit_noise_key_is_rejected() {
    // Missing the comma separator.
    let src = descriptor_with_noise(r#"{"two_qubit_fidelity": {"cx": {"01": 0.99}}}"#);
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(&err, BackendError::BadTwoQubitKey(k) if k == "01"),
        "got {err:?}"
    );
}

#[test]
fn non_numeric_qubit_noise_key_is_rejected() {
    let src = descriptor_with_noise(r#"{"single_qubit_fidelity": {"rz": {"abc": 0.99}}}"#);
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(&err, BackendError::BadQubitKey(k) if k == "abc"),
        "got {err:?}"
    );
}

#[test]
fn out_of_range_single_qubit_noise_is_rejected() {
    // num_qubits == 2, so qubit 99 is invalid.
    let src = descriptor_with_noise(r#"{"t1_us": {"99": 100.0}}"#);
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(
            err,
            BackendError::QubitOutOfRange {
                got: 99,
                num_qubits: 2
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn out_of_range_two_qubit_noise_is_rejected() {
    let src = descriptor_with_noise(r#"{"two_qubit_fidelity": {"cx": {"0,9": 0.99}}}"#);
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(
            err,
            BackendError::QubitOutOfRange {
                got: 9,
                num_qubits: 2
            }
        ),
        "got {err:?}"
    );
}

// --- Boundary sizes --------------------------------------------------------

#[test]
fn zero_and_one_qubit_graphs_are_well_formed() {
    let empty = ConnectivityGraph::all_to_all(0);
    assert_eq!(empty.num_qubits, 0);
    assert!(empty.dist.is_empty());

    let single = ConnectivityGraph::all_to_all(1);
    assert_eq!(single.dist(0, 0), 0);

    // The built-in target must also construct at these sizes.
    assert_eq!(fixed(&generic_openqasm::target(0)).num_qubits, 0);
    assert_eq!(fixed(&generic_openqasm::target(1)).num_qubits, 1);

    // try_from_edges with no edges is valid at n == 1.
    let g = ConnectivityGraph::try_from_edges(1, vec![]).expect("single qubit, no edges");
    assert_eq!(g.dist(0, 0), 0);
}

// --- Native-gate registry --------------------------------------------------

#[test]
fn native_gate_arities_are_correct() {
    let target = generic_openqasm::target(3);
    let arity = |name: &str| {
        fixed(&target)
            .native_gates
            .iter()
            .find(|g| g.name == name)
            .map(|g| g.num_qubits)
    };
    assert_eq!(arity("h"), Some(1));
    assert_eq!(arity("cx"), Some(2));
    assert_eq!(arity("ccx"), Some(3));
}

// --- Neutral-atom target descriptors ---------------------------------------

#[test]
fn generic_neutral_atom_target_loads_correctly() {
    let target = json::load(&workspace_path(
        "../targets/neutral_atom/generic_rna_v0.json",
    ))
    .expect("neutral atom target should load");
    let na = target
        .neutral_atom_target()
        .expect("expected neutral atom target");

    assert_eq!(target.id, "generic_reconfigurable_neutral_atom_v0");
    assert_eq!(target.kind_name(), "neutral_atom_reconfigurable");
    assert!(target.is_native("cz"));
    assert!(target.is_native("measure_z"));
    assert!(!target.is_native("cx"));
    assert_eq!(na.zone_capacity(backend::ZoneKind::Entanglement), 340);
    assert_eq!(na.interaction.max_parallel_entangling_pairs, 340);
    assert_eq!(na.movement.aod_rows, 100);
    assert_eq!(na.movement.aod_cols, 100);
    assert_eq!(na.interaction.min_rydberg_spacing_um, 18.75);

    let error_model = na
        .error_model
        .as_ref()
        .expect("generic_rna_v0 includes an example error_model");
    // Placeholder rates — deliberately not 1 - fidelity.* (ADR-0017).
    assert_eq!(error_model.rydberg, 0.002);
    assert_eq!(error_model.measurement, 0.003);
    assert_eq!(error_model.reset, 0.004);
    assert_eq!(error_model.movement, 0.0005);
    assert_eq!(error_model.transfer, 0.0007);
    assert_eq!(error_model.idle_per_us, 2e-9);
    assert_eq!(na.fidelity.cz, 0.995);
    assert!((error_model.rydberg - (1.0 - na.fidelity.cz)).abs() > 1e-9);
}

#[test]
fn neutral_atom_error_model_is_optional() {
    let mut value = neutral_sample_value();
    value.as_object_mut().expect("object").remove("error_model");
    let target =
        json::from_str(&value.to_string()).expect("target without error_model should load");
    let na = target
        .neutral_atom_target()
        .expect("expected neutral atom target");
    assert!(na.error_model.is_none());
    assert!(
        matches!(
            na.require_error_model(),
            Err(BackendError::MissingErrorModel)
        ),
        "QEC paths must hard-fail when error_model is absent"
    );
    assert!(
        na.require_error_model()
            .unwrap_err()
            .to_string()
            .contains("--emit-resource-report")
    );
}

#[test]
fn neutral_atom_error_model_rejects_unknown_fields() {
    let src = with_error_model_mutated(|m| {
        m.insert("typo_rate".into(), serde_json::json!(0.1));
    });
    let err = json::from_str(&src).unwrap_err();
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
    assert!(err.to_string().contains("typo_rate") || err.to_string().contains("unknown field"));
}

#[test]
fn neutral_atom_error_model_rejects_incomplete_object() {
    let src = with_error_model_mutated(|m| {
        m.remove("reset");
    });
    let err = json::from_str(&src).unwrap_err();
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
    assert!(
        err.to_string().contains("reset") || err.to_string().contains("missing field"),
        "got {err}"
    );
}

#[test]
fn neutral_atom_error_model_rejects_missing_key() {
    let src = with_error_model_mutated(|m| {
        m.remove("movement");
    });
    let err = json::from_str(&src).unwrap_err();
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
    assert!(
        err.to_string().contains("movement") || err.to_string().contains("missing field"),
        "got {err}"
    );
}

#[test]
fn neutral_atom_error_model_rejects_out_of_range_probability() {
    let src = with_error_model_mutated(|m| {
        m.insert("rydberg".into(), serde_json::json!(1.5));
    });
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(err.to_string().contains("error_model.rydberg"));
}

#[test]
fn neutral_atom_error_model_rejects_negative_probability() {
    let src = with_error_model_mutated(|m| {
        m.insert("measurement".into(), serde_json::json!(-0.01));
    });
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(err.to_string().contains("error_model.measurement"));
}

#[test]
fn neutral_atom_error_model_round_trips_fields() {
    let target = json::load(&workspace_path(
        "../targets/neutral_atom/generic_rna_v0.json",
    ))
    .expect("load");
    let na = target.neutral_atom_target().expect("na");
    let model = na.require_error_model().expect("error_model present");
    let snap = model.error_model_snapshot();
    assert_eq!(snap.rydberg, model.rydberg);
    assert_eq!(snap.measurement, model.measurement);
    assert_eq!(snap.reset, model.reset);
    assert_eq!(snap.movement, model.movement);
    assert_eq!(snap.transfer, model.transfer);
    assert_eq!(snap.idle_per_us, model.idle_per_us);

    let wire = serde_json::to_value(snap).expect("serialize snapshot");
    let back: backend::NeutralAtomErrorModelSnapshot =
        serde_json::from_value(wire).expect("deserialize snapshot");
    assert_eq!(back, snap);

    let domain = backend::NeutralAtomErrorModel::try_from(snap).expect("snapshot → domain");
    assert_eq!(&domain, model);

    // Descriptor round-trip preserves error_model rates.
    let desc = target.to_descriptor();
    let reloaded = backend::BackendTarget::try_from(desc).expect("descriptor → target");
    let na2 = reloaded.neutral_atom_target().expect("na");
    assert_eq!(na2.error_model, na.error_model);
}

#[test]
fn neutral_atom_error_model_snapshot_try_from_rejects_oor() {
    let bad = backend::NeutralAtomErrorModelSnapshot {
        rydberg: 1.5,
        measurement: 0.0,
        reset: 0.0,
        movement: 0.0,
        transfer: 0.0,
        idle_per_us: 0.0,
    };
    let err = backend::NeutralAtomErrorModel::try_from(bad).expect_err("oor");
    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(err.to_string().contains("error_model.rydberg"));
}

#[test]
fn neutral_atom_require_error_model_succeeds_when_present() {
    let target = json::load(&workspace_path(
        "../targets/neutral_atom/generic_rna_v0.json",
    ))
    .expect("load");
    let na = target.neutral_atom_target().expect("na");
    let model = na.require_error_model().expect("error_model present");
    assert!(model.rydberg > 0.0);
}

#[test]
fn fake_manila_v2_snapshot_loads_with_noise() {
    let target = json::load(&workspace_path("../targets/ibm/fake_manila_v2.json"))
        .expect("fake_manila_v2 snapshot should load");
    let fixed = target.fixed_target().expect("expected fixed IBM target");

    assert_eq!(target.id, "fake_manila_v2");
    assert_eq!(fixed.num_qubits, 5);
    assert_eq!(fixed.topology.edges.len(), 4);
    assert!(target.is_native("cx"));
    assert!(target.is_native("sx"));
    assert!(
        fixed
            .noise
            .two_qubit_fidelity
            .contains_key(&("cx".into(), 0, 1))
            || fixed
                .noise
                .two_qubit_fidelity
                .contains_key(&("cx".into(), 1, 0)),
        "expected bidirectional CX fidelity entries"
    );
    assert_eq!(fixed.noise.t1_us.len(), 5);
    assert_eq!(fixed.noise.readout_error.len(), 5);
}

#[test]
fn neutral_atom_target_round_trips_through_descriptor() {
    let target = json::load(&workspace_path(
        "../targets/neutral_atom/generic_rna_v0.json",
    ))
    .expect("neutral atom target should load");
    let serialized = serde_json::to_string(&target.to_descriptor()).expect("serialize");
    let reloaded = json::from_str(&serialized).expect("reload");

    assert_eq!(reloaded.id, target.id);
    assert_eq!(reloaded.kind_name(), "neutral_atom_reconfigurable");
    assert_eq!(reloaded.neutral_atom_target(), target.neutral_atom_target());
}

/// Build a jerk-limited variant of the generic RNA target by rewriting the
/// `movement.speed_model` object in the sample JSON (issue #308).
fn jerk_limited_sample_json(jerk_m_s3: f64, max_velocity_m_s: f64) -> String {
    let mut value = neutral_sample_value();
    let movement = value
        .get_mut("movement")
        .and_then(|v| v.as_object_mut())
        .expect("movement object");
    movement.insert(
        "speed_model".into(),
        serde_json::json!({
            "kind": "jerk_limited",
            "acceleration_m_s2": 2750.0,
            "jerk_m_s3": jerk_m_s3,
            "max_velocity_m_s": max_velocity_m_s,
        }),
    );
    serde_json::to_string(&value).expect("reserialize jerk-limited target")
}

#[test]
fn neutral_atom_jerk_limited_speed_model_loads_and_round_trips() {
    let src = jerk_limited_sample_json(1.0e8, 0.5);
    let target = json::from_str(&src).expect("jerk_limited target loads");
    let na = target.neutral_atom_target().expect("na");
    assert_eq!(
        na.movement.speed_model.kind,
        backend::AodSpeedModelKind::JerkLimited
    );
    assert_eq!(na.movement.speed_model.acceleration_m_s2, 2750.0);
    assert_eq!(na.movement.speed_model.jerk_m_s3, 1.0e8);
    assert_eq!(na.movement.speed_model.max_velocity_m_s, 0.5);

    // Descriptor round-trip preserves the jerk-limited kind + params.
    let serialized = serde_json::to_string(&target.to_descriptor()).expect("serialize");
    let reloaded = json::from_str(&serialized).expect("reload");
    assert_eq!(
        reloaded.neutral_atom_target(),
        target.neutral_atom_target(),
        "jerk_limited speed model must survive descriptor round-trip"
    );
    // The serialized form names the variant in snake_case.
    assert!(
        serialized.contains("\"kind\":\"jerk_limited\""),
        "kind not snake_case: {serialized}"
    );
    assert!(serialized.contains("\"jerk_m_s3\""));
    assert!(serialized.contains("\"max_velocity_m_s\""));
}

#[test]
fn neutral_atom_jerk_limited_rejects_nonpositive_jerk() {
    // jerk_m_s3 <= 0 is invalid for jerk_limited (sqrt ignores the field).
    let src = jerk_limited_sample_json(0.0, 0.5);
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(
        err.to_string().contains("movement.speed_model.jerk_m_s3"),
        "error should name jerk_m_s3: {err}"
    );
}

#[test]
fn neutral_atom_jerk_limited_rejects_negative_max_velocity() {
    let src = jerk_limited_sample_json(1.0e8, -0.1);
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(
        err.to_string()
            .contains("movement.speed_model.max_velocity_m_s"),
        "error should name max_velocity_m_s: {err}"
    );
}

#[test]
fn neutral_atom_sqrt_target_loads_without_jerk_fields() {
    // Backward compat: the pinned sqrt target JSON carries no jerk_m_s3 /
    // max_velocity_m_s fields; they default to 0.0 and stay unused under sqrt.
    let target = json::load(&workspace_path(
        "../targets/neutral_atom/generic_rna_v0.json",
    ))
    .expect("sqrt target loads");
    let na = target.neutral_atom_target().expect("na");
    assert_eq!(
        na.movement.speed_model.kind,
        backend::AodSpeedModelKind::Sqrt
    );
    assert_eq!(na.movement.speed_model.jerk_m_s3, 0.0);
    assert_eq!(na.movement.speed_model.max_velocity_m_s, 0.0);
}

#[test]
fn neutral_atom_negative_zone_extent_is_rejected() {
    let src = neutral_sample_json().replace("\"rows\": 73", "\"rows\": -1");
    let err = json::from_str(&src).unwrap_err();

    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(err.to_string().contains("zones[].rows"));
}

#[test]
fn neutral_atom_overlapping_zones_are_rejected() {
    let src =
        neutral_sample_json().replace("\"origin_um\": [0.0, 430.0]", "\"origin_um\": [10.0, 10.0]");
    let err = json::from_str(&src).unwrap_err();

    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(err.to_string().contains("overlap"));
}

#[test]
fn neutral_atom_parallel_pair_capacity_is_validated() {
    let src = neutral_sample_json().replace(
        "\"max_parallel_entangling_pairs\": 340",
        "\"max_parallel_entangling_pairs\": 341",
    );
    let err = json::from_str(&src).unwrap_err();

    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(
        err.to_string()
            .contains("exceeds entanglement zone capacity")
    );
}

#[test]
fn neutral_atom_zone_outside_grid_is_rejected() {
    let src = neutral_sample_json().replace("\"height_um\": 500.0", "\"height_um\": 320.0");
    let err = json::from_str(&src).unwrap_err();

    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(err.to_string().contains("exceeds grid bounds"));
}

#[test]
fn unknown_target_kind_is_rejected() {
    let src = r#"{
        "id": "mystery",
        "kind": "ion_trap",
        "native_gates": []
    }"#;
    let err = json::from_str(src).unwrap_err();

    assert!(
        matches!(&err, BackendError::UnknownTargetKind(kind) if kind == "ion_trap"),
        "got {err:?}"
    );
}

// --- Neutral-atom atom_loss_model (issue #310, Atomique Eqs. 1–2) -----------

/// Insert an `atom_loss_model` object into a copy of the sample JSON.
fn with_atom_loss_model(body: &str) -> String {
    let mut value = neutral_sample_value();
    value.as_object_mut().expect("object").insert(
        "atom_loss_model".into(),
        serde_json::from_str(body).expect("json"),
    );
    value.to_string()
}

#[test]
fn neutral_atom_loss_model_is_optional() {
    // generic_rna_v0.json carries no atom_loss_model (backward-compat): it
    // still loads and the field is None.
    let target = json::load(&workspace_path(
        "../targets/neutral_atom/generic_rna_v0.json",
    ))
    .expect("load");
    let na = target
        .neutral_atom_target()
        .expect("expected neutral atom target");
    assert!(na.atom_loss_model.is_none());
}

#[test]
fn neutral_atom_loss_model_loads_and_round_trips() {
    let src = with_atom_loss_model(r#"{"heating_rate_per_um": 0.01, "loss_coeff": 0.5}"#);
    let target = json::from_str(&src).expect("target with atom_loss_model loads");
    let na = target.neutral_atom_target().expect("na");
    let model = na
        .atom_loss_model
        .expect("atom_loss_model present after load");
    assert_eq!(model.heating_rate_per_um, 0.01);
    assert_eq!(model.loss_coeff, 0.5);

    // Descriptor round-trip preserves the loss model.
    let desc = target.to_descriptor();
    let reloaded = backend::BackendTarget::try_from(desc).expect("descriptor → target");
    let na2 = reloaded.neutral_atom_target().expect("na");
    assert_eq!(na2.atom_loss_model, na.atom_loss_model);

    // `to_descriptor` must not emit the key when the field is None.
    let none_src = neutral_sample_json();
    let none_target = json::from_str(&none_src).expect("load");
    let serialized = serde_json::to_string(&none_target.to_descriptor()).expect("serialize");
    assert!(
        !serialized.contains("atom_loss_model"),
        "None atom_loss_model must be skipped on serialize"
    );
}

#[test]
fn neutral_atom_loss_model_rejects_unknown_fields() {
    let src = with_atom_loss_model(
        r#"{"heating_rate_per_um": 0.01, "loss_coeff": 0.5, "cooling_rate": 0.2}"#,
    );
    let err = json::from_str(&src).unwrap_err();
    // `deny_unknown_fields` rejects at serde-deserialize time → Json error,
    // same as the error_model unknown-field test.
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
    assert!(
        err.to_string().contains("cooling_rate") || err.to_string().contains("unknown field"),
        "got {err}"
    );
}

#[test]
fn neutral_atom_loss_model_rejects_missing_key() {
    let src = with_atom_loss_model(r#"{"heating_rate_per_um": 0.01}"#);
    let err = json::from_str(&src).unwrap_err();
    assert!(matches!(err, BackendError::Json(_)), "got {err:?}");
    assert!(
        err.to_string().contains("loss_coeff") || err.to_string().contains("missing field"),
        "got {err}"
    );
}

#[test]
fn neutral_atom_loss_model_rejects_negative() {
    let src = with_atom_loss_model(r#"{"heating_rate_per_um": -0.01, "loss_coeff": 0.5}"#);
    let err = json::from_str(&src).unwrap_err();
    assert!(
        matches!(err, BackendError::InvalidTargetConfig(_)),
        "got {err:?}"
    );
    assert!(
        err.to_string()
            .contains("atom_loss_model.heating_rate_per_um")
    );
}
