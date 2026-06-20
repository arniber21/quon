use zx::graph::ZXGraph;

#[test]
fn empty_zx_graph_has_no_boundary_nodes() {
    let graph = ZXGraph::new();
    assert!(graph.inputs.is_empty());
    assert!(graph.outputs.is_empty());
    assert_eq!(graph.graph.node_count(), 0);
}
