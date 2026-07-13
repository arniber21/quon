//! Shared `quantum.dynamic` control-flow walk (issue #208).
//!
//! `quantum.dynamic` was walked independently by the metrics collector, depth
//! scheduling, and `quon_na`'s interaction-graph extract — each reimplementing
//! the same recursive descent into `unitary_region` bodies and both `if` arms,
//! with its own copy of the qubit-wire-identity tracker needed to keep a
//! logical qubit's identity continuous across those region boundaries. This
//! module is the one walk: it owns the recursion and the
//! [`WireTracker`](crate::passes::qubit_wiring::WireTracker) threading, and
//! calls back into a [`DynamicVisitor`] for every structural element (gate,
//! barrier, measure, reset, unitary_region enter/exit, if-arm enter/exit).
//! Consumers implement aggregation, not recursion.
//!
//! Not a consumer: [`crate::passes::sabre_routing`] (it *rewrites* operands
//! and inserts SWAPs mid-walk — a mutating pass, not a pure read) and
//! [`crate::passes::measurement_deferral`] (it only extracts gates from a
//! single `if` op's two branches, never recurses into nested
//! `unitary_region`/`if`). [`crate::emit::openqasm3`] also does not consume
//! this walk: its `unitary_region`/`if`-body traversal is over `quantum.circ`
//! ops (a different dialect, with fallible reification), not the
//! `quantum.dynamic` recursion this module owns.

use melior::ir::attribute::IntegerAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, BlockRef, OperationRef, RegionLike, Value};

use crate::dialect::{quantum_circ, quantum_dynamic};
use crate::passes::qubit_wiring::{self, WireTracker};

/// Which `quantum.dynamic.if` region a walk is currently inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfArm {
    /// Region 0 — taken when the condition bit is set.
    Then,
    /// Region 1 — taken when the condition bit is clear.
    Else,
}

impl IfArm {
    /// The op region index this arm occupies (`if`'s region layout, SPEC §6.3).
    pub fn region_index(self) -> usize {
        match self {
            IfArm::Then => 0,
            IfArm::Else => 1,
        }
    }
}

/// Callbacks for one recursive descent over a `quantum.dynamic` block.
///
/// Every method defaults to a no-op, so a visitor implements only the events
/// it needs. `qubit_roots` are [`WireTracker`] root ids: stable identifiers
/// for a logical qubit's wire that survive `unitary_region`/`if` boundaries
/// (a region's block argument aliases the enclosing op's operand root), not
/// raw SSA pointer identity.
pub trait DynamicVisitor<'c, 'a> {
    /// A `quantum.circ.gate` reached by the walk — inside the top-level
    /// block, a `unitary_region` body, or an `if` arm.
    fn gate(&mut self, op: OperationRef<'c, 'a>, qubit_roots: &[usize]) {
        let _ = (op, qubit_roots);
    }

    /// A `quantum.dynamic.barrier`.
    fn barrier(&mut self, op: OperationRef<'c, 'a>, qubit_roots: &[usize]) {
        let _ = (op, qubit_roots);
    }

    /// A `quantum.dynamic.measure`.
    fn measure(&mut self, op: OperationRef<'c, 'a>) {
        let _ = op;
    }

    /// A `quantum.dynamic.reset`.
    fn reset(&mut self, op: OperationRef<'c, 'a>) {
        let _ = op;
    }

    /// Any op the walk does not otherwise dispatch on (e.g. a qubit
    /// allocation, or a foreign op in a hand-built test fixture).
    fn other(&mut self, op: OperationRef<'c, 'a>) {
        let _ = op;
    }

    /// Entering a `unitary_region`'s body, before its ops are walked.
    /// `qubit_roots` are the roots of the region op's own qubit operands.
    fn enter_unitary_region(&mut self, op: OperationRef<'c, 'a>, qubit_roots: &[usize]) {
        let _ = (op, qubit_roots);
    }

    /// Leaving a `unitary_region`'s body, after its ops are walked.
    fn exit_unitary_region(&mut self, op: OperationRef<'c, 'a>) {
        let _ = op;
    }

    /// Entering one arm of a `quantum.dynamic.if`, before its ops are walked.
    /// `qubit_roots` are the roots of the `if` op's own qubit operands (the
    /// same for both arms — they share one operand list).
    fn enter_if_arm(&mut self, op: OperationRef<'c, 'a>, arm: IfArm, qubit_roots: &[usize]) {
        let _ = (op, arm, qubit_roots);
    }

    /// Leaving one arm of a `quantum.dynamic.if`, after its ops are walked.
    fn exit_if_arm(&mut self, op: OperationRef<'c, 'a>, arm: IfArm) {
        let _ = (op, arm);
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

fn read_i32_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<i32> {
    let value = operation.attribute(key).ok()?;
    IntegerAttribute::try_from(value)
        .ok()
        .map(|integer| integer.value() as i32)
}

/// Resolves the physical/logical indices for a gate's qubit operands,
/// preferring the walk's `qubit_roots` (identity threaded across region
/// boundaries) but folding in a directly-attached `phys_qubit` attribute when
/// present and not already among the roots. Shared by [`crate::metrics`] and
/// [`crate::passes::depth_scheduling`], which both need a gate's physical
/// qubits and only differ in what they do with them.
pub fn resolve_phys_qubits<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    qubit_roots: &[usize],
) -> Vec<i32> {
    let mut phys = if qubit_roots.is_empty() {
        read_i32_attr(operation, quantum_dynamic::attr::PHYS_QUBIT)
            .map(|value| vec![value])
            .unwrap_or_default()
    } else {
        qubit_roots.iter().map(|root| *root as i32).collect()
    };
    if let Some(attr_phys) = read_i32_attr(operation, quantum_dynamic::attr::PHYS_QUBIT)
        && !phys.contains(&attr_phys)
    {
        phys.push(attr_phys);
    }
    phys
}

/// Walks `block`, seeding a fresh [`WireTracker`] from its own block
/// arguments. Use this for a module's top-level executed body, or a
/// standalone `quantum.circ.func`/module body — each is an independent qubit
/// register and gets its own tracker.
pub fn walk_block<'c, 'a>(block: BlockRef<'c, 'a>, visitor: &mut impl DynamicVisitor<'c, 'a>) {
    let mut tracker = WireTracker::new();
    tracker.seed_block_args(&block);
    walk_block_with(block, &mut tracker, visitor);
}

fn walk_block_with<'c, 'a>(
    block: BlockRef<'c, 'a>,
    tracker: &mut WireTracker,
    visitor: &mut impl DynamicVisitor<'c, 'a>,
) {
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN || name == quantum_dynamic::op::YIELD {
            break;
        }
        if name == quantum_dynamic::op::BARRIER {
            let roots = tracker.roots_for_operands(current);
            visitor.barrier(current, &roots);
            tracker.observe_operation(current);
            continue;
        }
        if name == quantum_dynamic::op::UNITARY_REGION {
            recurse_unitary_region(current, tracker, visitor);
            continue;
        }
        if name == quantum_dynamic::op::IF {
            recurse_if_arm(current, IfArm::Then, tracker, visitor);
            recurse_if_arm(current, IfArm::Else, tracker, visitor);
            continue;
        }
        if name == quantum_dynamic::op::MEASURE {
            visitor.measure(current);
            tracker.observe_operation(current);
            continue;
        }
        if name == quantum_dynamic::op::RESET {
            visitor.reset(current);
            tracker.observe_operation(current);
            continue;
        }
        if name != quantum_circ::op::GATE {
            visitor.other(current);
            tracker.observe_operation(current);
            continue;
        }
        let roots = tracker.roots_for_operands(current);
        visitor.gate(current, &roots);
        tracker.observe_operation(current);
    }
}

/// Aliases `block`'s arguments (by position) to `roots`, so a region's entry
/// block arguments resolve to the *caller's* already-established qubit
/// identity rather than minting fresh ones.
fn alias_block_args<'c, 'a>(block: &BlockRef<'c, 'a>, roots: &[usize], tracker: &mut WireTracker) {
    for (index, root) in roots.iter().enumerate() {
        if let Ok(argument) = block.argument(index) {
            tracker.alias(Value::from(argument), *root);
        }
    }
}

/// Aliases `op`'s own qubit results back to `roots`, so the wire stays
/// continuous for the rest of the walk once a region boundary closes.
fn alias_results_back<'c, 'a>(
    op: OperationRef<'c, 'a>,
    roots: &[usize],
    tracker: &mut WireTracker,
) {
    for (result, root) in qubit_wiring::qubit_results(op)
        .into_iter()
        .zip(roots.iter())
    {
        tracker.alias(result, *root);
    }
}

fn recurse_unitary_region<'c, 'a>(
    op: OperationRef<'c, 'a>,
    tracker: &mut WireTracker,
    visitor: &mut impl DynamicVisitor<'c, 'a>,
) {
    let operand_roots = tracker.roots_for_operands(op);
    visitor.enter_unitary_region(op, &operand_roots);
    if let Ok(region) = op.region(0)
        && let Some(inner_block) = region.first_block()
    {
        alias_block_args(&inner_block, &operand_roots, tracker);
        walk_block_with(inner_block, tracker, visitor);
    }
    alias_results_back(op, &operand_roots, tracker);
    visitor.exit_unitary_region(op);
}

fn recurse_if_arm<'c, 'a>(
    op: OperationRef<'c, 'a>,
    arm: IfArm,
    tracker: &mut WireTracker,
    visitor: &mut impl DynamicVisitor<'c, 'a>,
) {
    let operand_roots = tracker.roots_for_operands(op);
    visitor.enter_if_arm(op, arm, &operand_roots);
    if let Ok(region) = op.region(arm.region_index())
        && let Some(inner_block) = region.first_block()
    {
        alias_block_args(&inner_block, &operand_roots, tracker);
        walk_block_with(inner_block, tracker, visitor);
    }
    alias_results_back(op, &operand_roots, tracker);
    visitor.exit_if_arm(op, arm);
}
