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
