//! OpenQASM 3.0 emitter — see issue #27, SPEC.md §9.1.
//!
//! The emitter is split into a **fallible boundary** ([`reify`]) and a **total
//! renderer** (`quon_core::qasm::render`). `reify` performs the one and only
//! validation pass: it walks the `quantum.dynamic` IR, resolves every gate
//! against the target's native set, assigns physical qubit indices, and
//! allocates classical-bit indices. Its output is a `quon_core::qasm::Program`
//! that is valid by construction — so rendering it to text cannot fail and needs
//! no knowledge of what the optimization passes did (see `quon_core::qasm`).
//!
//! A gate whose name does not resolve to a target-native gate is a [`reify`]
//! error, never silently-emitted invalid QASM.

use std::collections::{HashMap, HashSet};

use backend::BackendTarget;
use melior::ir::attribute::{FloatAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, BlockRef, Module, OperationRef, RegionLike, Value, ValueLike};
use quon_core::qasm::{self, BitId, Program, QasmGate, QubitId, Stmt};
use thiserror::Error;

use crate::dialect::{quantum_circ, quantum_dynamic};

/// Errors raised while reifying `quantum.dynamic` IR into a [`Program`].
#[derive(Debug, Error)]
pub enum EmitError {
    /// A gate name does not resolve to a gate in the target's native set.
    #[error(
        "gate `{name}` is not native to target `{target}` — run native gate decomposition first"
    )]
    NonNativeGate { name: String, target: String },
    /// A gate resolved to a variant of one arity but the IR gave a different
    /// operand count.
    #[error("gate `{name}` expected {expected} qubit operand(s), found {found}")]
    ArityMismatch {
        name: String,
        expected: usize,
        found: usize,
    },
    /// A qubit SSA value was used before any allocation produced it.
    #[error("qubit value used before it was allocated")]
    UnassignedQubit,
    /// A classical-bit SSA value was used as a condition before any measure
    /// produced it.
    #[error("classical bit used before it was measured")]
    UnassignedBit,
    /// A required operand was absent on a `quantum.dynamic` op.
    #[error("{op}: missing required operand or region")]
    MissingOperand { op: &'static str },
    /// An op appeared inside a unitary region or branch that is not a gate.
    #[error("unexpected op `{0}` inside a unitary region")]
    UnexpectedOp(String),
    /// The typed QASM builder rejected an invalid statement.
    #[error("invalid QASM program: {0}")]
    InvalidProgram(#[from] qasm::QasmError),
}

// ─── Melior helpers (mirror monadic_lowering.rs) ─────────────────────────────

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
}

fn read_string_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    attr: &str,
) -> Option<String> {
    let value = operation.attribute(attr).ok()?;
    StringAttribute::try_from(value)
        .ok()
        .map(|s| s.value().to_string())
}

fn read_angle<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> f64 {
    operation
        .attribute(quantum_circ::attr::ANGLE)
        .ok()
        .and_then(|attr| FloatAttribute::try_from(attr).ok())
        .map(|f| f.value())
        .unwrap_or(0.0)
}

fn qubit_operands<'c, 'a>(operation: OperationRef<'c, 'a>) -> Vec<Value<'c, 'a>> {
    operation
        .operands()
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .collect()
}

/// True iff `operation` allocates fresh qubits: no operands, ≥1 result, all
/// results qubit-typed (the shape monadic lowering's `test.qubit` / a future
/// `qreg` op produces).
fn is_allocation<'c, 'a>(operation: OperationRef<'c, 'a>) -> bool {
    let results: Vec<_> = operation.results().collect();
    operation.operand_count() == 0
        && !results.is_empty()
        && results
            .iter()
            .all(|r| quantum_circ::is_qubit_type(r.r#type()))
}

fn is_rotation(name: &str) -> bool {
    quon_core::gates::lookup(name).is_some_and(|g| g.parametric && g.arity == 1)
}

// ─── Reification ─────────────────────────────────────────────────────────────

/// Per-emit state: SSA value → physical index maps, the running register widths
/// (fixed up front by [`count_registers`]), and the target's native gate set.
struct Reifier<'t> {
    qubits: HashMap<usize, QubitId>,
    bits: HashMap<usize, BitId>,
    num_qubits: usize,
    num_bits: usize,
    next_qubit: usize,
    next_bit: usize,
    native: HashSet<String>,
    target_id: &'t str,
}

impl Reifier<'_> {
    fn lookup_qubit<'a>(&self, value: &impl ValueLike<'a>) -> Result<QubitId, EmitError> {
        self.qubits
            .get(&value_key(value))
            .copied()
            .ok_or(EmitError::UnassignedQubit)
    }

    fn lookup_bit<'a>(&self, value: &impl ValueLike<'a>) -> Result<BitId, EmitError> {
        self.bits
            .get(&value_key(value))
            .copied()
            .ok_or(EmitError::UnassignedBit)
    }

    fn alloc_qubit<'a>(
        &mut self,
        program: &Program,
        value: &impl ValueLike<'a>,
    ) -> Result<QubitId, EmitError> {
        let id = program
            .qubit(self.next_qubit)
            .ok_or(EmitError::UnassignedQubit)?;
        self.next_qubit += 1;
        self.qubits.insert(value_key(value), id);
        Ok(id)
    }

    fn alloc_bit<'a>(
        &mut self,
        program: &Program,
        value: &impl ValueLike<'a>,
    ) -> Result<BitId, EmitError> {
        let id = program.bit(self.next_bit).ok_or(EmitError::UnassignedBit)?;
        self.next_bit += 1;
        self.bits.insert(value_key(value), id);
        Ok(id)
    }

    /// Bind a freshly-produced qubit SSA value to an existing physical index
    /// (identity threading through gates / regions / resets).
    fn thread<'a>(&mut self, result: &impl ValueLike<'a>, id: QubitId) {
        self.qubits.insert(value_key(result), id);
    }

    fn unsupported(&self, name: &str) -> EmitError {
        EmitError::NonNativeGate {
            name: name.to_string(),
            target: self.target_id.to_string(),
        }
    }

    /// Resolve a Quon gate name + physical operands into a native `QasmGate`.
    /// `Ok(None)` for the identity gate (emits nothing); an error for any name
    /// not in the target's native set.
    ///
    /// Uses [`qasm::from_gate_info`] — the single registry → QASM adapter — so a
    /// new OpenQASM spelling in `quon_core::gates::REGISTRY` emits without a
    /// second hardcoded keyword match here.
    fn resolve_gate(
        &self,
        name: &str,
        angle: f64,
        qs: &[QubitId],
    ) -> Result<Option<QasmGate>, EmitError> {
        let Some(info) = quon_core::gates::lookup(name) else {
            return Err(self.unsupported(name));
        };
        let gate = match qasm::from_gate_info(info, angle, qs) {
            Ok(gate) => gate,
            Err(qasm::QasmGateBuildError::ArityMismatch {
                keyword,
                expected,
                found,
            }) => {
                return Err(EmitError::ArityMismatch {
                    name: keyword.to_string(),
                    expected,
                    found,
                });
            }
            Err(_) => return Err(self.unsupported(name)),
        };
        // The native-set gate: every emitted keyword must be supported by the
        // target. Total for `generic_openqasm`; a restricted target turns a
        // recognized-but-unsupported gate into a clear error.
        if let Some(ref g) = gate
            && !self.native.contains(g.keyword())
        {
            return Err(self.unsupported(name));
        }
        Ok(gate)
    }

    /// Walk a `quantum.circ` body block (a `unitary_region` body or an `if`
    /// branch), emitting one statement per gate. The block's arguments bind to
    /// `arg_ids` (the physical indices of the enclosing op's qubit operands).
    /// Returns the emitted statements and the physical indices its terminator
    /// yields, in result order.
    fn reify_circ_block<'c, 'a>(
        &mut self,
        block: BlockRef<'c, 'a>,
        arg_ids: &[QubitId],
    ) -> Result<(Vec<Stmt>, Vec<QubitId>), EmitError> {
        for (index, id) in arg_ids.iter().enumerate() {
            if let Ok(arg) = block.argument(index) {
                self.thread(&arg, *id);
            }
        }

        let mut stmts = Vec::new();
        let mut outputs = Vec::new();
        let mut op = block.first_operation();
        while let Some(current) = op {
            let name = op_name(&current);
            match name.as_str() {
                quantum_circ::op::RETURN | quantum_dynamic::op::YIELD => {
                    outputs = qubit_operands(current)
                        .iter()
                        .map(|v| self.lookup_qubit(v))
                        .collect::<Result<_, _>>()?;
                }
                quantum_circ::op::GATE => {
                    let gate_name = read_string_attr(&current, quantum_circ::attr::GATE_NAME)
                        .unwrap_or_default();
                    let operands = qubit_operands(current);
                    let qs = operands
                        .iter()
                        .map(|v| self.lookup_qubit(v))
                        .collect::<Result<Vec<_>, _>>()?;
                    let angle = if is_rotation(&gate_name) {
                        read_angle(&current)
                    } else {
                        0.0
                    };
                    if let Some(g) = self.resolve_gate(&gate_name, angle, &qs)? {
                        stmts.push(Stmt::Gate(g));
                    }
                    // A register index names a fixed physical slot, and no
                    // gate — including SWAP — relabels its own slot; SWAP's
                    // effect is to exchange the *contents* of two fixed slots,
                    // not to rename them. So result[i] always continues
                    // operand[i]'s slot, uniformly across every gate: which
                    // logical qubit's state that slot now holds is a routing
                    // concern (`sabre_routing`'s `wires` map), not this
                    // emitter's — it only threads slot identity.
                    for (i, q) in qs.iter().enumerate() {
                        if let Ok(result) = current.result(i) {
                            self.thread(&result, *q);
                        }
                    }
                }
                other => return Err(EmitError::UnexpectedOp(other.to_string())),
            }
            op = current.next_in_block();
        }
        Ok((stmts, outputs))
    }
}

/// First pass: count qubit allocations and measurements to fix the register
/// widths before any [`QubitId`] / [`BitId`] is constructed.
fn count_registers(top: BlockRef) -> (usize, usize) {
    let mut num_qubits = 0;
    let mut num_bits = 0;
    let mut op = top.first_operation();
    while let Some(current) = op {
        if is_allocation(current) {
            num_qubits += current.result_count();
        } else if op_name(&current) == quantum_dynamic::op::MEASURE {
            num_bits += 1;
        }
        op = current.next_in_block();
    }
    (num_qubits, num_bits)
}

/// Bind an op's qubit results to physical indices. `outputs` gives the physical
/// index each result carries (in result order); falls back to `fallback` when
/// the terminator did not name an output for that position.
fn thread_results(
    reifier: &mut Reifier,
    operation: OperationRef,
    fallback: &[QubitId],
    outputs: &[QubitId],
) {
    for (index, result) in operation.results().enumerate() {
        if !quantum_circ::is_qubit_type(result.r#type()) {
            continue;
        }
        if let Some(id) = outputs.get(index).or_else(|| fallback.get(index)) {
            reifier.thread(&result, *id);
        }
    }
}

/// Reify a monadic-lowered `quantum.dynamic` module into a typed [`Program`].
pub fn reify(module: &Module, target: &BackendTarget) -> Result<Program, EmitError> {
    let Some(top) = module
        .as_operation()
        .region(0)
        .ok()
        .and_then(|r| r.first_block())
    else {
        return Ok(Program::new(0, 0));
    };

    let (num_qubits, num_bits) = count_registers(top);
    let mut program = Program::new(num_qubits, num_bits);
    let mut reifier = Reifier {
        qubits: HashMap::new(),
        bits: HashMap::new(),
        num_qubits,
        num_bits,
        next_qubit: 0,
        next_bit: 0,
        native: target
            .native_gate_names()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        target_id: &target.id,
    };

    let mut op = top.first_operation();
    while let Some(current) = op {
        let name = op_name(&current);
        if is_allocation(current) {
            for result in current.results() {
                reifier.alloc_qubit(&program, &result)?;
            }
        } else {
            match name.as_str() {
                quantum_dynamic::op::MEASURE => {
                    let qubit = current
                        .operand(0)
                        .map_err(|_| EmitError::MissingOperand { op: "measure" })?;
                    let q = reifier.lookup_qubit(&qubit)?;
                    let bit_value = current
                        .result(0)
                        .map_err(|_| EmitError::MissingOperand { op: "measure" })?;
                    let b = reifier.alloc_bit(&program, &bit_value)?;
                    program.push_measure(q, b)?;
                }
                quantum_dynamic::op::RESET => {
                    let qubit = current
                        .operand(0)
                        .map_err(|_| EmitError::MissingOperand { op: "reset" })?;
                    let q = reifier.lookup_qubit(&qubit)?;
                    program.push_reset(q)?;
                    if let Ok(result) = current.result(0) {
                        reifier.thread(&result, q);
                    }
                }
                quantum_dynamic::op::UNITARY_REGION => {
                    let operands = qubit_operands(current);
                    let arg_ids = operands
                        .iter()
                        .map(|v| reifier.lookup_qubit(v))
                        .collect::<Result<Vec<_>, _>>()?;
                    let block = current.region(0).ok().and_then(|r| r.first_block()).ok_or(
                        EmitError::MissingOperand {
                            op: "unitary_region",
                        },
                    )?;
                    let (stmts, outputs) = reifier.reify_circ_block(block, &arg_ids)?;
                    program.extend(stmts)?;
                    thread_results(&mut reifier, current, &arg_ids, &outputs);
                }
                quantum_dynamic::op::IF => {
                    let condition = current
                        .operand(0)
                        .map_err(|_| EmitError::MissingOperand { op: "if" })?;
                    let bit = reifier.lookup_bit(&condition)?;
                    let operands = qubit_operands(current);
                    let arg_ids = operands
                        .iter()
                        .map(|v| reifier.lookup_qubit(v))
                        .collect::<Result<Vec<_>, _>>()?;
                    let then_block = current
                        .region(0)
                        .ok()
                        .and_then(|r| r.first_block())
                        .ok_or(EmitError::MissingOperand { op: "if" })?;
                    let (then_body, _) = reifier.reify_circ_block(then_block, &arg_ids)?;
                    let else_body = match current.region(1).ok().and_then(|r| r.first_block()) {
                        Some(block) => reifier.reify_circ_block(block, &arg_ids)?.0,
                        None => Vec::new(),
                    };
                    program.push_if(qasm::Expr::bit_is_set(bit), then_body, else_body)?;
                    // Feed-forward threads each qubit through unchanged.
                    let ids = arg_ids.clone();
                    thread_results(&mut reifier, current, &ids, &ids);
                }
                quantum_dynamic::op::BARRIER => {
                    let operands = qubit_operands(current);
                    let ids = operands
                        .iter()
                        .map(|v| reifier.lookup_qubit(v))
                        .collect::<Result<Vec<_>, _>>()?;
                    program.push_barrier(ids.clone())?;
                    thread_results(&mut reifier, current, &ids, &ids);
                }
                // Function/circuit definitions and anything else at the top
                // level are not executable statements — skip them.
                _ => {}
            }
        }
        op = current.next_in_block();
    }

    Ok(program)
}

/// Emit OpenQASM 3.0 text for a monadic-lowered module on `target`.
pub fn emit(module: &Module, target: &BackendTarget) -> Result<String, EmitError> {
    Ok(qasm::render(&reify(module, target)?))
}
