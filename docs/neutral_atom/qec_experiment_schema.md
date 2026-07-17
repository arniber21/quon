# QEC experiment artifact schema (`*.qec.json` + sibling `.stim`)

`--emit-qec-experiment <PATH>` dual-emits evaluation artifacts from one
`quon_qec` expanded-workload pass (ADR-0018). `<PATH>` is the JSON file; a
sibling `<stem>.stim` is written beside it (`foo.qec.json` → `foo.stim`).

Physical noise is **not** embedded in the Stim circuit (ADR-0024). Python
(`#253`) loads both files and annotates noise from the JSON `error_model`
before Sinter sampling. The resulting Sinter CSV is a **sampled** artifact —
keep it separate from the compiler analytic `ResourceReport`
(`--emit-resource-report`); do not fuse them or treat either as a threshold
claim (ADR-0020 / #246).

For a compiler-driven path that runs compilation, dual-emit, Sinter sampling,
and provenance-checked fusion from one entry point, see
`quonc --emit-qec-validation` and `docs/neutral_atom/qec_validation_report.md`
(#280). That fused report keeps the analytic and sampled evidence in separate
labeled sections (ADR-0020 amendment); it does not mutate this schema.

## `*.qec.json` fields (`schema_version: 1`, `kind: "qec_experiment"`)

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | u32 | Wire version (`1`) |
| `kind` | string | Always `"qec_experiment"` |
| `family` | string | Source family (`"repetition"` / `"surface"`) |
| `code_family` | string | Report label (`"repetition_code_toy"`, …) |
| `distance` | u32 | Code distance |
| `rounds` | u32 | Number of `memory_round` / syndrome cycles |
| `logical_ids` | u32[] | Logical qubit ids in the workload |
| `check_graph` | object | Atoms + stabilizer checks (see below) |
| `measurement_schedule` | array | Per expanded-round measure terminals |
| `logical_observables` | array | Logical Paulis as products of atom ids |
| `atom_site_map` | array | Atom → `{role, logical_id, index_in_block}` |
| `error_model` | object | Snapshot of target rates (ADR-0017); **required** |
| `na_refs` | array | Round refs into the same compile’s `quantum.na` schedule |
| `stim_file` | string | Basename of the sibling structure-level Stim file |

Unknown JSON fields are rejected on load (`deny_unknown_fields`). Emitting
without a target `error_model` is a hard compiler failure (never `1 − fidelity`).

### `check_graph`

- `atoms`, `data_atoms`, `check_atoms` — physical atom ids in layout order
- `stabilizers[]` — `{ logical_id, check_atom, basis?, data_atoms }`
  - `basis` is `"x"` / `"z"` (serde enum). **Defaults to `z`** when absent so
    schema_version 1 #255 repetition consumers keep deserializing; surface
    always emits an explicit basis.

### `measurement_schedule[]`

- `round_index`, `kind` (`construct` / `memory_round` / `measure_logical`)
- `logical_id`, `measured_atoms`, optional `basis` (`"x"` / `"z"` — serde enum)

### `atom_site_map[]`

- `atom`, `role` (`"data"` / `"check"` — serde enum), `logical_id`, `index_in_block`

### `logical_observables[]`

- `id`, `logical_id`, `basis` (`"x"` / `"z"` from **measure-logical**, not init), `atoms`

### `na_refs[]`

Always includes round structure from the expanded IR. When the NA schedule from
the same compile is available, memory-round entries set `barrier_cycle` to the
durable Wait cycle identified via `round_barrier_cuts` — fail-closed unless the
Wait count equals `#memory_rounds`. Optional `cycle_start` / `cycle_end` are
reserved for richer layer ranges.

### `error_model`

Same type as the neutral-atom target snapshot (`quon_qec::ErrorModelSnapshot` /
`backend::NeutralAtomErrorModelSnapshot` alias): `rydberg`, `measurement`,
`reset`, `movement`, `transfer`, `idle_per_us` (probabilities in `[0, 1]`).

## Sibling `.stim` (structure only)

Generated from the same `ExpandedWorkload` — never by re-parsing `quantum.na`.
Contains:

- `QUBIT_COORDS`, `R`, optional data `H` for X-init, layered non-overlapping
  `CX`+`TICK` (expand order with mid/after X-check `H`), `MR` / `MZ` (or `MX`),
  `TICK`
- `DETECTOR` (consecutive syndromes + final data closure)
  - Z-memory: first-round Z-checks only (`rotated_memory_z`-style)
  - X-memory (surface): first-round X-checks only (`rotated_memory_x`-style)
- `OBSERVABLE_INCLUDE` (product of data measurements in the measure basis)

**Surface schedule vs Stim FT:** expand uses a **serial Z-then-X** phase split
for hybrid NA scheduling. That is not Stim's interleaved 4-layer extraction —
do **not** claim Stim-equivalent fault-tolerant distance from this structure
alone.

**No** Stim noise channels (`DEPOLARIZE*`, `X_ERROR`, …). The compiler emits
Stim text without linking Stim C++. Python validates parse / detectors /
noiseless sampling via `python/test_qec_stim_smoke.py` (ADR-0022).

## Example

```bash
quonc examples/na_qec/repetition_d3_memory.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-qec-experiment /tmp/rep_d3.qec.json
# writes /tmp/rep_d3.qec.json and /tmp/rep_d3.stim
```
