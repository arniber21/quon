//! Extract an [`InteractionGraph`] from lowered `quantum.dynamic` IR.
//!
//! Uses [`mlir_bridge::dynamic_walk`] (same walker as metrics / depth
//! scheduling): recurse into `unitary_region` and both `if` arms; barriers
//! flush [`SegmentKind::DependencyDag`] segments; multi-qubit
//! `quantum.circ.gate` ops become interactions. ASAP layers and critical-path
//! marks follow Enola's ordered-DAG case; edge weights use Atomique `γ^l`.
//!
//! Prefer the module top-level body (post-monadic executed program). Named
//! `quantum.circ.func`s are a fallback for standalone circ-only fixtures — never
//! merged with a non-empty top-level extract (avoids double-counting inlined
//! callees). Logical qubit ids are densified to `0..n` after extraction.
//!
//! Wire-tracker roots for qubits allocated inside a block are raw SSA value
//! addresses, so ids are densified in **first-appearance order** during the
//! walk (block arguments first, then gate operands in program order) — never
//! by sorting the raw roots, which would make qubit numbering depend on
//! allocator layout and vary run to run.

use std::collections::{BTreeSet, HashMap};

use melior::ir::attribute::{FloatAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, Module, OperationRef, RegionLike, Value, ValueLike};
use thiserror::Error;

use mlir_bridge::dialect::quantum_circ;
use mlir_bridge::dynamic_walk::{self, DynamicVisitor};

use crate::graph::{
    DEFAULT_GAMMA, GraphError, Interaction, InteractionGraph, InteractionId, InteractionSegment,
    LogicalQubitId, SegmentKind, schedule_dependency_segment,
};

/// Errors while extracting an interaction graph from MLIR.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ExtractError {
    #[error("module has no body region")]
    EmptyModule,
    #[error(transparent)]
    Graph(#[from] GraphError),
}

/// A single-qubit `quantum.circ.gate` captured during extraction (issue #298).
///
/// Previously, extraction silently dropped every 1-qubit gate (arity < 2
/// isn't an "interaction" for entangling-layer scheduling purposes) — this
/// is the gap issue #298 closes. `after` anchors the gate to its position in
/// program order relative to the (unchanged, ≥2-qubit-only) interaction
/// graph: the id of the most recently extracted interaction that touched
/// `qubit`, or `None` if no interaction on `qubit` precedes it. Consumers
/// (`quon_na::pipeline::interleave_local_gates`) use this to splice the
/// gate's decomposed schedule actions into the right layer.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalGateExtract {
    pub qubit: LogicalQubitId,
    pub gate_name: String,
    pub angle: Option<f64>,
    pub after: Option<InteractionId>,
}

/// Extract with [`DEFAULT_GAMMA`], discarding any 1-qubit gates.
///
/// Kept for existing graph-only callers/tests; prefer
/// [`extract_interaction_graph_and_local_gates`] in the production `quonc`
/// pipeline so 1-qubit gates aren't silently dropped (issue #298).
pub fn extract_interaction_graph<'c>(
    module: &Module<'c>,
) -> Result<InteractionGraph, ExtractError> {
    extract_interaction_graph_with_gamma(module, DEFAULT_GAMMA)
}

/// Extract with a caller-supplied decay base `gamma ∈ (0, 1]`, discarding any
/// 1-qubit gates. See [`extract_interaction_graph`].
pub fn extract_interaction_graph_with_gamma<'c>(
    module: &Module<'c>,
    gamma: f64,
) -> Result<InteractionGraph, ExtractError> {
    let (graph, _local_gates) =
        extract_interaction_graph_and_local_gates_with_gamma(module, gamma)?;
    Ok(graph)
}

/// Extract with [`DEFAULT_GAMMA`], also returning captured 1-qubit gates.
pub fn extract_interaction_graph_and_local_gates<'c>(
    module: &Module<'c>,
) -> Result<(InteractionGraph, Vec<LocalGateExtract>), ExtractError> {
    extract_interaction_graph_and_local_gates_with_gamma(module, DEFAULT_GAMMA)
}

/// Extract with a caller-supplied decay base `gamma ∈ (0, 1]`, also returning
/// captured 1-qubit gates (issue #298) in program order.
pub fn extract_interaction_graph_and_local_gates_with_gamma<'c>(
    module: &Module<'c>,
    gamma: f64,
) -> Result<(InteractionGraph, Vec<LocalGateExtract>), ExtractError> {
    let Some(body) = module
        .as_operation()
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return Err(ExtractError::EmptyModule);
    };

    let mut interactions = Vec::new();
    let mut segments = Vec::new();
    let mut local_gates = Vec::new();
    let mut next_id = 0u32;
    let mut used_qubits = BTreeSet::new();
    let mut numbering = RootNumbering::default();

    // Top-level executed program (post-monadic-lowering), same as metrics.
    // After inlining, leftover `quantum.circ.func` callees are dead code — do
    // **not** merge them into the same graph (that double-counted interactions
    // and mixed block-arg ids with SSA-pointer roots).
    extract_block(
        body,
        &mut interactions,
        &mut segments,
        &mut local_gates,
        &mut next_id,
        &mut used_qubits,
        &mut numbering,
    );

    // Named `quantum.circ.func`s only when the top-level body contributed no
    // gates at all — standalone lit-style `quantum.circ.func`-only modules
    // (same fallback idea as depth_scheduling / sabre_routing). Checking both
    // `interactions` and `local_gates` (not just `interactions`) matters for a
    // program that only ever applies 1-qubit gates: before issue #298 an
    // empty `interactions` here always meant "nothing extracted", so it was
    // safe to trigger the FUNC fallback; now it can also mean "a real,
    // local-gate-only program", which must not be discarded in favor of a
    // (likely empty, since callees are inlined) FUNC re-scan.
    if interactions.is_empty() && local_gates.is_empty() {
        used_qubits.clear();
        numbering = RootNumbering::default();
        let mut op = body.first_operation();
        while let Some(current) = op {
            op = current.next_in_block();
            if op_name(&current) != quantum_circ::op::FUNC {
                continue;
            }
            let Ok(region) = current.region(0) else {
                continue;
            };
            let Some(block) = region.first_block() else {
                continue;
            };
            extract_block(
                block,
                &mut interactions,
                &mut segments,
                &mut local_gates,
                &mut next_id,
                &mut used_qubits,
                &mut numbering,
            );
        }
    }

    let (vertices, interactions, local_gates) =
        densify_logical_qubits(used_qubits, interactions, local_gates);
    let graph = InteractionGraph::from_interactions(vertices, interactions, segments, gamma)?;
    Ok((graph, local_gates))
}

/// Remap possibly sparse / pointer-derived [`LogicalQubitId`]s to dense `0..n`.
fn densify_logical_qubits(
    used_qubits: BTreeSet<LogicalQubitId>,
    mut interactions: Vec<Interaction>,
    mut local_gates: Vec<LocalGateExtract>,
) -> (Vec<LogicalQubitId>, Vec<Interaction>, Vec<LocalGateExtract>) {
    let vertices: Vec<LogicalQubitId> = used_qubits.into_iter().collect();
    let remap: HashMap<LogicalQubitId, LogicalQubitId> = vertices
        .iter()
        .enumerate()
        .map(|(i, &old)| (old, LogicalQubitId(i as u32)))
        .collect();
    for interaction in &mut interactions {
        for q in &mut interaction.qubits {
            if let Some(&mapped) = remap.get(q) {
                *q = mapped;
            }
        }
        interaction.qubits.sort();
        interaction.qubits.dedup();
    }
    for gate in &mut local_gates {
        if let Some(&mapped) = remap.get(&gate.qubit) {
            gate.qubit = mapped;
        }
    }
    let dense_vertices: Vec<LogicalQubitId> =
        (0..vertices.len() as u32).map(LogicalQubitId).collect();
    (dense_vertices, interactions, local_gates)
}

/// Densifies wire-tracker roots (raw SSA addresses or block-arg indices) to
/// stable `0..n` ids in first-appearance order.
#[derive(Default)]
struct RootNumbering {
    dense: HashMap<usize, u32>,
    next: u32,
}

impl RootNumbering {
    fn id(&mut self, root: usize) -> LogicalQubitId {
        let next = &mut self.next;
        let id = *self.dense.entry(root).or_insert_with(|| {
            let id = *next;
            *next += 1;
            id
        });
        LogicalQubitId(id)
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_block<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
    interactions: &mut Vec<Interaction>,
    segments: &mut Vec<InteractionSegment>,
    local_gates: &mut Vec<LocalGateExtract>,
    next_id: &mut u32,
    used_qubits: &mut BTreeSet<LogicalQubitId>,
    numbering: &mut RootNumbering,
) {
    // Block-argument qubits are wire-tracker roots at their argument index
    // (see `WireTracker::seed_block_args`); number them first, in order.
    for index in 0..block.argument_count() {
        if let Ok(argument) = block.argument(index) {
            let value = Value::from(argument);
            if quantum_circ::is_qubit_type(value.r#type()) {
                used_qubits.insert(numbering.id(index));
            }
        }
    }

    let mut visitor = ExtractVisitor {
        segments: vec![Vec::new()],
        numbering,
    };
    dynamic_walk::walk_block(block, &mut visitor);

    // Tracks, per qubit, the most recently extracted ≥2-qubit interaction —
    // the anchor a same-segment 1-qubit gate on that qubit attaches to
    // (issue #298). Deliberately *not* reset between segments: a barrier
    // flushes the entangling-scheduler's grouping, not the underlying
    // per-qubit program-order history a 1-qubit gate needs to anchor to.
    let mut last_interaction_by_qubit: HashMap<LogicalQubitId, InteractionId> = HashMap::new();

    for raw in visitor.segments {
        if raw.is_empty() {
            continue;
        }
        let mut segment_interactions = Vec::with_capacity(raw.len());
        let mut segment_ids = Vec::with_capacity(raw.len());
        for gate in raw {
            if gate.qubits.len() == 1 {
                let qubit = gate.qubits[0];
                used_qubits.insert(qubit);
                local_gates.push(LocalGateExtract {
                    qubit,
                    gate_name: gate.gate_name,
                    angle: gate.angle,
                    after: last_interaction_by_qubit.get(&qubit).copied(),
                });
                continue;
            }
            for &q in &gate.qubits {
                used_qubits.insert(q);
            }
            let id = InteractionId(*next_id);
            *next_id += 1;
            segment_ids.push(id);
            let mut qubits = gate.qubits;
            qubits.sort();
            qubits.dedup();
            for &q in &qubits {
                last_interaction_by_qubit.insert(q, id);
            }
            segment_interactions.push(Interaction {
                id,
                qubits,
                gate_name: gate.gate_name,
                dag_layer: 0,
                on_critical_path: false,
            });
        }
        schedule_dependency_segment(&mut segment_interactions);
        interactions.extend(segment_interactions);
        if !segment_ids.is_empty() {
            segments.push(InteractionSegment {
                kind: SegmentKind::DependencyDag,
                interactions: segment_ids,
            });
        }
    }
}

struct RawGate {
    qubits: Vec<LogicalQubitId>,
    gate_name: String,
    /// Present for parameterized single-qubit rotations (`rz`/`ry`/`u1`/...);
    /// unused for multi-qubit gates today.
    angle: Option<f64>,
}

struct ExtractVisitor<'n> {
    segments: Vec<Vec<RawGate>>,
    numbering: &'n mut RootNumbering,
}

impl<'c, 'a> DynamicVisitor<'c, 'a> for ExtractVisitor<'_> {
    fn gate(&mut self, op: OperationRef<'c, 'a>, qubit_roots: &[usize]) {
        if qubit_roots.is_empty() {
            return;
        }
        let gate_name = read_string_attr(&op, quantum_circ::attr::GATE_NAME).unwrap_or_default();
        let angle = read_f64_attr(&op, quantum_circ::attr::ANGLE);
        let qubits = qubit_roots
            .iter()
            .map(|root| self.numbering.id(*root))
            .collect();
        if let Some(segment) = self.segments.last_mut() {
            segment.push(RawGate {
                qubits,
                gate_name,
                angle,
            });
        }
    }

    fn barrier(&mut self, _op: OperationRef<'c, 'a>, _qubit_roots: &[usize]) {
        self.segments.push(Vec::new());
    }
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn read_string_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    key: &str,
) -> Option<String> {
    let value = operation.attribute(key).ok()?;
    StringAttribute::try_from(value)
        .ok()
        .map(|string| string.value().to_string())
}

fn read_f64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<f64> {
    let value = operation.attribute(key).ok()?;
    FloatAttribute::try_from(value).ok().map(|f| f.value())
}
