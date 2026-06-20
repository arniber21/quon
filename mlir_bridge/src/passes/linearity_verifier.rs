//! Linearity verifier pass for `quantum.circ` (SPEC §6.2, invariant 2).
//!
//! The defining invariant of the unitary dialect is *linearity*: every
//! `!qubit`-typed SSA value in a `quantum.circ.func` region must be consumed
//! exactly once. A value with **zero** uses is an implicit drop (a discarded
//! qubit, violating no-deletion); a value with **two or more** uses is an
//! implicit copy (violating no-cloning).
//!
//! This pass is a standalone region verifier. For each `quantum.circ.func` it
//! collects every qubit-typed definition — entry-block arguments and op results,
//! recursing through nested regions such as `borrow` bodies — counts how many
//! times each is used as an operand, and reports any whose use count is not
//! exactly one.
//!
//! Errors are accumulated into a [`Diagnostics`] (pure Rust) and only flushed to
//! MLIR at the single FFI boundary in [`crate::diagnostics`]. The pass signals
//! failure iff any diagnostic was produced.

use std::collections::HashMap;
use std::fmt;

use melior::ContextRef;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{BlockLike, Location, OperationRef, RegionLike, RegionRef, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};

use crate::diagnostics::Diagnostics;
use crate::dialect::quantum_circ;

/// How a qubit value entered scope — used only for diagnostic phrasing.
#[derive(Clone, Copy, Debug)]
enum DefKind {
    /// A `func`/`borrow` entry-block argument (a circuit input).
    BlockArgument,
    /// An op result (an intermediate qubit).
    Result,
}

impl fmt::Display for DefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefKind::BlockArgument => write!(f, "input"),
            DefKind::Result => write!(f, "intermediate"),
        }
    }
}

/// A qubit definition: its identity key, source location, and origin.
struct QubitDef<'c> {
    /// Stable per-value identity (the underlying MLIR value pointer).
    key: usize,
    /// Where the value is defined, for diagnostics.
    location: Location<'c>,
    /// How it was introduced.
    kind: DefKind,
}

/// A lifetime-free identity key for an SSA value.
///
/// Two `Value` handles refer to the same SSA value iff their underlying MLIR
/// pointers match. Projecting to the pointer lets us collect across borrow
/// scopes without fighting Melior's borrow lifetimes.
fn value_key<'c>(value: &impl ValueLike<'c>) -> usize {
    value.to_raw().ptr as usize
}

fn is_qubit<'c>(value: &impl ValueLike<'c>) -> bool {
    quantum_circ::is_qubit_type(value.r#type())
}

/// Verifies linearity for a single `quantum.circ.func` op, returning the
/// diagnostics it produced (empty when the region is linear).
pub fn check_linearity<'c: 'a, 'a, O: OperationLike<'c, 'a>>(func: &O) -> Diagnostics<'c> {
    let mut diagnostics = Diagnostics::new();
    let region = match func.region(0) {
        Ok(region) => region,
        Err(_) => return diagnostics,
    };

    let mut defs: Vec<QubitDef<'c>> = Vec::new();
    let mut uses: HashMap<usize, usize> = HashMap::new();
    collect_region(region, &mut defs, &mut uses);

    for def in defs {
        let count = uses.get(&def.key).copied().unwrap_or(0);
        if count != 1 {
            diagnostics.error(
                def.location,
                format!(
                    "{} qubit value has {} use(s); linearity requires exactly 1 (SPEC §6.2)",
                    def.kind, count
                ),
            );
        }
    }
    diagnostics
}

/// Recursively records qubit definitions and tallies qubit operand uses across
/// every block in `region`, descending into nested regions.
fn collect_region<'c>(
    region: RegionRef<'c, '_>,
    defs: &mut Vec<QubitDef<'c>>,
    uses: &mut HashMap<usize, usize>,
) {
    let mut block = region.first_block();
    while let Some(current) = block {
        for index in 0..current.argument_count() {
            if let Ok(argument) = current.argument(index)
                && is_qubit(&argument)
            {
                defs.push(QubitDef {
                    key: value_key(&argument),
                    location: argument.location(),
                    kind: DefKind::BlockArgument,
                });
            }
        }

        let mut operation = current.first_operation();
        while let Some(op) = operation {
            for operand in op.operands() {
                if is_qubit(&operand) {
                    *uses.entry(value_key(&operand)).or_insert(0) += 1;
                }
            }
            for result in op.results() {
                if is_qubit(&result) {
                    defs.push(QubitDef {
                        key: value_key(&result),
                        location: result.location(),
                        kind: DefKind::Result,
                    });
                }
            }
            for nested in op.regions() {
                collect_region(nested, defs, uses);
            }
            operation = op.next_in_block();
        }

        block = current.next_in_region();
    }
}

fn is_func<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> bool {
    let name = operation.name();
    name.as_string_ref().as_str() == Ok(quantum_circ::op::FUNC)
}

/// Recursively runs [`check_linearity`] on every `quantum.circ.func` reachable
/// from `operation`, accumulating diagnostics with their `'c` lifetime intact.
fn verify_module<'c>(operation: OperationRef<'c, '_>, diagnostics: &mut Diagnostics<'c>) {
    for region in operation.regions() {
        let mut block = region.first_block();
        while let Some(current) = block {
            let mut op = current.first_operation();
            while let Some(child) = op {
                if is_func(&child) {
                    diagnostics.absorb(check_linearity(&child));
                }
                verify_module(child, diagnostics);
                op = child.next_in_block();
            }
            block = current.next_in_region();
        }
    }
}

#[repr(align(8))]
struct PassId;

static LINEARITY_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct LinearityVerifier;

impl<'c> RunExternalPass<'c> for LinearityVerifier {
    fn initialize(&mut self, _context: ContextRef<'c>) {}

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        let mut diagnostics = Diagnostics::new();
        verify_module(operation, &mut diagnostics);
        if !diagnostics.emit() {
            pass.signal_failure();
        }
    }
}

/// Creates the `quantum.circ` linearity verifier pass.
pub fn create_pass() -> Pass {
    create_external(
        LinearityVerifier,
        TypeId::create(&LINEARITY_PASS_ID),
        "quantum-circ-linearity",
        "quantum-circ-linearity",
        "Verify that every !qubit value in a quantum.circ.func region has exactly one use",
        "",
        &[],
    )
}
