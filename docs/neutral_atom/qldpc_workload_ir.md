# qLDPC-style workload IR and resource model (issue #285)

Prototypes an abstract qLDPC-style QEC workload path focused on
compiler/resource modeling, not full decoding or threshold validation.

## Scope

This is a **compiler/resource-model prototype**:

- Represents a parity-check graph
- Generates syndrome-extraction workload structure
- Estimates neutral-atom-relevant resource pressure

**Not implemented (by design):**

- Full decoder (no threshold claim)
- Hardware-specific calibration (use generic public assumptions)
- Full qLDPC routing (movement pressure is a rough proxy)

Unsupported features fail clearly with actionable diagnostics.

## Model

- `ParityCheckGraph` — `n_data` data qubits, `n_checks` check ancillas, and
  a list of parity checks (stabilizer generators with basis and support)
- `QldpcResourceEstimate` — resource pressure: check weight, connectivity,
  ancilla demand, measurement rounds, movement pressure, peak atom demand
- `generate_syndrome_rounds()` — generates per-round CNOT schedules from the
  parity-check graph

## Resource estimates

| Field | Description |
|-------|-------------|
| `n_data` | Number of data qubits |
| `n_checks` | Number of check ancillas |
| `distance` | Code distance |
| `max_check_weight` | Maximum check weight (connectivity per ancilla) |
| `avg_check_weight` | Average check weight |
| `edge_count` | Total check-to-data edges (CNOT count per round) |
| `measurement_rounds` | Number of syndrome-extraction rounds |
| `movement_pressure` | Average Manhattan distance (proxy for NA movement) |
| `peak_atoms` | Data + check ancillas |
| `estimated_cycles_per_round` | Max check weight × 2 (Z-then-X split) |

## Toy examples

- `toy_5qubit_graph()` — [[5,1,3]] code (4 checks, weight 5)
- `toy_repetition_graph(d)` — repetition code (d-1 checks, weight 2)
