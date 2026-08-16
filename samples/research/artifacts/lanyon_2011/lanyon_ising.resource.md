# Neutral-atom analytic resource report

## Qubit resources
| Metric | Value |
| --- | ---: |
| Logical qubits | 2 |
| Physical atoms | 2 |

## Schedule metrics
| Metric | Value |
| --- | ---: |
| Estimated cycles | 94 |
| Bottleneck | rearrangement |
| Rydberg stages | 8 |
| Rearrangement steps | 1 |
| Rearrangement time (µs) | 336 |
| Trap transfers | 4 |
| Transfer time (µs) | 60 |
| Entangle2 count | 8 |
| EntangleN count | 0 |
| Measurement rounds | 1 |
| Reset rounds | 0 |
| Wait time (µs) | 0 |
| Total time (µs) | 1956 |
| Routing-aware search completed layers | 8 |
| Routing-aware search fell back to greedy (layers) | 0 |

## Atom pressure & reuse
| Metric | Value |
| --- | ---: |
| Peak atoms | 2 |
| Measurement count (ops) | 2 |
| Reset count (ops) | 0 |
| Reuse count (ops) | 0 |
| Reused ancilla count | 0 |

## Physical error budget
| Category | Contribution (rate × count) |
| --- | ---: |
| Rydberg | 0.016 |
| Measurement | 0.003 |
| Reset | 0 |
| Movement | 0.0005 |
| Transfer | 0.0028 |
| Idle | 0 |

## Fidelity estimate (Enola Eq. (1))
| Metric | Value |
| --- | ---: |
| Gate fidelity product | 0.933601093572 |
| Estimated fidelity (with idle decay) | 0.933567484233 |

## Notes
- Compiler analytic metrics only — not fused with Python/Sinter sampled CSV; neither artifact is a threshold claim (ADR-0020).
- Field names align with TUM RAP Table I / Enola headline metrics.
- `estimated_cycles` is `layers.len()`; `bottleneck` is the max of rydberg stages / rearrangement time / transfer time / measurement rounds (ties → mixed; all-zero → none).
- Non-QEC reports omit atoms-per-logical and code-family rows.
- `peak_atoms` counts distinct simultaneously-allocated atoms; qubit reuse (measure → reset → reuse of the same ancilla) lowers it versus a fresh-ancilla-per-round variant. `measurement_count` / `reset_count` are per-op tallies, distinct from the per-layer `measurement_rounds` / `reset_rounds`.
- Physical error budget lines are analytic schedule-count × rate contributions only — not sampled logical failure rates (Sinter) or threshold claims.
- Fidelity estimate is an analytic Enola Eq. (1) product over the compiled schedule (`fidelity.cz` for entangling actions, `fidelity.single_qubit` for local/global-ry actions including Hahn-echo bystander pulses, `fidelity.atom_transfer` for trap transfers, `fidelity.coherence_time_us` for idle decay) — not a logical error rate, not sampled (Sinter), and not a threshold claim; distinct from the analytic physical error budget (ADR-0017, `error_budget`, when present) — a `rate × schedule-count` sum, not a fidelity product.
