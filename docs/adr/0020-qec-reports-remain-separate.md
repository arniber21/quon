# Compiler resource reports and Sinter results stay separate artifacts

QEC evaluation produces distinct **primary** outputs: the compiler `ResourceReport` (schedule metrics, QEC metadata, analytic physical error-budget contributions) and a Python/Sinter sampled CSV (logical failures). They are not merged into one compiler JSON or a required companion summary that collapses evidence kinds.

Combining them in a single undifferentiated claim artifact would either mutate compiler DTOs after the fact or force the driver to know about Sinter. Readers may place the files side by side; documentation must state that analytic estimates and sampled results are different kinds of evidence and that neither is a threshold claim. Issue #246 is satisfied by complete separate emits plus clear labeling/docs, not by a fused report format.

## Amendment (#254 ablation sweeps)

An **optional** harness-level **join CSV** is allowed for workload × ablation sweeps (`python/quon_qec_benchmarks.py`). That join may place labeled analytic columns (`evidence_kind_analytic`) beside labeled sampled columns (`evidence_kind_sampled`) on one comparison row.

Constraints that still hold:

1. **Primary artifacts remain separate.** Each cell must still emit / retain the compiler `ResourceReport` JSON, the dual-emit `*.qec.json` + sibling `.stim`, and a **separate** Sinter CSV (sampled-only columns, same shape as `quon_qec_sinter.py`). The join CSV does not replace those.
2. **Do not delete primaries by default.** Sweep work directories keep report / experiment / sinter files unless the operator explicitly opts into cleanup.
3. **No fused claim summary.** The join is for ablation comparison only — not a threshold claim and not a mutation of the `ResourceReport` DTO.
