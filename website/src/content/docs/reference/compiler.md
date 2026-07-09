---
title: Compiler pipeline
description: High-level reference for the pipeline run by quonc.
---

`quonc` runs a front-to-back pipeline from Quon source to OpenQASM 3. The
pipeline is currently defined for the fixed-connectivity gate-model
architecture.

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
6. **Adapt to the fixed target.** The compiler decomposes into native gates,
   routes against the target connectivity, decomposes again after routing, and
   schedules circuit depth.
7. **Collect metrics and emit.** Metrics are collected from the physical IR,
   and the compiler reifies and emits an OpenQASM 3 program.

This is the stable high-level shape, not a promise that every internal pass or
its order is part of the public interface.

## Target-dependent stages

The selected fixed target supplies:

- the physical qubit count and connectivity used by routing;
- the native gate set used by decomposition and emission;
- coherence data used to choose the scheduling mode.

With no `--target`, the compiler uses `generic_openqasm`: a built-in 64-qubit,
all-to-all fixed target with the standard gate set and no device noise data.

Although the target loader also recognizes
`neutral_atom_reconfigurable`, that kind belongs to a separate architecture
family and is not accepted by this OpenQASM pipeline today. It can be inspected
with `quonc --print-target`. See
[Target selection in the CLI reference](../quonc/#target-selection) and the
[neutral-atom architecture model](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md).

## Diagnostics and debug checks

Parse, type, elaboration, lowering, and emission failures stop the compile and
produce diagnostics. `--dump-ir` exposes selected pipeline checkpoints on
standard error. `--verify-linear` adds debug linearity checks before and after
dynamic lowering; these checks are not enabled by default.

The compiler still builds the OpenQASM result when no output flag is supplied.
`--emit-qasm` controls whether `quonc` writes it to standard output. Metrics
flags report data collected from the same compile; they do not select a
different pipeline.

See the [quonc CLI reference](../quonc/) for command examples and the
[experiment-loop guide](https://github.com/arniber21/quon/blob/main/docs/agents/experiment-loop.md)
for watch mode, snapshots, and regression checks.
