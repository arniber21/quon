//! Dynamic qubit linearity verifier (SPEC §6.3, issue #6).
//!
//! Every `!qubit` SSA value in a dynamic scope must be consumed exactly once.
//! Measurement consumes a qubit permanently — a value used by both `measure` and
//! another op is reported as reuse-after-measure when applicable.
//!
//! `unitary_region` inner blocks reuse the circ linearity rules via
//! [`super::linearity_verifier::check_region_linearity`]; inner ops are not
//! folded into the outer dynamic scope.
//!
//! Note: this pass does not forbid stray `quantum.circ` ops appearing directly
//! in a dynamic scope (outside a `unitary_region`) — it treats them as ordinary
//! qubit consumers for linearity purposes. Enforcing that unitary work lives
//! only inside `unitary_region` is a separate structural rule, tracked apart
//! from linearity.

use std::collections::HashMap;
use std::fmt;

use melior::ContextRef;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{BlockLike, Location, OperationRef, RegionLike, RegionRef, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};

use crate::diagnostics::Diagnostics;
use crate::dialect::{quantum_circ, quantum_dynamic};
use quon_core::linearity::{self, UseCountViolation};

#[derive(Clone, Copy, Debug)]
enum DefKind {
    BlockArgument,
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

struct QubitDef<'c> {
    key: usize,
    location: Location<'c>,
    kind: DefKind,
}

struct QubitUse<'c> {
    user_name: String,
    location: Location<'c>,
    is_measure: bool,
}

fn value_key<'c>(value: &impl ValueLike<'c>) -> usize {
    value.to_raw().ptr as usize
}

fn is_qubit<'c>(value: &impl ValueLike<'c>) -> bool {
    quantum_circ::is_qubit_type(value.r#type())
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn is_measure_op(name: &str) -> bool {
    name == quantum_dynamic::op::MEASURE
}

/// Verifies dynamic qubit linearity within `region`, accumulating diagnostics.
pub fn check_dynamic_linearity<'c>(region: RegionRef<'c, '_>) -> Diagnostics<'c> {
    let mut diagnostics = Diagnostics::new();
    let mut defs = Vec::new();
    let mut uses: HashMap<usize, Vec<QubitUse<'c>>> = HashMap::new();
    collect_dynamic_scope(region, &mut defs, &mut uses, &mut diagnostics);
    check_scope(&defs, &uses, &mut diagnostics);
    diagnostics
}

fn collect_dynamic_scope<'c>(
    region: RegionRef<'c, '_>,
    defs: &mut Vec<QubitDef<'c>>,
    uses: &mut HashMap<usize, Vec<QubitUse<'c>>>,
    diagnostics: &mut Diagnostics<'c>,
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
            let name = op_name(&op);
            match name.as_str() {
                quantum_dynamic::op::UNITARY_REGION => {
                    record_qubit_operands(&op, uses);
                    record_qubit_results(&op, defs);
                    if let Ok(inner) = op.region(0) {
                        diagnostics.absorb(check_unitary_region_inner(inner));
                    }
                }
                quantum_dynamic::op::IF => {
                    // The condition operand is a `!quantum.bit`, not a qubit, so
                    // the type filter in `record_qubit_operands` excludes it; no
                    // need to special-case operand index 0.
                    record_qubit_operands(&op, uses);
                    record_qubit_results(&op, defs);
                    for region_index in 0..op.region_count() {
                        if let Ok(branch) = op.region(region_index) {
                            let mut branch_defs = Vec::new();
                            let mut branch_uses = HashMap::new();
                            collect_dynamic_scope(
                                branch,
                                &mut branch_defs,
                                &mut branch_uses,
                                diagnostics,
                            );
                            check_scope(&branch_defs, &branch_uses, diagnostics);
                        }
                    }
                }
                _ => {
                    record_qubit_operands(&op, uses);
                    record_qubit_results(&op, defs);
                }
            }
            operation = op.next_in_block();
        }

        block = current.next_in_region();
    }
}

fn check_scope<'c>(
    defs: &[QubitDef<'c>],
    uses: &HashMap<usize, Vec<QubitUse<'c>>>,
    diagnostics: &mut Diagnostics<'c>,
) {
    for def in defs {
        let use_list = uses.get(&def.key).map(Vec::as_slice).unwrap_or(&[]);
        let count = use_list.len();
        let has_measure = use_list.iter().any(|u| u.is_measure);
        let has_other = use_list.iter().any(|u| !u.is_measure);
        if let Some(violation) = linearity::classify_use_count(count, has_measure, has_other) {
            let message = match violation {
                UseCountViolation::ReuseAfterMeasure => {
                    // SSA use sites are unordered, so phrase this symmetrically
                    // rather than implying the measure came first: a measured
                    // qubit must not appear at any other use site.
                    let measure = use_list.iter().find(|u| u.is_measure);
                    let other = use_list.iter().find(|u| !u.is_measure);
                    format!(
                        "!qubit value is both measured by `{}` and used by `{}`; \
                         a measured qubit must not appear at any other use site (SPEC §6.3)",
                        measure.map(|u| u.user_name.as_str()).unwrap_or("measure"),
                        other.map(|u| u.user_name.as_str()).unwrap_or("consumer")
                    )
                }
                UseCountViolation::Unused | UseCountViolation::MultiUse { .. } => format!(
                    "{} qubit value has {} use(s); linearity requires exactly 1 (SPEC §6.3)",
                    def.kind, count
                ),
            };
            diagnostics.error(def.location, message);
        }
    }
}

fn record_qubit_operands<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    uses: &mut HashMap<usize, Vec<QubitUse<'c>>>,
) {
    let name = op_name(operation);
    for operand in operation.operands() {
        if is_qubit(&operand) {
            uses.entry(value_key(&operand)).or_default().push(QubitUse {
                user_name: name.clone(),
                location: operand.location(),
                is_measure: is_measure_op(&name),
            });
        }
    }
}

fn record_qubit_results<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    defs: &mut Vec<QubitDef<'c>>,
) {
    for result in operation.results() {
        if is_qubit(&result) {
            defs.push(QubitDef {
                key: value_key(&result),
                location: result.location(),
                kind: DefKind::Result,
            });
        }
    }
}

/// Linearity inside a `unitary_region` body: the inner ops obey `quantum.circ`
/// rules, so delegate to the circ linearity checker rather than duplicating the
/// collection walk.
fn check_unitary_region_inner<'c>(region: RegionRef<'c, '_>) -> Diagnostics<'c> {
    crate::passes::linearity_verifier::check_region_linearity(region)
}

fn verify_module<'c>(operation: OperationRef<'c, '_>, diagnostics: &mut Diagnostics<'c>) {
    for region in operation.regions() {
        diagnostics.absorb(check_dynamic_linearity(region));
    }
}

#[repr(align(8))]
struct PassId;

static DYNAMIC_LINEARITY_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct DynamicLinearityVerifier;

impl<'c> RunExternalPass<'c> for DynamicLinearityVerifier {
    fn initialize(&mut self, _context: ContextRef<'c>) {}

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        let mut diagnostics = Diagnostics::new();
        verify_module(operation, &mut diagnostics);
        if !diagnostics.emit() {
            pass.signal_failure();
        }
    }
}

/// Creates the `quantum.dynamic` qubit linearity verifier pass.
pub fn create_pass() -> Pass {
    create_external(
        DynamicLinearityVerifier,
        TypeId::create(&DYNAMIC_LINEARITY_PASS_ID),
        "quantum-dynamic-linearity",
        "quantum-dynamic-linearity",
        "Verify that every !qubit value in quantum.dynamic scope has exactly one use",
        "",
        &[],
    )
}
