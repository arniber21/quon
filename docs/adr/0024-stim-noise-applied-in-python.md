# Stim noise is applied in the Python harness, not by quonc

`--emit-qec-experiment` writes a structure-level `.stim` circuit (stabilizers, detectors, observables, round structure) without physical noise channels. The Python Stim/Sinter harness reads `error_model` from the sibling `*.qec.json` and annotates noise before sampling.

Putting noise in the compiler emit would freeze rates into the circuit and make sweeps awkward; keeping an ideal/structure `.stim` plus JSON parameters lets #253/#254 vary error rates without re-lowering. The JSON remains the parameter source of truth; the `.stim` file remains the geometry/detector source of truth. Both still come from one `quon_qec` IR pass (ADR-0018).
