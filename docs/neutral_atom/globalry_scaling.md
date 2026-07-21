# GlobalRy echo-refocus scaling ceiling (issue #322)

This document quantifies the O(N²) scaling cost of the Hahn-echo refocus
sequence (`push_global_ry_with_refocus`, issue #298) that makes every
`GlobalRy` whole-plane raster safe for bystander atoms, and analyzes why
zone isolation cannot remove this cost within the current architecture model
without new IR and hardware-model types.

Companion docs: [architecture_model.md](./architecture_model.md) §6 (Rydberg
range and isolation), §7 (zoned model); [benchmark_suite.md](./benchmark_suite.md).

## 1. Problem statement

Neutral-atom hardware locally addresses only the Z axis (via light shifts);
Y-axis rotations come from a single global microwave/Raman field that
illuminates **every** trapped atom simultaneously. The compiler models this
as [`NeutralAtomAction::GlobalRy`](../../quon_na/src/schedule.rs) — a
whole-plane raster with no atom list.

Because every logical atom is bound into the trap array from schedule start
(`layout.initial_bindings`), a bare `GlobalRy(theta)` intended for one atom
physically hits **all** trapped atoms, silently corrupting every bystander.
Issue #298 fixed this with a Hahn-echo composite pulse
([`push_global_ry_with_refocus`](../../quon_na/src/pipeline.rs)):

1. `GlobalRy(theta/2)` — first half-pulse
2. `LocalGate { Rz(pi) }` for every bystander atom
3. `GlobalRy(theta/2)` — second half-pulse
4. `LocalGate { Rz(-pi) }` for every bystander atom

This is **provably exact** (not an approximation): for a bystander,
`Rz(-pi) · Ry(theta/2) · Rz(pi) · Ry(theta/2) = I` — algebraic identity
(see the function's doc comment for the derivation). The wanted atom's two
untouched half-pulses compose to `Ry(theta)`.

**The cost:** one refocused rotation with N trapped atoms emits
`2 + 2*(N-1)` schedule actions (two `GlobalRy` + `N-1` `Rz(pi)` + `N-1`
`Rz(-pi)`). For N independent single-qubit rotations (one per atom), the
total is:

```
N * (2 + 2*(N-1)) = N * 2N = 2*N²
```

Since `interleave_local_gates` gives each action its own `ScheduleLayer`
(a deliberate correctness-first simplification — see its doc comment), that
is also `2*N²` layers/cycles. This is the **O(N²) scaling ceiling**.

## 2. Zone isolation feasibility analysis

### 2.1 What zones do

The zoned architecture ([architecture_model.md](./architecture_model.md) §7,
[AbstractModel] Sec. III-A) physically separates the chip into:

- **Storage zone** — dense static traps, shielded from the Rydberg beam
- **Entanglement zone** — paired traps under a zone-restricted Rydberg beam
- **Readout zone** (optional) — isolated measurement

The Rydberg beam covers **only the entanglement zone**, so storage-zone atoms
are shielded from entangling gates ([AbstractModel] Sec. III-A; [RAP] Sec. II-A).

### 2.2 Why zones do not isolate the GlobalRy raster

The `GlobalRy` raster is **not** a Rydberg entangling gate. It is a global
microwave/Raman 1Q rotation field. The zone model's beam restriction applies
to the **Rydberg laser** (entangling operations), not to the microwave/Raman
field that drives single-qubit Y rotations. The architecture model
([architecture_model.md](./architecture_model.md) §6) explicitly states:

> In the zoned model, R1–R3 hold *inside the entanglement zone*: ... the
> Rydberg beam covers only that zone, so storage-zone atoms are shielded.

This shielding is for the **Rydberg interaction** (entanglement), not for the
microwave/Raman drive. The [`NeutralAtomAction::GlobalRy`](../../quon_na/src/schedule.rs)
action is structurally "all atoms or none" — it has no atom list and no zone
parameter. The current IR and schedule types do not encode a notion of
"which spatial region a global raster covers."

### 2.3 What would be needed for zone-isolated GlobalRy

True spatial isolation of a `GlobalRy` raster would require:

1. **A new action type** — `ZonedGlobalRy { theta, zone_id }` or similar,
   carrying a zone restriction so the raster illuminates only atoms in a
   specific zone. This is a fundamental change to
   [`NeutralAtomAction`](../../quon_na/src/schedule.rs), which today models
   `GlobalRy` as a parameterless whole-plane broadcast.

2. **A hardware-model change** — the architecture model would need to
   describe zone-restricted microwave/Raman beams, which no current cited
   source ([OLSQ-DPQA], [Enola], [Atomique], [RAP], [AbstractModel], [QMAP-docs])
   models. All describe global 1Q rotation fields; zone-restricted 1Q
   addressing is not in the reproduced literature.

3. **An IR change** — the `quantum.na` dialect's `GlobalRy` op would need a
   zone attribute, and the pipeline would need to track which zone each
   atom is in at each schedule step (currently, atom→zone membership is only
   checked for entangling/measurement actions in `validate_zone_constraints`).

4. **Movement preconditions** — to isolate a rotation to a subset of atoms,
   those atoms would need to be physically moved to a separate zone before
   the raster, adding movement overhead that may exceed the refocus savings
   for small N. The compiler would need a cost model for this tradeoff.

### 2.4 Conclusion

Zone isolation of the `GlobalRy` raster is **not tractable** within the
existing types. The current `ZoneSpec`/`ZonedArchitecture` model zone
membership only for entangling and measurement legality, not for 1Q rotation
addressing. Implementing zone-isolated rasters requires new action types,
hardware-model extensions, and literature support that are beyond the scope
of issue #322. The echo-refocus sequence is the correct correctness-preserving
approach given the current model; its O(N²) cost is the price.

## 3. Benchmark methodology

### 3.1 Harness

The benchmark is a deterministic Rust example binary:
[`quon_na/examples/globalry_scaling.rs`](../../quon_na/examples/globalry_scaling.rs).

It replicates the exact `push_global_ry_with_refocus` logic (the function is
private in the pipeline module, but it is pure and well-documented), builds
schedules for N = 2..16 independent single-qubit `ry(theta)` rotations (one
per atom), and measures:

- **Total actions** — sum of all `NeutralAtomAction`s across all layers
- **Total layers** — `ScheduleLayer` count (one action per layer, matching
  `interleave_local_gates`' policy)
- **GlobalRy count** — number of `GlobalRy` raster actions
- **LocalRz count** — number of `LocalGate { Rz(_) }` echo pulses
- **JSON size** — serialized schedule JSON byte count

No wall-clock timing is used (it would be flaky in CI). All measured
quantities are exact and reproducible.

### 3.2 Property tests

[`quon_na/tests/globalry_scaling_props.rs`](../../quon_na/tests/globalry_scaling_props.rs)
asserts the structural invariants with `proptest` over N = 2..32:

1. `total_actions == 2 * N²` (the O(N²) formula)
2. `total_layers == total_actions` (one action per layer)
3. `global_ry_count == 2 * N` (the O(N) raster component)
4. `local_rz_count == 2 * N * (N-1)` (the O(N²) echo-pulse component)
5. Marginal cost increases: `actions(N+1) - actions(N) > actions(N) - actions(N-1)`
6. Exact marginal cost: `actions(N+1) - actions(N) == 4*N + 2`
7. All `GlobalRy` actions carry half-angles (theta/2)
8. Per-rotation correctness: wanted atom has 0 echo pulses, each bystander has 2

## 4. Measured data (N = 2..16)

Produced by `cargo run -p quon_na --example globalry_scaling --features mlir`:

| N atoms | N rotations | Total actions | Total layers | GlobalRy count | LocalRz count | JSON size (bytes) |
|--------:|------------:|--------------:|-------------:|---------------:|--------------:|------------------:|
| 2 | 2 | 8 | 8 | 4 | 4 | 681 |
| 3 | 3 | 18 | 18 | 6 | 12 | 1,645 |
| 4 | 4 | 32 | 32 | 8 | 24 | 2,975 |
| 5 | 5 | 50 | 50 | 10 | 40 | 4,697 |
| 6 | 6 | 72 | 72 | 12 | 60 | 6,843 |
| 7 | 7 | 98 | 98 | 14 | 84 | 9,353 |
| 8 | 8 | 128 | 128 | 16 | 112 | 12,287 |
| 9 | 9 | 162 | 162 | 18 | 144 | 15,619 |
| 10 | 10 | 200 | 200 | 20 | 180 | 19,351 |
| 11 | 11 | 242 | 242 | 22 | 220 | 23,497 |
| 12 | 12 | 288 | 288 | 24 | 264 | 28,077 |
| 13 | 13 | 338 | 338 | 26 | 312 | 33,029 |
| 14 | 14 | 392 | 392 | 28 | 364 | 38,385 |
| 15 | 15 | 450 | 450 | 30 | 420 | 44,171 |
| 16 | 16 | 512 | 512 | 32 | 480 | 50,331 |

**Verification:** all measured counts match the `2*N²` formula exactly.

### Component breakdown

The `2*N²` total breaks down as:

- **GlobalRy rasters:** `2*N` — the O(N) component (two half-pulses per rotation)
- **LocalRz echo pulses:** `2*N*(N-1)` — the O(N²) component (2 echo pulses
  per bystander, N-1 bystanders, N rotations)

The O(N²) cost is entirely in the echo pulses, not the rasters. The rasters
themselves are only O(N); the bystander protection is the scaling bottleneck.

### JSON size growth

The serialized schedule JSON grows roughly linearly with the action count
(~98 bytes/action at N=16). This is a secondary concern (schedule size
affects serialization/metadata overhead) but confirms the O(N²) pattern
extends to output size.

## 5. Extrapolation

Using the `2*N²` formula (verified exact for N ≤ 16):

| N atoms | Predicted actions (2*N²) | Predicted layers | Approx JSON size |
|--------:|------------------------:|-----------------:|------------------:|
| 20 | 800 | 800 | ~77 KB |
| 32 | 2,048 | 2,048 | ~197 KB |
| 50 | 5,000 | 5,000 | ~480 KB |
| 64 | 8,192 | 8,192 | ~786 KB |
| 100 | 20,000 | 20,000 | ~1.9 MB |
| 128 | 32,768 | 32,768 | ~3.1 MB |
| 256 | 131,072 | 131,072 | ~12.6 MB |

**Practical ceiling:** at N ≈ 50–64 atoms with independent 1Q rotations,
the schedule exceeds 5,000 layers — comparable to deep entangling circuits
on the same qubit count. At N = 256 (a large but realistic neutral-atom
array), the schedule would have 131,072 layers from 1Q rotations alone,
before any entangling gates. This is the practical ceiling of the
echo-refocus approach: the schedule depth from 1Q rotations dominates the
total circuit depth well before entangling-gate depth becomes the bottleneck.

**Fidelity impact:** each action carries `fidelity.single_qubit` per the
Enola Eq. (1) product (see [architecture_model.md](./architecture_model.md)
§11.2). With `fidelity.single_qubit = 0.9997` (the cited value), the gate
fidelity product for N independent rotations is
`0.9997^(2*N²)`. At N=16, this is `0.9997^512 ≈ 0.857`; at N=32, `0.9997^2048
≈ 0.539`; at N=64, `0.9997^8192 ≈ 0.085`. The fidelity degradation from echo
pulses alone becomes severe well before the schedule depth becomes
administratively unmanageable.

## 6. Architectural changes needed to remove the ceiling

### 6.1 Zone-isolated rasters (intractable without new types)

As analyzed in §2, this requires a new `ZonedGlobalRy` action type, a
hardware model for zone-restricted 1Q beams, and zone-tracking through the
pipeline. Not a modification to existing types — a new feature.

### 6.2 Angle batching (packing optimization)

A more tractable optimization: if multiple atoms need the **same** `ry`
angle in the same schedule region, a single refocused `GlobalRy` can serve
all of them simultaneously — the echo pulses are only needed for atoms that
need a *different* angle (or no rotation). This would reduce the cost from
`2*N²` to `2*N² / k` where k is the average batch size, but requires:

1. **Layer packing** — `interleave_local_gates` currently gives each action
   its own layer for correctness (preventing reordering). Packing
   independent-atom local gates into shared layers is explicitly called out
   as a "follow-up optimization, not a correctness requirement" in that
   function's doc comment.
2. **Angle analysis** — a pass that groups atoms by rotation angle and
   emits one raster per group rather than one per gate.
3. **Partial echo** — when a batch covers atoms {a₁, ..., aₖ} with the same
   angle, the echo pulses are only needed for the N−k atoms outside the
   batch, reducing per-batch cost from `2 + 2*(N-1)` to `2 + 2*(N-k)`.

This is a compiler optimization, not a hardware model change, and could be
implemented within the existing types. It is the recommended path forward.

### 6.3 Local Y-addressing (hardware model change)

If the hardware model were extended to support locally-addressed Y rotations
(via e.g. focused AC Stark shifts or individual Raman beams), the `GlobalRy`
action would be replaced by a `LocalGate { Ry(theta) }` action, eliminating
the echo-refocus sequence entirely. This is a hardware capability that no
cited source models; it would be a new architecture variant, not an extension
of the current model.

## 7. Summary

| Aspect | Status |
| --- | --- |
| Echo-refocus correctness | ✓ Exact (issue #298, Hahn-echo identity) |
| Scaling pattern | O(N²) actions/layers for N independent rotations |
| Formula | `2 * N²` actions, verified exact for N ≤ 16, proptest for N ≤ 32 |
| Zone isolation feasibility | Not tractable — zones shield Rydberg, not 1Q raster |
| Practical ceiling | N ≈ 50–64 before fidelity/product degradation dominates |
| Recommended fix path | Angle batching (§6.2) — compiler optimization within existing types |
| Hardware-model fix | Zone-restricted 1Q beams or local Y-addressing — new architecture variant |

## References

- **[OLSQ-DPQA]** Tan, Bluvstein, Lukin, Cong, Quantum 8, 1281 (2024). arXiv:2306.03487.
- **[Enola]** Tan, Lin, Cong, ASPDAC 2025. arXiv:2405.15095.
- **[RAP]** Stade, Lin, Cong, Wille, ICCAD 2025. arXiv:2505.22715.
- **[AbstractModel]** Stade, Schmid, Burgholzer, Wille, IEEE QCE 2024. arXiv:2405.08068.
- **[QMAP-docs]** Munich Quantum Toolkit, QMAP documentation.
