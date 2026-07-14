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

use melior::ir::attribute::StringAttribute;
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

/// Extract with [`DEFAULT_GAMMA`].
pub fn extract_interaction_graph<'c>(
    module: &Module<'c>,
) -> Result<InteractionGraph, ExtractError> {
    extract_interaction_graph_with_gamma(module, DEFAULT_GAMMA)
}

/// Extract with a caller-supplied decay base `gamma ∈ (0, 1]`.
pub fn extract_interaction_graph_with_gamma<'c>(
    module: &Module<'c>,
    gamma: f64,
) -> Result<InteractionGraph, ExtractError> {
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
        &mut next_id,
        &mut used_qubits,
        &mut numbering,
    );

    // Named `quantum.circ.func`s only when the top-level body contributed no
    // multi-qubit gates — standalone lit-style `quantum.circ.func`-only
    // modules (same fallback idea as depth_scheduling / sabre_routing).
    if interactions.is_empty() {
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
                &mut next_id,
                &mut used_qubits,
                &mut numbering,
            );
        }
    }

    let (vertices, interactions) = densify_logical_qubits(used_qubits, interactions);
    Ok(InteractionGraph::from_interactions(
        vertices,
        interactions,
        segments,
        gamma,
    )?)
}

/// Remap possibly sparse / pointer-derived [`LogicalQubitId`]s to dense `0..n`.
fn densify_logical_qubits(
    used_qubits: BTreeSet<LogicalQubitId>,
    mut interactions: Vec<Interaction>,
) -> (Vec<LogicalQubitId>, Vec<Interaction>) {
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
    let dense_vertices: Vec<LogicalQubitId> =
        (0..vertices.len() as u32).map(LogicalQubitId).collect();
    (dense_vertices, interactions)
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

fn extract_block<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
    interactions: &mut Vec<Interaction>,
    segments: &mut Vec<InteractionSegment>,
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

    for raw in visitor.segments {
        if raw.is_empty() {
            continue;
        }
        let mut segment_interactions = Vec::with_capacity(raw.len());
        let mut segment_ids = Vec::with_capacity(raw.len());
        for gate in raw {
            for &q in &gate.qubits {
                used_qubits.insert(q);
            }
            let id = InteractionId(*next_id);
            *next_id += 1;
            segment_ids.push(id);
            let mut qubits = gate.qubits;
            qubits.sort();
            qubits.dedup();
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
        segments.push(InteractionSegment {
            kind: SegmentKind::DependencyDag,
            interactions: segment_ids,
        });
    }
}

struct RawGate {
    qubits: Vec<LogicalQubitId>,
    gate_name: String,
}

struct ExtractVisitor<'n> {
    segments: Vec<Vec<RawGate>>,
    numbering: &'n mut RootNumbering,
}

impl<'c, 'a> DynamicVisitor<'c, 'a> for ExtractVisitor<'_> {
    fn gate(&mut self, op: OperationRef<'c, 'a>, qubit_roots: &[usize]) {
        if qubit_roots.len() < 2 {
            return;
        }
        let gate_name = read_string_attr(&op, quantum_circ::attr::GATE_NAME).unwrap_or_default();
        let qubits = qubit_roots
            .iter()
            .map(|root| self.numbering.id(*root))
            .collect();
        if let Some(segment) = self.segments.last_mut() {
            segment.push(RawGate { qubits, gate_name });
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
