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

## `rap_table_i.json` (issue #111)

A **pinned freeze** of `generic_rna_v0.json`, byte-for-byte identical except
for `id`. It exists so the [RAP] Table I regression anchor (issue #111) does
not silently drift when `generic_rna_v0.json` is tuned for other purposes —
CI for that regression loads `rap_table_i.json`, never `generic_rna_v0.json`
directly. See
[`docs/neutral_atom/rap_table_i_methodology.md`](../../docs/neutral_atom/rap_table_i_methodology.md)
for the full methodology (metric mapping, timing model, tolerances, Phase
1/2 split) and
[`docs/neutral_atom/literature_notes.md`](../../docs/neutral_atom/literature_notes.md)
for the [RAP] citation. Changing this file's numeric fields requires
re-deriving the Phase 1 dump numbers and, if Phase 2 hard asserts are live,
updating their tolerance windows — do not "helpfully" resync it to
`generic_rna_v0.json` after that file changes.
