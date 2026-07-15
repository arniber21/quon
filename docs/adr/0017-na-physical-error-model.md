# Neutral-atom targets carry a parallel `error_model` for QEC

`NeutralAtomTarget` keeps existing `fidelity` fields for non-QEC cost hooks and adds an optional sibling `error_model` object used by QEC reporting and `--emit-qec-experiment`. v1 fields are explicit physical error probabilities: `rydberg`, `measurement`, `reset`, `movement`, `transfer`, and `idle_per_us`.

We do not replace fidelities (would break checked-in targets and conflate two uses) and do not silently derive errors as `1 - fidelity` (easy to mis-scale across per-op vs per-time quantities). When QEC error reporting or experiment emit is requested and `error_model` is missing, the compiler fails with a clear diagnostic rather than warning and inventing defaults. Analytic report “error budget” lines are schedule-count × rate contributions only — never logical error rates or threshold claims.
