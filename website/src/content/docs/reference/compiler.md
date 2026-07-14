---
title: Compiler pipeline
description: High-level reference for the pipeline run by quonc.
---

`quonc` runs a shared frontend and MLIR-backed lowering path before selecting a
target-specific artifact: OpenQASM 3 for fixed gate-model targets, or
schedule/resource outputs for reconfigurable neutral-atom targets.

## Pipeline overview

1. **Parse and desugar.** The frontend parses `.qn` source and desugars surface
   syntax into the core AST.
2. **Typecheck and elaborate.** The linear typechecker validates declarations,
   quantum ownership, depth bounds, register widths, and Clifford
   classifications. Parametric circuit calls are specialized at concrete call
   sites.
3. **Lower to MLIR generic-form IR.** Circuit functions lower through explicit
   Rust builders around unregistered `quantum.circ` operations. Run blocks
   first use monadic staging operations.
4. **Optimize circuit IR.** Circuit-level passes simplify gates and rotations,
   run cancellation/merging, perform compiler uncomputation, and apply bounded
   ZX simplification where the representation supports it.
5. **Lower dynamic behavior.** Monadic staging lowers to dynamic quantum IR;
   measurement deferral and classical-region fusion normalize dynamic behavior.
6. **Select the backend path.** Fixed targets run native-gate decomposition,
   SABRE-style routing, post-routing decomposition, depth scheduling, metrics,
   and OpenQASM 3 emission. Neutral-atom targets extract an interaction graph,
   schedule entangling layers, plan movement, compact the schedule, and build a
   resource report.

This is the stable high-level shape. Individual internal pass ordering remains
an implementation detail.

## Target-dependent stages

Fixed targets supply:

- physical qubit count and connectivity for routing;
- native gate set for decomposition and OpenQASM emission;
- coherence and timing metadata for scheduling and metrics;
- dynamic-circuit capability flags for target reporting.

With no `--target`, the compiler uses `generic_openqasm`: a built-in 64-qubit,
all-to-all fixed target with the standard gate set.

Neutral-atom targets supply:

- zone and array geometry;
- AOD movement and Rydberg interaction parameters;
- operation timing and fidelity data;
- resource and cost-model parameters.

Those targets use `--emit-na-schedule` and `--emit-resource-report` rather than
`--emit-qasm`.

## Diagnostics and debug checks

Parse, type, elaboration, lowering, and emission failures stop the compile and
produce diagnostics. `--dump-ir` exposes selected pipeline checkpoints on
standard error. `--verify-linear` adds debug linearity checks before and after
dynamic lowering.

The compiler still builds the target artifact when no output flag is supplied.
Output flags control what is written to standard output or files; metrics flags
report data collected from the same compile.

Useful inspection commands:

```bash
cargo run -p quonc -- --list-passes
cargo run -p quonc -- test/verify/bell.qn --dump-ir --emit-qasm
cargo run -p quonc -- \
  --target targets/neutral_atom/generic_rna_v0.json \
  --print-target
```

See the [quonc CLI reference](../quonc/) for command examples and the
[maturation path](/guides/roadmap/) for the production-hardening direction.
