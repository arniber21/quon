# Dual-emit QEC experiments: semantic JSON + Stim circuit

`--emit-qec-experiment` produces two artifacts from one `quon_qec` workload IR pass: (1) versioned semantic `*.qec.json` (family, distance, rounds, logical ids, check graph, observables, atom/site map, error_model snapshot, references into `quantum.na`) and (2) a generated `*.stim` circuit for Sinter.

The JSON is the Quon metadata source of truth; the `.stim` file is the evaluation artifact (structure/detectors/observables only — physical noise is applied in Python per ADR-0024). To avoid drift, both are generated from the same in-memory workload IR in a single emit step — never by re-parsing `quantum.na` or by a separate Python reconstruction of the circuit. The compiler emits Stim text without linking the Stim C++ library; Python loads the `.stim`, annotates noise from the JSON `error_model`, and runs Sinter.

CLI: `--emit-qec-experiment <PATH>` writes `<PATH>` as the JSON and a sibling `<stem>.stim` (same stem, `.stim` extension) unless a future flag overrides the Stim path.
