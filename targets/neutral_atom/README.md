# Neutral-Atom Targets

`generic_rna_v0.json` is the checked-in v0 descriptor for
`TargetKind::NeutralAtomReconfigurable`.

Every field and assumption is documented in
[`docs/neutral_atom/architecture_model.md`](../../docs/neutral_atom/architecture_model.md),
especially Section 8 for the JSON schema and Section 8.6 for constant
provenance. Values marked placeholder there are tuning or illustrative values,
not literature measurements.

`error_model` (ADR-0017) is an optional sibling of `fidelity` used by QEC
error-budget reporting and `--emit-qec-experiment`. The checked-in example is
illustrative; rates are never derived as `1 - fidelity`.
