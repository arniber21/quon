//! Unit tests for interaction-graph core types (issue #103).

use std::collections::BTreeSet;

use proptest::prelude::*;
use quon_na::{
    DEFAULT_GAMMA, GraphError, Interaction, InteractionGraph, InteractionId, InteractionSegment,
    LogicalQubitId, SegmentKind, aggregate_edges, cubic_commutation_graph,
    erdos_renyi_commutation_graph, schedule_dependency_segment,
};
use serde_json::json;
fn cz(id: u32, a: u32, b: u32, layer: u32, critical: bool) -> Interaction {
    let mut qubits = vec![LogicalQubitId(a), LogicalQubitId(b)];
    qubits.sort();
    Interaction {
        id: InteractionId(id),
        qubits,
        gate_name: "CZ".to_string(),
        dag_layer: layer,
        on_critical_path: critical,
    }
}

fn triangle_graph() -> InteractionGraph {
    InteractionGraph::from_interactions(
        vec![LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2)],
        vec![
            cz(0, 0, 1, 0, true),
            cz(1, 1, 2, 0, true),
            cz(2, 0, 2, 0, false),
        ],
        vec![InteractionSegment {
            kind: SegmentKind::DependencyDag,
            interactions: vec![InteractionId(0), InteractionId(1), InteractionId(2)],
        }],
        DEFAULT_GAMMA,
    )
    .expect("triangle")
}

#[test]
fn json_round_trip() {
    let graph = triangle_graph();
    let text = graph.to_json_string_pretty().unwrap();
    let parsed: InteractionGraph = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, graph);
}

#[test]
fn json_rejects_unknown_fields() {
    let value = json!({
        "vertices": [{"0": 0}],
        "interactions": [],
        "edges": [],
        "segments": [],
        "gamma": 0.8,
        "extra": true
    });
    // LogicalQubitId is a newtype tuple — use the correct shape and still reject extras.
    let bad = r#"{
        "vertices": [0],
        "interactions": [],
        "edges": [],
        "segments": [],
        "gamma": 0.8,
        "extra": true
    }"#;
    let err = serde_json::from_str::<InteractionGraph>(bad).unwrap_err();
    assert!(err.to_string().contains("unknown field") || err.to_string().contains("extra"));
    let _ = value;
}

#[test]
fn dot_contains_nodes_and_weighted_edges() {
    let graph = triangle_graph();
    let dot = graph.to_dot();
    assert!(dot.contains("q0"));
    assert!(dot.contains("q1"));
    assert!(dot.contains("q2"));
    assert!(dot.contains("q0 -- q1"));
    assert!(
        dot.contains("label=\"1.000\"")
            || dot.contains("label=\"0.800\"")
            || dot.contains("label=")
    );
    assert!(dot.contains("penwidth=2.5"));
}

#[test]
fn validate_rejects_self_loop() {
    let mut graph = triangle_graph();
    graph.edges[0].a = LogicalQubitId(1);
    graph.edges[0].b = LogicalQubitId(1);
    assert!(matches!(
        graph.validate(),
        Err(GraphError::SelfLoop(LogicalQubitId(1)))
    ));
}

#[test]
fn validate_rejects_arity_one() {
    let err = InteractionGraph::from_interactions(
        vec![LogicalQubitId(0)],
        vec![Interaction {
            id: InteractionId(0),
            qubits: vec![LogicalQubitId(0)],
            gate_name: "X".to_string(),
            dag_layer: 0,
            on_critical_path: false,
        }],
        vec![InteractionSegment {
            kind: SegmentKind::DependencyDag,
            interactions: vec![InteractionId(0)],
        }],
        DEFAULT_GAMMA,
    )
    .unwrap_err();
    assert!(matches!(err, GraphError::Arity(InteractionId(0))));
}

#[test]
fn validate_rejects_unknown_segment_id() {
    let err = InteractionGraph::from_interactions(
        vec![LogicalQubitId(0), LogicalQubitId(1)],
        vec![cz(0, 0, 1, 0, true)],
        vec![InteractionSegment {
            kind: SegmentKind::DependencyDag,
            interactions: vec![InteractionId(99)],
        }],
        DEFAULT_GAMMA,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        GraphError::UnknownInteraction(InteractionId(99))
            | GraphError::UnpartitionedInteraction(InteractionId(0))
    ));
}

#[test]
fn weight_aggregation_two_layers() {
    let interactions = vec![cz(0, 0, 1, 0, true), cz(1, 0, 1, 1, true)];
    let edges = aggregate_edges(&interactions, DEFAULT_GAMMA).unwrap();
    assert_eq!(edges.len(), 1);
    assert!((edges[0].weight - (1.0 + DEFAULT_GAMMA)).abs() < 1e-12);
}

#[test]
fn three_qubit_gate_makes_three_pairwise_edges() {
    let graph = InteractionGraph::from_interactions(
        vec![LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2)],
        vec![Interaction {
            id: InteractionId(0),
            qubits: vec![LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2)],
            gate_name: "CCZ".to_string(),
            dag_layer: 0,
            on_critical_path: true,
        }],
        vec![InteractionSegment {
            kind: SegmentKind::DependencyDag,
            interactions: vec![InteractionId(0)],
        }],
        DEFAULT_GAMMA,
    )
    .unwrap();
    assert_eq!(graph.edges.len(), 3);
}

#[test]
fn critical_path_linear_vs_parallel() {
    let mut chain = vec![
        cz(0, 0, 1, 0, false),
        cz(1, 1, 2, 0, false),
        cz(2, 2, 3, 0, false),
    ];
    schedule_dependency_segment(&mut chain);
    assert_eq!(
        chain.iter().map(|i| i.dag_layer).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(chain.iter().all(|i| i.on_critical_path));

    let mut parallel = vec![cz(0, 0, 1, 0, false), cz(1, 2, 3, 0, false)];
    schedule_dependency_segment(&mut parallel);
    assert_eq!(parallel[0].dag_layer, 0);
    assert_eq!(parallel[1].dag_layer, 0);
    assert!(parallel.iter().all(|i| i.on_critical_path));
}

#[test]
fn cubic_generator_is_3_regular() {
    for n in [4u32, 6, 8, 10, 12] {
        let graph = cubic_commutation_graph(n).unwrap();
        assert_eq!(graph.vertices.len(), n as usize);
        let mut degree = vec![0u32; n as usize];
        for edge in &graph.edges {
            degree[edge.a.0 as usize] += 1;
            degree[edge.b.0 as usize] += 1;
        }
        assert!(degree.iter().all(|&d| d == 3), "n={n} degrees={degree:?}");
        assert_eq!(graph.segments.len(), 1);
        assert_eq!(graph.segments[0].kind, SegmentKind::CommutationGroup);
        assert!(graph.interactions.iter().all(|i| !i.on_critical_path));
    }
}

// ---------------------------------------------------------------------------
// Issue #402: deserialization must route through `InteractionGraph::validate`
// — the same invariants `from_interactions` enforces — so raw JSON can never
// produce an invalid graph at a wire boundary.
// ---------------------------------------------------------------------------

/// A valid triangle graph serialized to a JSON value — the base for mutations.
fn valid_graph_value() -> serde_json::Value {
    serde_json::to_value(triangle_graph()).expect("serialize valid graph")
}

/// Assert that deserializing a mutated graph fails with a `GraphError`
/// validation message (not a structural serde decode error).
fn assert_rejects(mutated: serde_json::Value) {
    let err = serde_json::from_value::<InteractionGraph>(mutated)
        .expect_err("invalid graph must be rejected at deserialize time");
    let msg = err.to_string();
    assert!(
        [
            "gamma must be in",
            "duplicate vertex id",
            "duplicate interaction id",
            "has fewer than 2 qubits",
            "has unsorted or duplicate qubits",
            "references unknown vertex",
            "edge endpoints not in vertex set",
            "self-loop on",
            "edge endpoints must be ordered",
            "segment references unknown interaction",
            "is not covered by exactly one segment",
            "edge weight must be finite and non-negative",
        ]
        .iter()
        .any(|p| msg.contains(p)),
        "expected a GraphError validation message, got: {msg}"
    );
}

#[test]
fn deserialize_valid_graph_equals_original() {
    let graph = triangle_graph();
    let value = serde_json::to_value(&graph).unwrap();
    let parsed: InteractionGraph = serde_json::from_value(value).unwrap();
    assert_eq!(parsed, graph);
}

#[test]
fn deserialize_rejects_gamma_out_of_range() {
    for bad in [0.0_f64, -0.5, 1.7] {
        let mut v = valid_graph_value();
        v["gamma"] = json!(bad);
        assert_rejects(v);
    }
}

#[test]
fn deserialize_rejects_unknown_qubit_in_interaction() {
    let mut v = valid_graph_value();
    v["interactions"][0]["qubits"] = json!([0, 99]);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_arity_one() {
    let mut v = valid_graph_value();
    v["interactions"][0]["qubits"] = json!([0]);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_uncanonical_qubits() {
    let mut v = valid_graph_value();
    // Unsorted: [1, 0] is not canonical for a 2-qubit interaction.
    v["interactions"][0]["qubits"] = json!([1, 0]);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_duplicate_vertex() {
    let mut v = valid_graph_value();
    v["vertices"] = json!([0, 0, 1, 2]);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_duplicate_interaction_id() {
    let mut v = valid_graph_value();
    let id0 = v["interactions"][0]["id"].clone();
    v["interactions"][1]["id"] = id0;
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_self_loop_edge() {
    let mut v = valid_graph_value();
    v["edges"][0]["a"] = v["edges"][0]["b"].clone();
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_unordered_edge() {
    let mut v = valid_graph_value();
    let a = v["edges"][0]["a"].clone();
    let b = v["edges"][0]["b"].clone();
    // Swap so that a > b.
    v["edges"][0]["a"] = b;
    v["edges"][0]["b"] = a;
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_unknown_endpoint() {
    let mut v = valid_graph_value();
    v["edges"][0]["a"] = json!(99);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_negative_edge_weight() {
    let mut v = valid_graph_value();
    v["edges"][0]["weight"] = json!(-1.0);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_edge_referencing_unknown_interaction() {
    let mut v = valid_graph_value();
    v["edges"][0]["interactions"] = json!([99]);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_segment_unknown_interaction() {
    let mut v = valid_graph_value();
    v["segments"][0]["interactions"] = json!([99]);
    assert_rejects(v);
}

#[test]
fn deserialize_rejects_unpartitioned_interaction() {
    let mut v = valid_graph_value();
    // Triangle has interactions {0,1,2} in one segment; drop interaction 2.
    v["segments"][0]["interactions"] = json!([0, 1]);
    assert_rejects(v);
}

/// Generate (n, deduped simple edges) for an Erdős–Rényi-style commutation graph.
fn er_strategy() -> impl Strategy<Value = (u32, Vec<(u32, u32)>)> {
    (2u32..=24).prop_flat_map(|n| {
        prop::collection::vec((0..n, 0..n), 0..(n as usize * 2)).prop_map(move |raw| {
            let edges: Vec<(u32, u32)> = raw
                .into_iter()
                .filter(|(a, b)| a != b)
                .map(|(a, b)| if a < b { (a, b) } else { (b, a) })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            (n, edges)
        })
    })
}

proptest! {
    /// JSON round-trip of a validated graph preserves every invariant: the
    /// deserialized graph is structurally equal to the original, proving
    /// deserialization reuses the validated constructor path.
    #[test]
    fn json_round_trip_preserves_invariants((n, edges) in er_strategy()) {
        let graph = erdos_renyi_commutation_graph(n, &edges).expect("valid ER graph");
        let value = serde_json::to_value(&graph).expect("serialize");
        let parsed: InteractionGraph = serde_json::from_value(value).expect("deserialize");
        prop_assert_eq!(parsed, graph);
    }

    /// Cubic (3-regular) commutation graphs — a denser, always-valid fixture —
    /// also round-trip losslessly through JSON.
    #[test]
    fn cubic_graph_round_trips(n in (4u32..=20).prop_filter("even", |n| n % 2 == 0)) {
        let graph = cubic_commutation_graph(n).expect("cubic graph");
        let value = serde_json::to_value(&graph).expect("serialize");
        let parsed: InteractionGraph = serde_json::from_value(value).expect("deserialize");
        prop_assert_eq!(parsed, graph);
    }
}
