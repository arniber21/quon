# QEC validation report (`*.validation.json` / `*.validation.md`)

`quonc --emit-qec-validation <PATH>` is a **compiler-driven** validation /
report-fusion path (#280). From one user-facing entry point it:

1. compiles the QEC program for a neutral-atom target,
2. dual-emits the QEC experiment (`*.qec.json` + sibling structure-level
   `.stim`, ADR-0018),
3. builds the analytic `ResourceReport` (schedule metrics + physical error
   budget = rate × counts, ADR-0017),
4. shells out to the Python Stim/Sinter harness
   (`python/quon_qec_sinter.py --json`) to sample logical failures
   (noise annotated from the JSON `error_model`, ADR-0024), and
5. fuses the analytic and sampled evidence — after a provenance check — into a
   **new, separate** report artifact with clearly labeled sections.

> **Sampled results are validation evidence, not a threshold claim.** The
> analytic and sampled numbers are *different kinds of evidence*; they sit side
> by side with provenance and are never collapsed into an undifferentiated
> below-threshold claim (ADR-0020).

## Why a third artifact (ADR-0020)

ADR-0020 keeps the compiler `ResourceReport` and the Sinter CSV as separate
primaries. The fused validation report is an **optional third artifact** (the
same pattern as the #254 ablation join CSV): it *embeds* an unmodified
`ResourceReport` in its `analytic` section rather than mutating that DTO, and it
keeps the `analytic` and `sampled` evidence in separate `evidence_kind`-labeled
sections. The `*.qec.json` / `.stim`, the analytic `ResourceReport` JSON, and
the sampled-evidence JSON are all written as separate sibling files beside the
report.

## Usage

```bash
quonc examples/na_qec/repetition_d3_memory.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-qec-validation /tmp/rep_d3.validation.json \
  --validation-shots 1024 --validation-seed 7 --validation-decoder pymatching
```

Writes, for a base of `rep_d3`:

| File | Contents |
| --- | --- |
| `rep_d3.validation.json` | Fused report (analytic + sampled sections) |
| `rep_d3.validation.md` | Human-readable rendering of the same report |
| `rep_d3.qec.json` + `rep_d3.stim` | QEC experiment dual-emit (ADR-0018) |
| `rep_d3.resource_report.json` | Analytic `ResourceReport` primary (ADR-0017) |
| `rep_d3.sampled.json` | Sampled-evidence JSON from the harness |

Flags (`quonc --help`, heading **QEC validation**):

- `--validation-shots N` — Sinter shots (default `64`).
- `--validation-seed SEED` — deterministic Stim sampler seed (default `7`).
- `--validation-decoder NAME` — Sinter decoder (default `pymatching`).
- `--attach-sampled PATH` — fuse a pre-sampled evidence JSON instead of shelling
  out to Python (offline / CI without the Stim stack).
- `--allow-sampled-mismatch` — downgrade a provenance mismatch from a refusal to
  a recorded warning.
- `--python PATH` / `--sinter-harness PATH` — override the interpreter / harness
  script (defaults search the repo `.venv` then `python3`, and up from the CWD
  for `python/quon_qec_sinter.py`).

The Stim/Sinter stack must be installed for the default path
(`pip install -r python/requirements.txt`, or `just setup-python`). Without it,
use `--attach-sampled` with a harness-produced JSON.

## Report schema (`schema_version: 1`, `kind: "qec_validation_report"`)

| Field | Meaning |
| --- | --- |
| `disclaimer` | Top-level "validation evidence, not a threshold claim" note |
| `provenance` | Fingerprint tying sampled data to the compiled artifact |
| `analytic` | `evidence_kind: "analytic"` section embedding the `ResourceReport` |
| `sampled` | `evidence_kind: "sampled"` section from Stim/Sinter |
| `mismatch_warnings` | Present only with `--allow-sampled-mismatch` (else omitted) |

### `provenance`

`source`, `target_id`, `family`, `code_family`, `distance`, `rounds`,
`logical_ids`, `experiment_sha256` (SHA-256 of the emitted `*.qec.json` bytes),
`stim_file`.

### `analytic`

`evidence_kind` (`"analytic"`), `disclaimer`, and `resource_report` — the
unmodified `ResourceReport` (schedule metrics + `error_budget` +
`gate_fidelity_product` / `estimated_fidelity`, the Enola Eq. (1) fidelity
estimate, issue #305 — a distinct analytic estimate from `error_budget`, see
architecture_model.md §11.2).

### `sampled`

`evidence_kind` (`"sampled"`), `disclaimer`, `decoder`, `seed`, `tick_us`,
`confidence_level`, and `experiments[]`. Each experiment records `experiment`,
`experiment_sha256`, `family`, `code_family`, `distance`, `rounds`,
`logical_observables` (names), and `results[]`. Each result records `shots`,
`error_scale`, `noise_model` (the sampled `error_model`), `logical_failures`,
`logical_failure_rate`, and a Wilson `confidence_interval`
(`{low, high, level, method}`).

## Provenance / mismatch handling

Before fusing, the compiler compares the sampled evidence fingerprint against
the artifact it just emitted (`experiment_sha256`, plus `family`,
`code_family`, `distance`, `rounds`). If they disagree — for example, sampled
data attached for a different distance — fusion **refuses** with a field-level
diff:

```
error: sampled data does not match the compiled QEC artifact — refusing to fuse
incompatible evidence (pass --allow-sampled-mismatch to downgrade to a warning):
  - distance: artifact 3 != sampled 5
```

Pass `--allow-sampled-mismatch` to attach anyway; the discrepancies are recorded
in `mismatch_warnings` (and flagged in the Markdown) instead of aborting.

## Relationship to other artifacts

- `--emit-resource-report` — analytic primary only (ADR-0017 / ADR-0020).
- `--emit-qec-experiment` — QEC dual-emit primary only (ADR-0018).
- `python/quon_qec_sinter.py` — sampled Sinter CSV / `--json` primary (ADR-0024).
- `python/quon_qec_benchmarks.py` — ablation **join CSV** (ADR-0020 amendment,
  #254).
- `--emit-qec-validation` — the fused validation report (this doc, #280). It
  reuses the above building blocks and does not replace them.
