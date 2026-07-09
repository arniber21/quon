---
title: Compiler pipeline
description: High-level reference for the pipeline run by quonc across hardware targets.
---

`quonc` runs a front-to-back pipeline from Quon source onto a selected
**hardware target** (`--target`). Fixed gate-model targets may emit OpenQASM 3
as an intermediary for Aer and tooling; neutral-atom targets emit schedules and
resource reports (and, tracked in [#167](https://github.com/arniber21/quon/issues/167),
`quantum.na` MLIR). See
[ADR-0010](https://github.com/arniber21/quon/blob/main/docs/adr/0010-hardware-targets-primary-openqasm-intermediary.md).

## Pipeline overview

1. **Parse and desugar.** The frontend parses the `.qn` source and desugars
   surface syntax into the core AST.
2. **Typecheck and elaborate.** The linear typechecker validates declarations.
   Parametric circuit calls are evaluated at concrete call sites and
   specialized into monomorphic circuit functions.
3. **Lower to MLIR.** Circuit functions lower to the `quantum.circ` dialect.
   Run blocks first use monadic staging operations.
4. **Optimize circuit IR.** Circuit-level rewrites simplify gates and
   rotations, perform compiler uncomputation, and apply ZX and Clifford+T
   optimizations until the IR reaches a fixpoint or the pipeline's round limit.
5. **Lower dynamic behavior.** Monadic staging lowers to dynamic quantum IR;
   measurement deferral and classical-region fusion then normalize dynamic
   behavior.
6. **Architecture-specific backend.**
   - **Fixed:** native-gate decomposition, SABRE routing, depth scheduling,
     then optional OpenQASM 3 emission (`--emit-qasm`).
   - **Neutral-atom:** interaction-graph extraction, entangling-layer
     scheduling, zoned RAP or flat AOD movement, optional compaction, then
     schedule / resource-report emit (`--emit-na-schedule`,
     `--emit-resource-report`).
7. **Collect metrics.** Metrics are collected from the resulting IR or schedule.

This is the stable high-level shape, not a promise that every internal pass or
its order is part of the public interface. `quonc --list-passes` prints the
same outline from the binary.

## Target-dependent stages

`--target` loads a `BackendTarget` JSON descriptor. The `kind` field selects
the architecture family (`fixed` or `neutral_atom_reconfigurable`).

For **fixed** targets the descriptor supplies:

- the physical qubit count and connectivity used by routing;
- the native gate set used by decomposition and OpenQASM emission;
- coherence data used to choose the scheduling mode.

With no `--target`, the compiler uses `generic_openqasm`: a built-in 64-qubit,
all-to-all **fixed** target with the standard gate set and no device noise data
— a convenience default, still a hardware-shaped `TargetKind::Fixed`.

For **neutral-atom** targets the descriptor supplies zones, AOD movement,
Rydberg parameters, timing, and cost-model fields used by `quon_na`. See the
[neutral-atom architecture model](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md)
and [backends guide](../backends/).

## Diagnostics and debug checks

Parse, type, elaboration, lowering, and emission failures stop the compile and
produce diagnostics. `--dump-ir` exposes selected pipeline checkpoints on
standard error. `--verify-linear` adds debug linearity checks before and after
dynamic lowering; these checks are not enabled by default.

Emit flags select which artifacts are written (`--emit-qasm`,
`--emit-na-schedule`, `--emit-resource-report`). Metrics flags report data
collected from the same compile; they do not select a different pipeline.

See the [quonc CLI reference](../quonc/) for command examples and the
[experiment-loop guide](https://github.com/arniber21/quon/blob/main/docs/agents/experiment-loop.md)
for watch mode, snapshots, and regression checks.
