# Put QEC workload IR in a shared `quon_qec` crate

The MLIR-free QEC workload IR (blocks, rounds, logical ops, stabilizer/check graph, experiment-facing types) lives in a new workspace crate `quon_qec`, not inside `quon_na` or `mlir_bridge`.

`quon_na` remains the neutral-atom scheduler and `quantum.na` owner (ADR-0008). QEC semantics and Stim/Sinter-facing experiment types are shared across the NA backend and external Python tooling, so they should not be buried in the architecture-specific crate. Frontend/`mlir_bridge` lower QEC builtins into `quantum.dynamic`; `quon_na` (or the driver) collects them into `quon_qec` structures and expands schedules from there.

Existing sizing helpers in `quon_na::qec` (`CodeFamily`, `expand_code_block`, …) should migrate or re-export through `quon_qec` so family formulas and workload IR share one source of truth.
