//! Interaction graph for neutral-atom compilation (issue #103).
//!
//! Vertices are logical qubits (future code-block scheduling units) — or, in
//! the hybrid QEC path (#318), **physical atoms** indexed by [`AtomVertexId`].
//! The graph is generic over a thin [`VertexId`] label `V` (default
//! [`LogicalQubitId`]) so the bare Quon-program path compiles unchanged while
//! the hybrid QEC schedule feeds atom identities without a
//! `LogicalQubitId(atom.0)` numeric cast. Edges are undirected pairwise
//! interactions weighted with Atomique's layer-decayed gate-frequency formula
//! `Σ γ^l` ([Atomique] Sec. III-A; default [`DEFAULT_GAMMA`] = 0.8). Segments
//! preserve Enola's distinction between commutation groups and ordered
//! dependency DAGs ([Enola] Sec. 3) so later Misra–Gries edge-coloring (#105)
//! can apply the right bound.
//!
//! See `docs/neutral_atom/architecture_model.md` §4 and
//! `docs/adr/0029-atom-indexed-hybrid-interaction-graph.md`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Backend-only logical qubit / future code-block scheduling unit.
///
/// Owned by [`quon_qec::LogicalQubitId`] (CONTEXT "Logical qubit"); re-exported
/// here so the interaction-graph layer shares one newtype with the QEC workload IR.
pub use quon_qec::LogicalQubitId;
use quon_qec::PhysicalAtomId;

/// Vertex identity carried by an [`InteractionGraph`]: a thin, densely-indexed
/// label whose [`VertexId::index`] is the value the NA layout / schedule layer
/// uses as `AtomId(self.index())`.
///
/// The bound mirrors exactly what the graph and its schedule-request DTOs
/// require — copyable, orderable, hashable, and serde round-trippable — so
/// [`InteractionGraph<V>`], [`Interaction<V>`], [`InteractionEdge<V>`], and
/// the request/results that hold them stay serializable for any `V`.
///
/// Implemented for [`LogicalQubitId`] (the bare Quon-program path; the default
/// `V`) and [`AtomVertexId`] (the hybrid QEC path, #318). See ADR-0029.
pub trait VertexId:
    Copy
    + Clone
    + std::fmt::Debug
    + PartialEq
    + Eq
    + PartialOrd
    + Ord
    + std::hash::Hash
    + Serialize
    + serde::de::DeserializeOwned
{
    /// Dense `u32` index — `AtomId(self.index())` in the NA layout / schedule.
    fn index(self) -> u32;
}

impl VertexId for LogicalQubitId {
    #[inline]
    fn index(self) -> u32 {
        self.0
    }
}

/// Atom identity as a vertex in the hybrid QEC interaction graph (#318).
///
/// Distinct from [`LogicalQubitId`]: the hybrid QEC schedule feeds *physical
/// atoms* (post-[`quon_qec::expand_workload`] [`PhysicalAtomId`]s) into the
/// interaction graph, not logical qubits. This newtype names that physical
/// identity at the place/entangle seam so placers/schedulers operate on atoms
/// directly instead of through a `LogicalQubitId(atom.0)` numeric cast — the
/// cast hid a missing seam between "logical qubit" and "atom" (CONTEXT).
///
/// Structurally identical to `LogicalQubitId(pub u32)`; the two never share a
/// call site because the hybrid path fixes `V = AtomVertexId` while the bare
/// path keeps the default `V = LogicalQubitId`. At the schedule-artifact
/// boundary the hybrid path projects `AtomVertexId -> LogicalQubitId` (the
/// vertex id is just a label by then — atom identity is baked into the
/// schedule actions as `AtomId`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomVertexId(pub u32);

impl AtomVertexId {
    /// Wrap a physical atom id as a graph vertex.
    #[inline]
    pub fn from_atom(atom: PhysicalAtomId) -> Self {
        Self(atom.0)
    }

    /// Recover the physical atom id this vertex labels.
    #[inline]
    pub fn atom(self) -> PhysicalAtomId {
        PhysicalAtomId(self.0)
    }

    /// Dense `u32` index (== [`VertexId::index`]).
    #[inline]
    pub fn index(self) -> u32 {
        self.0
    }
}

impl From<PhysicalAtomId> for AtomVertexId {
    #[inline]
    fn from(atom: PhysicalAtomId) -> Self {
        Self(atom.0)
    }
}

impl VertexId for AtomVertexId {
    #[inline]
    fn index(self) -> u32 {
        self.0
    }
}

/// Dense identifier for one multi-qubit entangling interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionId(pub u32);

/// One multi-qubit entangling interaction (arity ≥ 2).
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); see
/// [`InteractionGraph`]. `qubits` carries `V` ids, so the hybrid QEC path
/// (#318) stores [`AtomVertexId`]s here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interaction<V = LogicalQubitId> {
    pub id: InteractionId,
    /// Canonical sorted unique vertex ids (len ≥ 2).
    pub qubits: Vec<V>,
    pub gate_name: String,
    /// ASAP dependency-DAG layer index within the enclosing segment (0-based).
    pub dag_layer: u32,
    /// True iff this gate lies on some longest path of the 2Q+ dependency DAG
    /// within its segment.
    pub on_critical_path: bool,
}

/// How interactions in a segment may be ordered for entangling-layer scheduling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    /// Order is free among interactions; #105 may edge-color the conflict graph
    /// and cite Enola Thm. 1 (≤ S_opt + 1) when the group is a true commutation set.
    CommutationGroup,
    /// Ordered dependency DAG; critical path is a lower bound; ASAP is optimal
    /// for stage count within the segment (Enola Sec. 3 ordered case).
    DependencyDag,
}

/// A barrier-bounded (or synthetic) group of interactions.
///
/// Non-generic: interaction ids are dense `u32` labels independent of the
/// vertex label `V`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionSegment {
    pub kind: SegmentKind,
    pub interactions: Vec<InteractionId>,
}

/// Undirected pairwise edge with Atomique-style layer-decayed weight.
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); `a`/`b`
/// carry `V` ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionEdge<V = LogicalQubitId> {
    pub a: V,
    pub b: V,
    /// `Σ_g γ^{dag_layer(g)}` over interactions that touch both endpoints.
    pub weight: f64,
    /// Interaction ids that contributed to this edge.
    pub interactions: Vec<InteractionId>,
}

/// Weighted interaction graph extracted from a circuit or built synthetically.
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]). The bare
/// Quon-program path uses the default; the hybrid QEC path (#318) uses
/// `InteractionGraph<AtomVertexId>` so placers/schedulers name physical atoms,
/// not logical qubits. See ADR-0029.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionGraph<V = LogicalQubitId> {
    pub vertices: Vec<V>,
    pub interactions: Vec<Interaction<V>>,
    pub edges: Vec<InteractionEdge<V>>,
    pub segments: Vec<InteractionSegment>,
    /// Decay base used for weights; recorded for reproducibility.
    pub gamma: f64,
}

/// Default Atomique layer-decay base ([Atomique] Sec. III-A).
pub const DEFAULT_GAMMA: f64 = 0.8;

/// Structural problems with an [`InteractionGraph`].
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); the
/// vertex-carrying variants hold `V`.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum GraphError<V = LogicalQubitId> {
    #[error("interaction {0:?} has fewer than 2 qubits")]
    Arity(InteractionId),
    #[error("interaction {0:?} has unsorted or duplicate qubits")]
    UncanonicalQubits(InteractionId),
    #[error("duplicate vertex id {0:?}")]
    DuplicateVertex(V),
    #[error("interaction {0:?} references unknown vertex {1:?}")]
    UnknownQubit(InteractionId, V),
    #[error("edge endpoints not in vertex set: {0:?}, {1:?}")]
    UnknownEndpoint(V, V),
    #[error("self-loop on {0:?}")]
    SelfLoop(V),
    #[error("edge endpoints must be ordered a < b, got {0:?}, {1:?}")]
    UnorderedEdge(V, V),
    #[error("duplicate interaction id {0:?}")]
    DuplicateInteraction(InteractionId),
    #[error("segment references unknown interaction {0:?}")]
    UnknownInteraction(InteractionId),
    #[error("interaction {0:?} is not covered by exactly one segment")]
    UnpartitionedInteraction(InteractionId),
    #[error("gamma must be in (0, 1], got {0}")]
    InvalidGamma(f64),
    #[error("edge weight must be finite and non-negative, got {0}")]
    InvalidWeight(f64),
    #[error("cubic graph requires even n ≥ 4, got {0}")]
    InvalidCubicOrder(u32),
}

impl<V: VertexId> InteractionGraph<V> {
    /// Build a graph from interactions, computing pairwise edges with `γ^l` weights.
    ///
    /// For k-qubit gates (k > 2), every unordered pair among the k qubits receives
    /// the interaction's weight contribution (complete subgraph).
    pub fn from_interactions(
        vertices: Vec<V>,
        interactions: Vec<Interaction<V>>,
        segments: Vec<InteractionSegment>,
        gamma: f64,
    ) -> Result<Self, GraphError<V>> {
        let edges = aggregate_edges(&interactions, gamma)?;
        let graph = Self {
            vertices,
            interactions,
            edges,
            segments,
            gamma,
        };
        graph.validate()?;
        Ok(graph)
    }

    /// Validate structural invariants.
    pub fn validate(&self) -> Result<(), GraphError<V>> {
        if !(self.gamma > 0.0 && self.gamma <= 1.0) {
            return Err(GraphError::InvalidGamma(self.gamma));
        }

        let mut seen_vertices = BTreeSet::new();
        for &v in &self.vertices {
            if !seen_vertices.insert(v) {
                return Err(GraphError::DuplicateVertex(v));
            }
        }

        let mut interaction_ids = BTreeSet::new();
        for interaction in &self.interactions {
            if !interaction_ids.insert(interaction.id) {
                return Err(GraphError::DuplicateInteraction(interaction.id));
            }
            if interaction.qubits.len() < 2 {
                return Err(GraphError::Arity(interaction.id));
            }
            let mut sorted_unique = interaction.qubits.clone();
            sorted_unique.sort();
            sorted_unique.dedup();
            if sorted_unique != interaction.qubits {
                return Err(GraphError::UncanonicalQubits(interaction.id));
            }
            for &q in &interaction.qubits {
                if !seen_vertices.contains(&q) {
                    return Err(GraphError::UnknownQubit(interaction.id, q));
                }
            }
        }

        for edge in &self.edges {
            if edge.a == edge.b {
                return Err(GraphError::SelfLoop(edge.a));
            }
            if edge.a >= edge.b {
                return Err(GraphError::UnorderedEdge(edge.a, edge.b));
            }
            if !seen_vertices.contains(&edge.a) || !seen_vertices.contains(&edge.b) {
                return Err(GraphError::UnknownEndpoint(edge.a, edge.b));
            }
            if !edge.weight.is_finite() || edge.weight < 0.0 {
                return Err(GraphError::InvalidWeight(edge.weight));
            }
            for &id in &edge.interactions {
                if !interaction_ids.contains(&id) {
                    return Err(GraphError::UnknownInteraction(id));
                }
            }
        }

        let mut covered = BTreeMap::<InteractionId, u32>::new();
        for segment in &self.segments {
            for &id in &segment.interactions {
                if !interaction_ids.contains(&id) {
                    return Err(GraphError::UnknownInteraction(id));
                }
                *covered.entry(id).or_insert(0) += 1;
            }
        }
        for &id in &interaction_ids {
            match covered.get(&id).copied().unwrap_or(0) {
                1 => {}
                _ => return Err(GraphError::UnpartitionedInteraction(id)),
            }
        }

        Ok(())
    }

    /// Serialize to a JSON value.
    pub fn to_json_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }

    /// Pretty-printed JSON string.
    pub fn to_json_string_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Export as an undirected DOT graph.
    ///
    /// Node labels are vertex indices; edge labels are weights (3 decimal
    /// places). Edges with any critical-path contributor are drawn thicker.
    pub fn to_dot(&self) -> String {
        let mut out = String::from("graph InteractionGraph {\n");
        for v in &self.vertices {
            out.push_str(&format!("  q{} [label=\"{}\"];\n", v.index(), v.index()));
        }
        let critical: BTreeSet<InteractionId> = self
            .interactions
            .iter()
            .filter(|i| i.on_critical_path)
            .map(|i| i.id)
            .collect();
        for edge in &self.edges {
            let is_critical = edge.interactions.iter().any(|id| critical.contains(id));
            let attrs = if is_critical {
                format!("label=\"{:.3}\", penwidth=2.5", edge.weight)
            } else {
                format!("label=\"{:.3}\"", edge.weight)
            };
            out.push_str(&format!(
                "  q{} -- q{} [{attrs}];\n",
                edge.a.index(),
                edge.b.index()
            ));
        }
        out.push_str("}\n");
        out
    }
}

/// Aggregate pairwise edges from interactions using Atomique `γ^l` weights.
pub fn aggregate_edges<V: VertexId>(
    interactions: &[Interaction<V>],
    gamma: f64,
) -> Result<Vec<InteractionEdge<V>>, GraphError<V>> {
    if !(gamma > 0.0 && gamma <= 1.0) {
        return Err(GraphError::InvalidGamma(gamma));
    }

    let mut acc: BTreeMap<(V, V), (f64, Vec<InteractionId>)> = BTreeMap::new();

    for interaction in interactions {
        if interaction.qubits.len() < 2 {
            return Err(GraphError::Arity(interaction.id));
        }
        let contribution = gamma.powi(interaction.dag_layer as i32);
        for i in 0..interaction.qubits.len() {
            for j in (i + 1)..interaction.qubits.len() {
                let mut a = interaction.qubits[i];
                let mut b = interaction.qubits[j];
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                if a == b {
                    return Err(GraphError::SelfLoop(a));
                }
                let entry = acc.entry((a, b)).or_insert((0.0, Vec::new()));
                entry.0 += contribution;
                if !entry.1.contains(&interaction.id) {
                    entry.1.push(interaction.id);
                }
            }
        }
    }

    Ok(acc
        .into_iter()
        .map(|((a, b), (weight, interactions))| InteractionEdge {
            a,
            b,
            weight,
            interactions,
        })
        .collect())
}

/// Mark critical-path interactions in a dependency-DAG segment.
///
/// ASAP layers are assumed already stored on each interaction. A gate is on the
/// critical path iff `asap[g] + longest_suffix[g] == global_max_depth`.
pub fn mark_critical_path<V: VertexId>(interactions: &mut [Interaction<V>]) {
    let n = interactions.len();
    if n == 0 {
        return;
    }

    let mut preds: Vec<Vec<usize>> = vec![vec![]; n];
    let mut succs: Vec<Vec<usize>> = vec![vec![]; n];
    let mut last_on_qubit: BTreeMap<V, usize> = BTreeMap::new();

    for (index, gate) in interactions.iter().enumerate() {
        for &q in &gate.qubits {
            if let Some(&pred) = last_on_qubit.get(&q) {
                preds[index].push(pred);
            }
        }
        for &q in &gate.qubits {
            last_on_qubit.insert(q, index);
        }
    }
    for (index, gate_preds) in preds.iter().enumerate() {
        for &pred in gate_preds {
            succs[pred].push(index);
        }
    }

    let mut asap = vec![0u32; n];
    for index in 0..n {
        let earliest = preds[index]
            .iter()
            .map(|&pred| asap[pred] + 1)
            .max()
            .unwrap_or(0);
        asap[index] = earliest;
        interactions[index].dag_layer = earliest;
    }

    let mut longest_suffix = vec![0u32; n];
    for index in (0..n).rev() {
        let suffix = succs[index]
            .iter()
            .map(|&succ| 1 + longest_suffix[succ])
            .max()
            .unwrap_or(0);
        longest_suffix[index] = suffix;
    }

    let global_max = asap
        .iter()
        .zip(longest_suffix.iter())
        .map(|(a, s)| a + s)
        .max()
        .unwrap_or(0);

    for index in 0..n {
        interactions[index].on_critical_path = asap[index] + longest_suffix[index] == global_max;
    }
}

/// Assign ASAP `dag_layer` and critical-path flags for a dependency-DAG segment.
pub fn schedule_dependency_segment<V: VertexId>(interactions: &mut [Interaction<V>]) {
    mark_critical_path(interactions);
}

/// Build a synthetic 3-regular (cubic) interaction graph for stress tests.
///
/// `n` must be even and ≥ 4. Uses a circular-ladder (prism) construction that
/// is always 3-regular. All interactions sit in one [`SegmentKind::CommutationGroup`]
/// with `dag_layer = 0` and `on_critical_path = false` (critical path is
/// meaningless in a pure commutation group). Returns the default-vertex
/// (`LogicalQubitId`) graph.
pub fn cubic_commutation_graph(n: u32) -> Result<InteractionGraph, GraphError> {
    if n < 4 || !n.is_multiple_of(2) {
        return Err(GraphError::InvalidCubicOrder(n));
    }
    let edges = cubic_edges(n as usize);
    let vertices: Vec<LogicalQubitId> = (0..n).map(LogicalQubitId).collect();
    let mut interactions = Vec::with_capacity(edges.len());
    let mut ids = Vec::with_capacity(edges.len());
    for (i, (a, b)) in edges.into_iter().enumerate() {
        let id = InteractionId(i as u32);
        ids.push(id);
        interactions.push(Interaction {
            id,
            qubits: vec![LogicalQubitId(a as u32), LogicalQubitId(b as u32)],
            gate_name: "CZ".to_string(),
            dag_layer: 0,
            on_critical_path: false,
        });
    }
    InteractionGraph::from_interactions(
        vertices,
        interactions,
        vec![InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: ids,
        }],
        DEFAULT_GAMMA,
    )
}

fn cubic_edges(n: usize) -> Vec<(usize, usize)> {
    // Circulant C_n(1, n/2): each vertex joins ±1 and antipode n/2 — 3-regular
    // for even n ≥ 4.
    let mut edges = BTreeSet::new();
    let half = n / 2;
    for i in 0..n {
        edges.insert(order_pair(i, (i + 1) % n));
        edges.insert(order_pair(i, (i + half) % n));
    }
    edges.into_iter().collect()
}

fn order_pair(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

/// Build an Erdős–Rényi-style simple graph as a commutation-group interaction graph.
///
/// Returns the default-vertex (`LogicalQubitId`) graph.
pub fn erdos_renyi_commutation_graph(
    n: u32,
    edges: &[(u32, u32)],
) -> Result<InteractionGraph, GraphError> {
    let vertices: Vec<LogicalQubitId> = (0..n).map(LogicalQubitId).collect();
    let mut unique = BTreeSet::new();
    for &(a, b) in edges {
        if a == b || a >= n || b >= n {
            continue;
        }
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        unique.insert((lo, hi));
    }
    let mut interactions = Vec::new();
    let mut ids = Vec::new();
    for (i, &(a, b)) in unique.iter().enumerate() {
        let id = InteractionId(i as u32);
        ids.push(id);
        interactions.push(Interaction {
            id,
            qubits: vec![LogicalQubitId(a), LogicalQubitId(b)],
            gate_name: "CZ".to_string(),
            dag_layer: 0,
            on_critical_path: false,
        });
    }
    let segments = if ids.is_empty() {
        Vec::new()
    } else {
        vec![InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: ids,
        }]
    };
    InteractionGraph::from_interactions(vertices, interactions, segments, DEFAULT_GAMMA)
}
