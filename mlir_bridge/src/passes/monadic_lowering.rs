//! Monadic lowering pass (issue #17, SPEC §6).
//!
//! Converts staging ops inside `quantum.circ.run` regions to `quantum.dynamic` IR.
//! `quantum.circ.func` definitions are preserved; their bodies are inlined into
//! `quantum.dynamic.unitary_region` blocks on `apply` / `cond_apply`.

use std::collections::HashMap;

use melior::ir::attribute::{BoolAttribute, FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::{OperationBuilder, OperationLike};
use melior::ir::r#type::TypeId;
use melior::ir::{
    Block, BlockLike, Location, Module, Operation, OperationRef, Region, RegionLike, Value,
    ValueLike,
};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use thiserror::Error;

use crate::dialect::monadic_staging as staging;
use crate::dialect::qec_dynamic;
use crate::dialect::{quantum_circ, quantum_dynamic};

#[derive(Debug, Error)]
pub enum LowerError {
    #[error("no quantum.circ.run region found in module")]
    MissingRun,
    #[error("apply callee `{0}` not found")]
    UnknownCallee(String),
    #[error("func `{0}` has no body")]
    EmptyFunc(String),
    #[error("unsupported op `{0}` in run region")]
    UnsupportedOp(String),
    #[error("failed to build `{op}`: {message}")]
    Build { op: &'static str, message: String },
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
}

fn map_value<'c, 'a>(map: &HashMap<usize, Value<'c, 'a>>, value: Value<'c, 'a>) -> Value<'c, 'a> {
    map.get(&value_key(&value)).copied().unwrap_or(value)
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
    attr: &str,
) -> Option<String> {
    let value = operation.attribute(attr).ok()?;
    StringAttribute::try_from(value)
        .ok()
        .map(|string| string.value().to_string())
}

fn read_i64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, attr: &str) -> Option<i64> {
    let value = operation.attribute(attr).ok()?;
    IntegerAttribute::try_from(value)
        .ok()
        .map(|integer| integer.value())
}

fn read_bool_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, attr: &str) -> Option<bool> {
    let value = operation.attribute(attr).ok()?;
    BoolAttribute::try_from(value)
        .ok()
        .map(|boolean| boolean.value())
}

fn read_f64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, attr: &str) -> Option<f64> {
    let value = operation.attribute(attr).ok()?;
    FloatAttribute::try_from(value).ok().map(|f| f.value())
}

fn read_depth_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> quon_core::DepthExpr {
    read_string_attr(operation, quantum_circ::attr::DEPTH)
        .and_then(|text| quon_core::DepthExpr::parse(&text).ok())
        .unwrap_or(quon_core::DepthExpr::Nat(0))
}

struct FuncInfo<'c, 'a> {
    operation: OperationRef<'c, 'a>,
    depth: quon_core::DepthExpr,
    clifford: bool,
}

fn module_body<'c, 'a>(module: OperationRef<'c, 'a>) -> Option<BlockRef<'c, 'a>> {
    module.region(0).ok()?.first_block()
}

type BlockRef<'c, 'a> = melior::ir::BlockRef<'c, 'a>;

fn walk_module_ops<'c, 'a, F>(module: OperationRef<'c, 'a>, mut visit: F)
where
    F: FnMut(OperationRef<'c, 'a>),
{
    let Some(block) = module_body(module) else {
        return;
    };
    let mut op = block.first_operation();
    while let Some(current) = op {
        visit(current);
        op = current.next_in_block();
    }
}

fn collect_funcs<'c, 'a>(module: OperationRef<'c, 'a>) -> HashMap<String, FuncInfo<'c, 'a>> {
    let mut funcs = HashMap::new();
    walk_module_ops(module, |current_op| {
        if op_name(&current_op) == quantum_circ::op::FUNC
            && let Some(name) = read_string_attr(&current_op, quantum_circ::attr::SYM_NAME)
        {
            funcs.insert(
                name,
                FuncInfo {
                    depth: read_depth_attr(&current_op),
                    clifford: read_bool_attr(&current_op, quantum_circ::attr::CLIFFORD)
                        .unwrap_or(false),
                    operation: current_op,
                },
            );
        }
    });
    funcs
}

fn find_run_op<'c, 'a>(module: OperationRef<'c, 'a>) -> Option<OperationRef<'c, 'a>> {
    let mut found = None;
    walk_module_ops(module, |current_op| {
        if found.is_none() && op_name(&current_op) == staging::op::RUN {
            found = Some(current_op);
        }
    });
    found
}

fn foreign_qubit<'c>(
    context: &'c Context,
    location: Location<'c>,
) -> Result<Operation<'c>, LowerError> {
    OperationBuilder::new("test.qubit", location)
        .add_results(&[quantum_circ::qubit_type(context)])
        .build()
        .map_err(|error| LowerError::Build {
            op: "test.qubit",
            message: error.to_string(),
        })
}

fn insert_lowered<'c, 'a>(
    body: &impl BlockLike<'c, 'a>,
    run_op: OperationRef<'c, 'a>,
    after: &mut Option<OperationRef<'c, 'a>>,
    operation: Operation<'c>,
) -> OperationRef<'c, 'a> {
    let inserted = if let Some(previous) = *after {
        body.insert_operation_after(previous, operation)
    } else {
        body.insert_operation_before(run_op, operation)
    };
    *after = Some(inserted);
    inserted
}

fn remap_qubit_operands<'c, 'a>(
    operation: OperationRef<'c, 'a>,
    map: &HashMap<usize, Value<'c, 'a>>,
) -> Vec<Value<'c, 'a>> {
    operation
        .operands()
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .map(|operand| map_value(map, operand))
        .collect()
}

fn inline_func_as_unitary_body<'c, 'a>(
    context: &'c Context,
    func: &FuncInfo<'c, 'a>,
    input_qubits: &[Value<'c, 'a>],
    location: Location<'c>,
    yield_terminator: bool,
) -> Result<(Region<'c>, Vec<Value<'c, 'a>>), LowerError> {
    let callee =
        read_string_attr(&func.operation, quantum_circ::attr::SYM_NAME).unwrap_or_default();
    let entry = func
        .operation
        .region(0)
        .map_err(|_| LowerError::EmptyFunc(callee.clone()))?
        .first_block()
        .ok_or_else(|| LowerError::EmptyFunc(callee.clone()))?;

    let qubit = quantum_circ::qubit_type(context);
    let region = Region::new();
    let block = Block::new(
        &input_qubits
            .iter()
            .map(|_| (qubit, location))
            .collect::<Vec<_>>(),
    );

    let mut arg_map: HashMap<usize, Value<'c, 'a>> = HashMap::new();
    for index in 0..block.argument_count() {
        let argument = block.argument(index).map_err(|_| LowerError::Build {
            op: quantum_dynamic::op::UNITARY_REGION,
            message: format!("missing block argument #{index}"),
        })?;
        arg_map.insert(value_key(&argument), Value::from(argument));
    }
    for index in 0..entry.argument_count() {
        if let (Ok(entry_arg), Ok(inner_arg)) = (entry.argument(index), block.argument(index)) {
            arg_map.insert(value_key(&entry_arg), Value::from(inner_arg));
        }
    }

    let mut op = entry.first_operation();
    let mut return_operands = Vec::new();
    while let Some(gate_op) = op {
        let name = op_name(&gate_op);
        if name == quantum_circ::op::RETURN {
            return_operands = gate_op
                .operands()
                .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
                .map(|operand| map_value(&arg_map, operand))
                .collect();
            break;
        }

        if name == quantum_circ::op::GATE {
            let gate_name = read_string_attr(&gate_op, quantum_circ::attr::GATE_NAME)
                .unwrap_or_else(|| "?".to_string());
            let depth_contribution =
                read_i64_attr(&gate_op, quantum_circ::attr::DEPTH_CONTRIBUTION).unwrap_or(1);
            let clifford = read_bool_attr(&gate_op, quantum_circ::attr::CLIFFORD).unwrap_or(true);
            let angle = read_f64_attr(&gate_op, quantum_circ::attr::ANGLE);
            let operands: Vec<Value<'c, 'a>> = gate_op
                .operands()
                .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
                .map(|operand| map_value(&arg_map, operand))
                .collect();
            // A rotation gate carries its angle as a separate `angle` attribute
            // (`quantum_circ::rotation_gate`, not `quantum_circ::gate`) — losing
            // that distinction here would silently rebuild it as an angle-less
            // gate, inlining e.g. `Rz(theta)` as if it took no parameter at all.
            let built = if let Some(theta) = angle {
                let Some(&qubit) = operands.first() else {
                    return Err(LowerError::Build {
                        op: quantum_circ::op::GATE,
                        message: "rotation gate has no qubit operand".to_string(),
                    });
                };
                quantum_circ::rotation_gate(
                    context,
                    &gate_name,
                    theta,
                    depth_contribution,
                    clifford,
                    qubit,
                    location,
                )
            } else {
                quantum_circ::gate(
                    context,
                    &gate_name,
                    depth_contribution,
                    clifford,
                    &operands,
                    location,
                )
            }
            .map_err(|error| LowerError::Build {
                op: quantum_circ::op::GATE,
                message: error.to_string(),
            })?;
            let appended = block.append_operation(built);
            // Thread by the *result* SSA value: a downstream gate consumes the
            // previous gate's result (the live wire), not its operand. Mapping
            // operand→result instead would leave chained gates referencing the
            // original func's dangling SSA values.
            for (result_index, gate_result) in gate_op
                .results()
                .filter(|result| quantum_circ::is_qubit_type(result.r#type()))
                .enumerate()
            {
                if let Ok(result) = appended.result(result_index) {
                    arg_map.insert(value_key(&gate_result), Value::from(result));
                }
            }
        } else {
            return Err(LowerError::UnsupportedOp(name));
        }
        op = gate_op.next_in_block();
    }

    let outputs = if return_operands.is_empty() {
        (0..block.argument_count())
            .map(|index| {
                block
                    .argument(index)
                    .map(Value::from)
                    .map_err(|_| LowerError::Build {
                        op: quantum_dynamic::op::UNITARY_REGION,
                        message: format!("missing block argument #{index}"),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        return_operands
    };

    if yield_terminator {
        block.append_operation(
            quantum_dynamic::r#yield(&outputs, location).map_err(|error| LowerError::Build {
                op: quantum_dynamic::op::YIELD,
                message: error.to_string(),
            })?,
        );
    } else {
        block.append_operation(quantum_circ::r#return(&outputs, location).map_err(|error| {
            LowerError::Build {
                op: quantum_circ::op::RETURN,
                message: error.to_string(),
            }
        })?);
    }

    region.append_block(block);
    Ok((region, outputs))
}

fn lower_run_region<'c, 'a>(
    context: &'c Context,
    module: OperationRef<'c, 'a>,
    funcs: &HashMap<String, FuncInfo<'c, 'a>>,
    run_op: OperationRef<'c, 'a>,
) -> Result<(), LowerError> {
    let location = run_op.location();
    let module_block = module_body(module).ok_or(LowerError::MissingRun)?;
    let mut insert_after: Option<OperationRef<'c, 'a>> = None;
    let region = run_op.region(0).map_err(|_| LowerError::MissingRun)?;
    let entry = region.first_block().ok_or(LowerError::MissingRun)?;

    let mut value_map: HashMap<usize, Value<'c, 'a>> = HashMap::new();
    for index in 0..entry.argument_count() {
        if let Ok(argument) = entry.argument(index) {
            let mapped = run_op
                .operand(index)
                .unwrap_or_else(|_| Value::from(argument));
            value_map.insert(value_key(&argument), mapped);
        }
    }

    let mut op = entry.first_operation();
    while let Some(staging_op) = op {
        let name = op_name(&staging_op);
        let next = staging_op.next_in_block();
        match name.as_str() {
            staging::op::YIELD => break,
            staging::op::QREG => {
                let count = read_i64_attr(&staging_op, staging::attr::COUNT).unwrap_or(0);
                for index in 0..count {
                    let inserted = insert_lowered(
                        &module_block,
                        run_op,
                        &mut insert_after,
                        foreign_qubit(context, location)?,
                    );
                    if let Ok(staging_result) = staging_op.result(index as usize) {
                        let qubit_result = inserted.result(0).map_err(|_| LowerError::Build {
                            op: staging::op::QREG,
                            message: "missing qubit result".to_string(),
                        })?;
                        value_map.insert(value_key(&staging_result), Value::from(qubit_result));
                    }
                }
            }
            staging::op::APPLY => {
                let callee = read_string_attr(&staging_op, staging::attr::CALLEE)
                    .ok_or_else(|| LowerError::UnknownCallee("?".to_string()))?;
                let qubits = remap_qubit_operands(staging_op, &value_map);
                let func = funcs
                    .get(&callee)
                    .ok_or_else(|| LowerError::UnknownCallee(callee.clone()))?;
                let (unitary_body, _) =
                    inline_func_as_unitary_body(context, func, &qubits, location, false)?;
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    quantum_dynamic::unitary_region(
                        context,
                        &qubits,
                        &func.depth,
                        func.clifford,
                        unitary_body,
                        location,
                    )
                    .map_err(|error| LowerError::Build {
                        op: quantum_dynamic::op::UNITARY_REGION,
                        message: error.to_string(),
                    })?,
                );
                for (index, result) in inserted.results().enumerate() {
                    if let Ok(staging_result) = staging_op.result(index) {
                        value_map.insert(value_key(&staging_result), Value::from(result));
                    }
                }
            }
            staging::op::MEASURE | staging::op::DISCARD => {
                let qubit = map_value(
                    &value_map,
                    staging_op.operand(0).map_err(|_| LowerError::Build {
                        op: quantum_dynamic::op::MEASURE,
                        message: "missing qubit operand".to_string(),
                    })?,
                );
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    quantum_dynamic::measure(context, qubit, location).map_err(|error| {
                        LowerError::Build {
                            op: quantum_dynamic::op::MEASURE,
                            message: error.to_string(),
                        }
                    })?,
                );
                if name == staging::op::MEASURE
                    && let Ok(staging_result) = staging_op.result(0)
                {
                    let bit_result = inserted.result(0).map_err(|_| LowerError::Build {
                        op: quantum_dynamic::op::MEASURE,
                        message: "missing measure result".to_string(),
                    })?;
                    value_map.insert(value_key(&staging_result), Value::from(bit_result));
                }
            }
            staging::op::RESET => {
                let qubit = map_value(
                    &value_map,
                    staging_op.operand(0).map_err(|_| LowerError::Build {
                        op: quantum_dynamic::op::RESET,
                        message: "missing qubit operand".to_string(),
                    })?,
                );
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    quantum_dynamic::reset(context, qubit, location).map_err(|error| {
                        LowerError::Build {
                            op: quantum_dynamic::op::RESET,
                            message: error.to_string(),
                        }
                    })?,
                );
                if let Ok(staging_result) = staging_op.result(0) {
                    let qubit_result = inserted.result(0).map_err(|_| LowerError::Build {
                        op: quantum_dynamic::op::RESET,
                        message: "missing reset result".to_string(),
                    })?;
                    value_map.insert(value_key(&staging_result), Value::from(qubit_result));
                }
            }
            staging::op::COND_APPLY => {
                let then_callee =
                    read_string_attr(&staging_op, staging::attr::THEN_CALLEE).unwrap_or_default();
                let else_callee =
                    read_string_attr(&staging_op, staging::attr::ELSE_CALLEE).unwrap_or_default();
                let condition = map_value(
                    &value_map,
                    staging_op.operand(0).map_err(|_| LowerError::Build {
                        op: quantum_dynamic::op::IF,
                        message: "missing condition operand".to_string(),
                    })?,
                );
                let qubits = remap_qubit_operands(staging_op, &value_map);
                let then_region = {
                    let func = funcs
                        .get(&then_callee)
                        .ok_or_else(|| LowerError::UnknownCallee(then_callee.clone()))?;
                    inline_func_as_unitary_body(context, func, &qubits, location, true)?.0
                };
                let else_region = {
                    let func = funcs
                        .get(&else_callee)
                        .ok_or_else(|| LowerError::UnknownCallee(else_callee.clone()))?;
                    inline_func_as_unitary_body(context, func, &qubits, location, true)?.0
                };
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    quantum_dynamic::r#if(
                        context,
                        condition,
                        &qubits,
                        then_region,
                        else_region,
                        location,
                    )
                    .map_err(|error| LowerError::Build {
                        op: quantum_dynamic::op::IF,
                        message: error.to_string(),
                    })?,
                );
                for (index, result) in inserted.results().enumerate() {
                    if let Ok(staging_result) = staging_op.result(index) {
                        value_map.insert(value_key(&staging_result), Value::from(result));
                    }
                }
            }
            staging::op::QEC_CONSTRUCT => {
                let family = read_string_attr(&staging_op, staging::attr::FAMILY).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::CONSTRUCT,
                        message: "missing family".into(),
                    },
                )?;
                let distance = read_i64_attr(&staging_op, staging::attr::DISTANCE).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::CONSTRUCT,
                        message: "missing distance".into(),
                    },
                )?;
                let basis = read_string_attr(&staging_op, staging::attr::BASIS).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::CONSTRUCT,
                        message: "missing basis".into(),
                    },
                )?;
                let logical_id = read_i64_attr(&staging_op, staging::attr::LOGICAL_ID).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::CONSTRUCT,
                        message: "missing logical_id".into(),
                    },
                )?;
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    qec_dynamic::qec_construct(
                        context, &family, distance, &basis, logical_id, location,
                    )
                    .map_err(|error| LowerError::Build {
                        op: qec_dynamic::op::CONSTRUCT,
                        message: error.to_string(),
                    })?,
                );
                if let Ok(staging_result) = staging_op.result(0) {
                    let result = inserted.result(0).map_err(|_| LowerError::Build {
                        op: qec_dynamic::op::CONSTRUCT,
                        message: "missing construct result".into(),
                    })?;
                    value_map.insert(value_key(&staging_result), Value::from(result));
                }
            }
            staging::op::QEC_MEMORY_ROUND => {
                let block_val = map_value(
                    &value_map,
                    staging_op.operand(0).map_err(|_| LowerError::Build {
                        op: qec_dynamic::op::MEMORY_ROUND,
                        message: "missing block operand".into(),
                    })?,
                );
                let logical_id = read_i64_attr(&staging_op, staging::attr::LOGICAL_ID).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::MEMORY_ROUND,
                        message: "missing logical_id".into(),
                    },
                )?;
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    qec_dynamic::qec_memory_round(context, block_val, logical_id, location)
                        .map_err(|error| LowerError::Build {
                            op: qec_dynamic::op::MEMORY_ROUND,
                            message: error.to_string(),
                        })?,
                );
                if let Ok(staging_result) = staging_op.result(0) {
                    let result = inserted.result(0).map_err(|_| LowerError::Build {
                        op: qec_dynamic::op::MEMORY_ROUND,
                        message: "missing memory_round result".into(),
                    })?;
                    value_map.insert(value_key(&staging_result), Value::from(result));
                }
            }
            staging::op::QEC_MEASURE_LOGICAL => {
                let block_val = map_value(
                    &value_map,
                    staging_op.operand(0).map_err(|_| LowerError::Build {
                        op: qec_dynamic::op::MEASURE_LOGICAL,
                        message: "missing block operand".into(),
                    })?,
                );
                let basis = read_string_attr(&staging_op, staging::attr::BASIS).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::MEASURE_LOGICAL,
                        message: "missing basis".into(),
                    },
                )?;
                let logical_id = read_i64_attr(&staging_op, staging::attr::LOGICAL_ID).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::MEASURE_LOGICAL,
                        message: "missing logical_id".into(),
                    },
                )?;
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    qec_dynamic::qec_measure_logical(
                        context, block_val, &basis, logical_id, location,
                    )
                    .map_err(|error| LowerError::Build {
                        op: qec_dynamic::op::MEASURE_LOGICAL,
                        message: error.to_string(),
                    })?,
                );
                if let Ok(staging_result) = staging_op.result(0) {
                    let result = inserted.result(0).map_err(|_| LowerError::Build {
                        op: qec_dynamic::op::MEASURE_LOGICAL,
                        message: "missing measure_logical result".into(),
                    })?;
                    value_map.insert(value_key(&staging_result), Value::from(result));
                }
            }
            staging::op::QEC_LOGICAL_CX => {
                let control = map_value(
                    &value_map,
                    staging_op.operand(0).map_err(|_| LowerError::Build {
                        op: qec_dynamic::op::LOGICAL_CX,
                        message: "missing control operand".into(),
                    })?,
                );
                let target = map_value(
                    &value_map,
                    staging_op.operand(1).map_err(|_| LowerError::Build {
                        op: qec_dynamic::op::LOGICAL_CX,
                        message: "missing target operand".into(),
                    })?,
                );
                let control_id = read_i64_attr(&staging_op, staging::attr::CONTROL_ID).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::LOGICAL_CX,
                        message: "missing control_id".into(),
                    },
                )?;
                let target_id = read_i64_attr(&staging_op, staging::attr::TARGET_ID).ok_or(
                    LowerError::Build {
                        op: qec_dynamic::op::LOGICAL_CX,
                        message: "missing target_id".into(),
                    },
                )?;
                let inserted = insert_lowered(
                    &module_block,
                    run_op,
                    &mut insert_after,
                    qec_dynamic::qec_logical_cx(
                        context, control, target, control_id, target_id, location,
                    )
                    .map_err(|error| LowerError::Build {
                        op: qec_dynamic::op::LOGICAL_CX,
                        message: error.to_string(),
                    })?,
                );
                for (index, result) in inserted.results().enumerate() {
                    if let Ok(staging_result) = staging_op.result(index) {
                        value_map.insert(value_key(&staging_result), Value::from(result));
                    }
                }
            }
            other => return Err(LowerError::UnsupportedOp(other.to_string())),
        }
        op = next;
    }

    Ok(())
}

fn lower_module<'c, 'a>(
    context: &'c Context,
    module: OperationRef<'c, 'a>,
) -> Result<(), LowerError> {
    let funcs = collect_funcs(module);
    let run_op = find_run_op(module).ok_or(LowerError::MissingRun)?;
    lower_run_region(context, module, &funcs, run_op)?;
    IrRewriter::new(context).as_rewriter_base().erase_op(run_op);
    Ok(())
}

/// Runs monadic lowering on `module` in place (issue #17).
pub fn run_on_module<'c>(context: &'c Context, module: &Module<'c>) -> Result<(), LowerError> {
    lower_module(context, module.as_operation())
}

#[repr(align(8))]
struct PassId;

static MONADIC_LOWERING_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct MonadicLowering {
    context: usize,
}

impl MonadicLowering {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for MonadicLowering {
    fn initialize(&mut self, context: ContextRef<'c>) {
        // SAFETY: the pass manager keeps the context alive for the pass lifetime.
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        if let Err(error) = lower_module(context, operation) {
            eprintln!("monadic-lowering: {error}");
            pass.signal_failure();
        }
    }
}

/// Creates the monadic lowering pass (`quantum.circ.run` → `quantum.dynamic`).
pub fn create_pass() -> Pass {
    create_external(
        MonadicLowering::new(),
        TypeId::create(&MONADIC_LOWERING_PASS_ID),
        "monadic-lowering",
        "monadic-lowering",
        "Lower quantum.circ.run staging ops to quantum.dynamic IR",
        "",
        &[],
    )
}
