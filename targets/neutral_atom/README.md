# Neutral-Atom Targets

`generic_rna_v0.json` is the checked-in v0 descriptor for
`TargetKind::NeutralAtomReconfigurable`.

Every field and assumption is documented in
[`docs/neutral_atom/architecture_model.md`](../../docs/neutral_atom/architecture_model.md),
especially Section 8 for the JSON schema and Section 8.6 for constant
provenance. Values marked placeholder there are tuning or illustrative values,
not literature measurements.

`error_model` (ADR-0017) is an optional sibling of `fidelity` used by QEC
error-budget reporting (`--emit-resource-report`) and `--emit-qec-experiment`.
The checked-in values are **placeholders** (see architecture_model.md §8.6) and
are deliberately not `1 - fidelity`. Missing `error_model` is a hard failure
when emitting a resource report.
