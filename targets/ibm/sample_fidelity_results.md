# Sample noisy fidelity results (issue #117)

Checked-in stand-in for `python/noisy_fidelity.py` when Qiskit / Aer are not
installed in the local environment. Values below are **illustrative** of the
expected qualitative pattern for FakeManilaV2 (linear 5q + published noise):
all-to-all + same noise usually retains higher Hellinger fidelity vs ideal than
the topology-constrained IBM target, because SABRE inserts SWAPs on non-local
two-qubit gates.

Regenerate the live table with:

```bash
pip install -r python/requirements.txt
cargo build -p quonc
QUONC=target/debug/quonc python python/noisy_fidelity.py \
  --target targets/ibm/fake_manila_v2.json --shots 4096 --seed 1234
```

## Sample table (Hellinger fidelity vs ideal Aer)

| program | F_all2all (noise only) | F_ibm (topology + noise) | delta (ibm − all2all) |
|---|---:|---:|---:|
| bell | 0.97 | 0.97 | 0.00 |
| teleport | 0.93 | 0.90 | −0.03 |
| bernstein_vazirani | 0.91 | 0.86 | −0.05 |
| grover | 0.88 | 0.82 | −0.06 |
| qft | 0.85 | 0.76 | −0.09 |
| ising | 0.90 | 0.84 | −0.06 |
| qaoa | 0.87 | 0.79 | −0.08 |
| shor | 0.80 | 0.68 | −0.12 |

### Residual risk

- This sample table is **not** a live Aer measurement from this checkout.
- Absolute fidelities depend on Aer noise-model mapping (depolarizing + symmetric
  readout) and shot noise; treat deltas as directional, not golden numbers.
- Programs that fail to compile against the 5q Manila target (width / mid-circuit
  constraints) should be marked `ERROR` / `SKIP` by the live script rather than
  forced into this table.
