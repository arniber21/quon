# IBM backend targets

Frozen IBM-style `BackendTarget` JSON descriptors for noise-aware compilation
and Aer verification. **No live hardware access and no IBM API token are
required** — every artifact here is checked into git.

## Available snapshots

| File | Device | Notes |
|---|---|---|
| [`fake_manila_v2.json`](fake_manila_v2.json) | FakeManilaV2 (5q Falcon) | Source of truth for issue #117 |

See [`SOURCE.md`](SOURCE.md) for calibration provenance and regeneration notes.

## Usage

```bash
# Inspect the loaded target
cargo run -p quonc -- --print-target --target targets/ibm/fake_manila_v2.json

# Compile a verify program against Manila topology + noise
cargo run -p quonc -- --emit-qasm --target targets/ibm/fake_manila_v2.json \
  test/verify/bell.qn

# Optional: weight SABRE toward quieter edges (default γ = 0.3)
cargo run -p quonc -- --emit-qasm --target targets/ibm/fake_manila_v2.json \
  --sabre-gamma 1.0 test/verify/bernstein_vazirani.qn
```

## Noisy fidelity report (optional)

`python/noisy_fidelity.py` compares Hellinger fidelity vs ideal Aer for the
eight PRD reference programs under (1) all-to-all + same noise and (2) the full
IBM target (topology + noise). It is **report-only** and needs a local Qiskit /
Aer install (`python/requirements.txt`).

If those packages are unavailable, use the checked-in sample table:

- [`sample_fidelity_results.md`](sample_fidelity_results.md)

## Non-goals (see also issue #82)

- Live IBM queue / Runtime job submission
- Automatic snapshot refresh in CI
- Token-gated calibration pulls
