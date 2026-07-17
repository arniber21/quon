# Compiler resource reports and Sinter results stay separate artifacts

QEC evaluation produces distinct **primary** outputs: the compiler `ResourceReport` (schedule metrics, QEC metadata, analytic physical error-budget contributions) and a Python/Sinter sampled CSV (logical failures). They are not merged into one compiler JSON or a required companion summary that collapses evidence kinds.

Combining them in a single undifferentiated claim artifact would either mutate compiler DTOs after the fact or force the driver to know about Sinter. Readers may place the files side by side; documentation must state that analytic estimates and sampled results are different kinds of evidence and that neither is a threshold claim. Issue #246 is satisfied by complete separate emits plus clear labeling/docs, not by a fused report format.

## Amendment (#254 ablation sweeps)

An **optional** harness-level **join CSV** is allowed for workload × ablation sweeps (`python/quon_qec_benchmarks.py`). That join may place labeled analytic columns (`evidence_kind_analytic`) beside labeled sampled columns (`evidence_kind_sampled`) on one comparison row.

Constraints that still hold:

1. **Primary artifacts remain separate.** Each cell must still emit / retain the compiler `ResourceReport` JSON, the dual-emit `*.qec.json` + sibling `.stim`, and a **separate** Sinter CSV (sampled-only columns, same shape as `quon_qec_sinter.py`). The join CSV does not replace those.
2. **Do not delete primaries by default.** Sweep work directories keep report / experiment / sinter files unless the operator explicitly opts into cleanup.
3. **No fused claim summary.** The join is for ablation comparison only — not a threshold claim and not a mutation of the `ResourceReport` DTO.

## Amendment (#280 fused validation report)

An **optional** compiler-driven **fused validation report** is allowed as a *third* artifact: `quonc --emit-qec-validation <PATH>` writes `*.validation.json` (and a sibling `*.validation.md`) that places the analytic `ResourceReport` beside sampled Stim/Sinter evidence in **clearly labeled, separate sections** (`analytic` with `evidence_kind: "analytic"`, `sampled` with `evidence_kind: "sampled"`), plus a `provenance` fingerprint (experiment SHA-256, family, distance, rounds). The command runs one user-facing pipeline: compile → QEC dual-emit → analytic resource report → Python `quon_qec_sinter.py --json` sampling → provenance-checked fusion. See `docs/neutral_atom/qec_validation_report.md`.

Constraints that still hold:

1. **The primary `ResourceReport` DTO is not mutated.** The fused report *embeds* an unmodified `ResourceReport` inside its `analytic` section; it never adds sampled fields to `ResourceReport` (whose `deny_unknown_fields` still rejects them).
2. **Evidence kinds stay separate.** Analytic estimates and sampled logical failures live in different labeled sections and are never collapsed into one undifferentiated number.
3. **Primaries remain separate on disk.** The validation run keeps the `*.qec.json` + `.stim`, the analytic `ResourceReport` JSON, and the sampled-evidence JSON as separate sibling files beside the fused report.
4. **Provenance is enforced.** Fusion **refuses** (or, with `--allow-sampled-mismatch`, **warns** and records the discrepancy) when sampled data was produced against an incompatible compiler artifact.
5. **Not a threshold claim.** The fused report is validation evidence — analytic and sampled numbers side by side with provenance — not a below-threshold claim.
