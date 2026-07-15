# Logical CX via fixed-layout three-patch lattice surgery

`logical_cx` on compatible `QecBlock<Surface, d>` values lowers to a simplified Horsman-style lattice-surgery gadget: control patch, target patch, and a transitional ancilla patch of the same distance, with a canonical linear layout and a fixed smooth/rough merge–split phase sequence. Byproduct Pauli frame updates are recorded in the QEC workload IR and Stim detectors/observables; there is no online decoder.

We rejected a two-patch-only parity merge (not a real CX gadget) and a Stim-only placeholder CNOT (too weak a neutral-atom claim). Acceptance requires a working `d=3` end-to-end path; the same construction should extend to other odd `d ≥ 3` when straightforward, but a general lattice-surgery router / arbitrary patch placement is out of scope.
