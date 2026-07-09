//! Extract an [`InteractionGraph`] from lowered `quantum.dynamic` IR.
//!
//! Walk pattern mirrors `mlir_bridge::metrics` / `depth_scheduling`: recurse
//! into `unitary_region` and both `if` arms; barriers flush
//! [`SegmentKind::DependencyDag`] segments; multi-qubit `quantum.circ.gate`
//! ops become interactions. ASAP layers and critical-path marks follow Enola's
//! ordered-DAG case; edge weights use Atomique `γ^l`.
//!
//! Keep the collect loop in sync with those mlir_bridge walks (short-term
//! duplication; a shared collector may land later).

use std::collections::{BTreeSet, HashMap};

use melior::ir::attribute::StringAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, Module, OperationRef, RegionLike, Value, ValueLike};
use thiserror::Error;

use mlir_bridge::dialect::{quantum_circ, quantum_dynamic};

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

    // Top-level executed program (post-monadic-lowering), same as metrics.
    extract_block(
        body,
        &mut interactions,
        &mut segments,
        &mut next_id,
        &mut used_qubits,
    );

    // Named `quantum.circ.func`s — used by standalone lit-style fixtures (same
    // dual path as depth_scheduling). Roots are per-func block-arg indices.
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
        );
    }

    let vertices: Vec<LogicalQubitId> = used_qubits.into_iter().collect();
    Ok(InteractionGraph::from_interactions(
        vertices,
        interactions,
        segments,
        gamma,
    )?)
}

fn extract_block<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
    interactions: &mut Vec<Interaction>,
    segments: &mut Vec<InteractionSegment>,
    next_id: &mut u32,
    used_qubits: &mut BTreeSet<LogicalQubitId>,
) {
    let mut tracker = LocalWireTracker::new();
    tracker.seed_block_args(&block);
    for index in 0..block.argument_count() {
        if let Ok(argument) = block.argument(index) {
            let value = Value::from(argument);
            if quantum_circ::is_qubit_type(value.r#type()) {
                used_qubits.insert(LogicalQubitId(index as u32));
            }
        }
    }

    let mut raw_segments: Vec<Vec<RawGate>> = vec![Vec::new()];
    collect_gates(block, &mut tracker, &mut raw_segments);

    for raw in raw_segments {
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

/// Local copy of mlir_bridge's WireTracker (crate-private there).
/// Keep root-aliasing semantics aligned with `qubit_wiring::WireTracker`.
struct LocalWireTracker {
    roots: HashMap<usize, usize>,
}

impl LocalWireTracker {
    fn new() -> Self {
        Self {
            roots: HashMap::new(),
        }
    }

    fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
        value.to_raw().ptr as usize
    }

    fn seed_block_args<'c, 'a, B: BlockLike<'c, 'a>>(&mut self, block: &B) {
        for index in 0..block.argument_count() {
            if let Ok(argument) = block.argument(index) {
                let value = Value::from(argument);
                if quantum_circ::is_qubit_type(value.r#type()) {
                    self.roots.insert(Self::value_key(&value), index);
                }
            }
        }
    }

    fn root<'c, 'a>(&mut self, value: Value<'c, 'a>) -> usize {
        let key = Self::value_key(&value);
        *self.roots.entry(key).or_insert(key)
    }

    fn alias<'c, 'a>(&mut self, value: Value<'c, 'a>, root: usize) {
        self.roots.insert(Self::value_key(&value), root);
    }

    fn qubit_operands<'c, 'a>(operation: OperationRef<'c, 'a>) -> Vec<Value<'c, 'a>> {
        operation
            .operands()
            .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
            .collect()
    }

    fn qubit_results<'c, 'a>(operation: OperationRef<'c, 'a>) -> Vec<Value<'c, 'a>> {
        operation
            .results()
            .filter(|result| quantum_circ::is_qubit_type(result.r#type()))
            .map(Value::from)
            .collect()
    }

    fn roots_for_operands<'c, 'a>(&mut self, operation: OperationRef<'c, 'a>) -> Vec<usize> {
        Self::qubit_operands(operation)
            .into_iter()
            .map(|value| self.root(value))
            .collect()
    }

    fn observe_operation<'c, 'a>(&mut self, operation: OperationRef<'c, 'a>) {
        let operand_roots = self.roots_for_operands(operation);
        let results = Self::qubit_results(operation);
        if operand_roots.len() == results.len() {
            for (result, root) in results.into_iter().zip(operand_roots) {
                self.roots.insert(Self::value_key(&result), root);
            }
        } else if operand_roots.is_empty() {
            for result in results {
                let key = Self::value_key(&result);
                self.roots.insert(key, key);
            }
        }
    }
}

fn collect_gates<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
    tracker: &mut LocalWireTracker,
    segments: &mut Vec<Vec<RawGate>>,
) {
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN || name == quantum_dynamic::op::YIELD {
            break;
        }
        if name == quantum_dynamic::op::BARRIER {
            tracker.observe_operation(current);
            segments.push(Vec::new());
            continue;
        }
        if name == quantum_dynamic::op::UNITARY_REGION {
            recurse_region(current, 0, tracker, segments);
            continue;
        }
        if name == quantum_dynamic::op::IF {
            // Both arms (conservative), consistent with metrics.
            recurse_region(current, 0, tracker, segments);
            recurse_region(current, 1, tracker, segments);
            continue;
        }
        if name == quantum_dynamic::op::MEASURE || name == quantum_dynamic::op::RESET {
            tracker.observe_operation(current);
            continue;
        }
        if name != quantum_circ::op::GATE {
            tracker.observe_operation(current);
            continue;
        }

        let roots = tracker.roots_for_operands(current);
        tracker.observe_operation(current);
        if roots.len() < 2 {
            continue;
        }
        let gate_name =
            read_string_attr(&current, quantum_circ::attr::GATE_NAME).unwrap_or_default();
        let qubits = roots
            .into_iter()
            .map(|root| LogicalQubitId(root as u32))
            .collect();
        if let Some(segment) = segments.last_mut() {
            segment.push(RawGate { qubits, gate_name });
        }
    }
}

fn recurse_region<'c, 'a>(
    op: OperationRef<'c, 'a>,
    region_index: usize,
    tracker: &mut LocalWireTracker,
    segments: &mut Vec<Vec<RawGate>>,
) {
    let operand_roots = tracker.roots_for_operands(op);
    let Ok(region) = op.region(region_index) else {
        return;
    };
    let Some(inner_block) = region.first_block() else {
        return;
    };
    for (index, root) in operand_roots.iter().enumerate() {
        if let Ok(argument) = inner_block.argument(index) {
            tracker.alias(Value::from(argument), *root);
        }
    }
    collect_gates(inner_block, tracker, segments);
    let results = LocalWireTracker::qubit_results(op);
    for (result, root) in results.into_iter().zip(operand_roots.iter()) {
        tracker.alias(result, *root);
    }
}
