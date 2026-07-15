# Logical CX via fixed-layout three-patch lattice surgery

`logical_cx` on compatible `QecBlock<Surface, d>` values lowers to a simplified Horsman-style lattice-surgery gadget: control patch, target patch, and a transitional ancilla patch of the same distance.

Canonical layout is **L-shaped** so merge boundaries match surface-code geometry: rough (ZZ) merge on the shared left/right edge (control↔ancilla) and smooth (XX) merge on the shared top/bottom edge (ancilla↔target). The phase sequence is fixed merge → seam-split → merge → seam-split → ancilla logical-Z measure; byproduct Pauli frame updates are **outcome-conditioned** (apply when the named measurement parity is −1) and recorded in the QEC workload IR and Stim observables. Split rounds re-measure the seam (not Wait-only placeholders); surrounding `memory_round` ops restore full patch error correction. There is no online decoder.

Hybrid dual-emit: the NA schedule uses geometric seam CX checks (L/R and top/bottom); structure Stim evaluates Horsman merges as logical-operator `MPP` measurements so frame-corrected Z observables are sinter-evaluable under noiseless codespace prep. This is intentionally simplified — not a claim of Stim-equivalent FT distance for the geometric seam schedule alone.

We rejected a two-patch-only parity merge (not a real CX gadget) and a Stim-only placeholder CNOT (too weak a neutral-atom claim). Acceptance requires a working `d=3` end-to-end path; the same construction should extend to other odd `d ≥ 3` when straightforward, but a general lattice-surgery router / arbitrary patch placement is out of scope.
