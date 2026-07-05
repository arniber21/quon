//! Shared SSA qubit-wire tracing helpers for optimization/physical passes.

use std::collections::HashMap;

use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, OperationRef, Value, ValueLike};

use crate::dialect::quantum_circ;

pub fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
}

pub fn qubit_operands<'c, 'a>(operation: OperationRef<'c, 'a>) -> Vec<Value<'c, 'a>> {
    operation
        .operands()
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .collect()
}

pub fn qubit_results<'c, 'a>(operation: OperationRef<'c, 'a>) -> Vec<Value<'c, 'a>> {
    operation
        .results()
        .filter(|result| quantum_circ::is_qubit_type(result.r#type()))
        .map(Value::from)
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct WireTracker {
    roots: HashMap<usize, usize>,
}

impl WireTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed_block_args<'c, 'a, B: BlockLike<'c, 'a>>(&mut self, block: &B) {
        for index in 0..block.argument_count() {
            if let Ok(argument) = block.argument(index) {
                let value = Value::from(argument);
                if quantum_circ::is_qubit_type(value.r#type()) {
                    self.roots.insert(value_key(&value), index);
                }
            }
        }
    }

    pub fn root<'c, 'a>(&mut self, value: Value<'c, 'a>) -> usize {
        let key = value_key(&value);
        *self.roots.entry(key).or_insert(key)
    }

    /// Forces `value`'s root to a caller-supplied identity, overriding the
    /// default "fresh root per unseen value" behavior of [`Self::root`].
    ///
    /// Used to thread a qubit's identity across a `quantum.dynamic.unitary_region`
    /// or `quantum.dynamic.if` boundary: the region's block argument and its
    /// corresponding outer operand are the *same* wire, so the argument must
    /// alias the operand's already-established root rather than mint a new one.
    pub fn alias<'c, 'a>(&mut self, value: Value<'c, 'a>, root: usize) {
        self.roots.insert(value_key(&value), root);
    }

    pub fn roots_for_operands<'c, 'a>(&mut self, operation: OperationRef<'c, 'a>) -> Vec<usize> {
        qubit_operands(operation)
            .into_iter()
            .map(|value| self.root(value))
            .collect()
    }

    pub fn observe_operation<'c, 'a>(&mut self, operation: OperationRef<'c, 'a>) {
        let operand_roots = self.roots_for_operands(operation);
        let results = qubit_results(operation);
        if operand_roots.len() == results.len() {
            for (result, root) in results.into_iter().zip(operand_roots) {
                self.roots.insert(value_key(&result), root);
            }
        } else if operand_roots.is_empty() {
            for result in results {
                let key = value_key(&result);
                self.roots.insert(key, key);
            }
        }
    }
}

pub fn roots_before<'c, 'a, B: BlockLike<'c, 'a>>(
    block: &B,
    target: OperationRef<'c, 'a>,
) -> Vec<usize> {
    let mut tracker = WireTracker::new();
    tracker.seed_block_args(block);
    let target_key = target.to_raw().ptr as usize;
    let mut op = block.first_operation();
    while let Some(current) = op {
        if current.to_raw().ptr as usize == target_key {
            return tracker.roots_for_operands(current);
        }
        tracker.observe_operation(current);
        op = current.next_in_block();
    }
    Vec::new()
}
