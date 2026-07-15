# Compiler resource reports and Sinter results stay separate artifacts

QEC evaluation produces distinct outputs: the compiler `ResourceReport` (schedule metrics, QEC metadata, analytic physical error-budget contributions) and Python/Sinter CSV (sampled logical failures). They are not merged into one JSON or a required companion summary file.

Combining them in a single artifact would either mutate compiler DTOs after the fact or force the driver to know about Sinter. Readers may place the files side by side; documentation must state that analytic estimates and sampled results are different kinds of evidence and that neither is a threshold claim. Issue #246 is satisfied by complete separate emits plus clear labeling/docs, not by a fused report format.
