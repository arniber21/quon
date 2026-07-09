# IBM snapshot provenance

## `fake_manila_v2.json`

| Field | Value |
|---|---|
| Target id | `fake_manila_v2` |
| Device family | IBM Falcon r5.11 segment L (historical `ibmq_manila` / Qiskit `FakeManilaV2`) |
| Qubits | 5 |
| Topology | Linear chain `0—1—2—3—4` (undirected unique edges from the FakeManila coupling map) |
| Native gates | `cx`, `rz`, `sx`, `x` (IBM basis; `id` omitted as a virtual identity) |
| Calibration date in source props | `2024-05-27T15:27:23-03:00` |

### Upstream sources

Checked against the public Qiskit IBM Runtime fake-provider fixtures (no IBM cloud token):

- [`conf_manila.json`](https://github.com/Qiskit/qiskit-ibm-runtime/blob/main/qiskit_ibm_runtime/fake_provider/backends/manila/conf_manila.json)
- [`props_manila.json`](https://github.com/Qiskit/qiskit-ibm-runtime/blob/main/qiskit_ibm_runtime/fake_provider/backends/manila/props_manila.json)

Mapped into Quon's fixed `BackendTarget` / `NoiseDescriptor` schema (`backend/src/descriptor.rs`):

- `gate_error` → fidelity `1 - gate_error` under `single_qubit_fidelity` / `two_qubit_fidelity`
- qubit `T1` / `T2` → `t1_us` / `t2_us` (µs, as published)
- qubit `readout_error` → `readout_error`
- mean `readout_length` (ns) → `meas_latency_us`

Directional CX fidelities are preserved for both coupling directions (`0,1` and `1,0`, …). Topology edges are stored undirected once, matching other Quon fixtures.

### Regeneration

```bash
# Optional: requires qiskit-ibm-runtime (maintainer machine only)
python python/ibm_snapshot_to_target.py \
  --backend fake_manila \
  --out targets/ibm/fake_manila_v2.json

# Or from downloaded conf/props JSON (no Qiskit import needed):
python python/ibm_snapshot_to_target.py \
  --conf path/to/conf_manila.json \
  --props path/to/props_manila.json \
  --out targets/ibm/fake_manila_v2.json
```

CI and local verification must use the checked-in JSON only — never live IBM Runtime.
