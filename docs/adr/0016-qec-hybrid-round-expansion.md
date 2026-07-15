# Hybrid QEC expansion: per-round planners + round barriers

QEC workloads expand into `quantum.na` by a hybrid path: `quon_qec` generates a concrete physical gate/interaction graph for each logical op or memory round; existing `quon_na` place/entangle/move/compact planners run *inside* a round; explicit round barriers and measurement/feedforward dependencies prevent compaction or reordering across rounds.

Surface memory rounds use a **serial Z-then-X** phase split (`z_cnot_count` / mid H / X CXs / after H), with Hadamards as first-class `quantum.na.local_gate` schedule actions. That serial split is for hybrid NA scheduling fidelity — it is **not** Stim's interleaved 4-layer extraction and must not be claimed Stim-equivalent for fault-tolerant distance.

A pure “synthetic graph through the whole program” path would lose QEC round structure needed for verification and Stim artifacts. A fully QEC-specific scheduler would duplicate AOD/Rydberg work already invested in #103–#108. The hybrid keeps literature-faithful NA planning while making syndrome-round boundaries first-class for `--verify-na` and experiment JSON.
