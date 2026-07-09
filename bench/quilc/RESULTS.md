# quonc vs quilc benchmark results

Target: linear 5-qubit, native 2Q = CX/CNOT. Within-20% flagged vs quilc.

| circuit | quonc 2Q | quilc 2Q | Δ2Q | quonc depth | quilc depth | Δdepth | input T |
|---|---:|---:|---:|---:|---:|---:|---:|
| bell | 1 | 1 | +0.0% | 6 | 4 | +50.0% | 0 |
| bernstein_vazirani | 14 | 5 | +180.0% | 23 | 21 | +9.5% | 0 |
| grover | 4 | 0 | n/a | 23 | 3 | +666.7% | 0 |
| ghz_4 | 3 | 3 | +0.0% | 8 | 6 | +33.3% | 0 |
| cnot_ladder_5 | 4 | 4 | +0.0% | 4 | 4 | +0.0% | 0 |
| qft_3 | 21 | 15 | +40.0% | 36 | 55 | -34.5% | 0 |
| ising_step | 6 | 6 | +0.0% | 15 | 14 | +7.1% | 0 |
| qaoa_k3 | 12 | 9 | +33.3% | 26 | 32 | -18.8% | 0 |
| clifford_t_phase | 2 | 2 | +0.0% | 10 | 7 | +42.9% | 4 |
| rx_rz_network | 6 | 6 | +0.0% | 24 | 28 | -14.3% | 0 |
| swap_via_cnot | 6 | 3 | +100.0% | 6 | 3 | +100.0% | 0 |
| deutsch_jozsa | 14 | 5 | +180.0% | 23 | 21 | +9.5% | 0 |

Both compilers fold `T` into `RZ(π/4)` before final metrics (quonc native decomp; quilc Xhalves ISA), so post-compile T-count is 0. The **input T** column is the logical Clifford+T count from the shared Quil source.

## Findings summary

- Within 20% of quilc on **both** 2Q count and depth: cnot_ladder_5, ising_step, rx_rz_network
- Lags quilc (outside band on 2Q and/or depth): bell (depth +50.0%), bernstein_vazirani (2Q +180.0%), grover (2Q n/a, depth +666.7%), ghz_4 (depth +33.3%), qft_3 (2Q +40.0%, depth -34.5%), qaoa_k3 (2Q +33.3%), clifford_t_phase (depth +42.9%), swap_via_cnot (2Q +100.0%, depth +100.0%), deutsch_jozsa (2Q +180.0%)
- T-count: both compilers absorb T into RZ before final metrics; see **input T** (e.g. `clifford_t_phase` has 4).
- Notable: `grover` is cancelled to 0 two-qubit gates by quilc (identity-like rewrite) while quonc still emits 4 CX — large depth/2Q gap.
- Methodology note: quilc is Rigetti's optimizing Quil compiler ([quil-lang/quilc](https://github.com/quil-lang/quilc)); this bench matches topology + 2Q native (CNOT) but 1Q natives differ (quonc: rz/sx/x; quilc: RZ + RX(k·π/2)). Depth definitions also differ (quonc schedule_time vs quilc gate-depth statistic).
