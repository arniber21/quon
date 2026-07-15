# Routing sensitivity

Compares two compiler configs — the same `.qn` source compiled against two
different `BackendTarget` connectivity graphs — and reads the `--metrics`
diff to explain *why* the numbers moved. The mechanism (SABRE routing +
SWAP insertion), not the specific circuit, is the point.

## Status

Owned by pack [#188](https://github.com/arniber21/quon/issues/188). Catalog
id: `workflows/routing-sensitivity`.

## The circuit

`routing_sensitivity.qn` is a 4-qubit "hub" pattern — `CNOT @(0,2)`,
`CNOT @(0,3)`, `CNOT @(1,3)` — chosen because its interaction graph (0
talking to 2 and 3; 1 talking to 3) can't be embedded in a 4-node line
without at least one SWAP.

## Config A: the default all-to-all target

```bash
export QUONC=$PWD/target/debug/quonc   # cargo build -p quonc first
$QUONC samples/workflows/routing_sensitivity/routing_sensitivity.qn --metrics-json -
```

```json
{
  "target": { "id": "generic_openqasm" },
  "metrics": { "depth": 3, "gate_count": 3, "qubit_count": 4, "swap_count": 0 }
}
```

(fields trimmed to the ones that matter here — full snapshot is
`schema_version`-stable JSON, see the CLI reference). Every qubit pair is
adjacent on this target, so all three `CNOT`s route as-is: `depth == 3`,
one gate per `CNOT`.

## Config B: a 4-qubit linear chain

[`backend/tests/fixtures/device_linear_chain.json`](../../../backend/tests/fixtures/device_linear_chain.json)
restricts connectivity to edges `0-1, 1-2, 2-3` — a line. Qubit 0 and qubit 3
are three hops apart.

```bash
$QUONC samples/workflows/routing_sensitivity/routing_sensitivity.qn \
  --target backend/tests/fixtures/device_linear_chain.json \
  --metrics-json -
```

```json
{
  "target": { "id": "linear_chain_device" },
  "metrics": { "depth": 9, "gate_count": 12, "qubit_count": 4, "swap_count": 0 }
}
```

## Explaining the diff

`gate_count` triples (3 → 12) and `depth` triples (3 → 9). Add `--emit-qasm`
to see why:

```bash
$QUONC samples/workflows/routing_sensitivity/routing_sensitivity.qn \
  --target backend/tests/fixtures/device_linear_chain.json --emit-qasm
```

The output has runs like `cx q[1], q[2]; cx q[2], q[1]; cx q[1], q[2];` —
three CNOTs implementing one SWAP, the standard decomposition SABre inserts
to move a logical qubit onto an adjacent physical one before the "real" CNOT
can execute directly. Each of the three original CNOTs needed at least one
neighbor swapped into place first, which is where the extra 9 gates (12 − 3)
come from.

`swap_count` reads `0` in both snapshots here: it counts an explicit
`quantum.dynamic.swap` op, and this target's routing pass decomposes SWAPs
into native CNOTs (`native_gate_decomp (post-SWAP)`, stage 7 of the fixed
path in `--list-passes`) before the metrics pass runs — so on this path,
read the SWAP cost off `gate_count`/`depth`, not `swap_count`. This is a
real, current metrics-semantics wrinkle, not a bug this workflow is claiming
to fix; note it rather than paper over it.

## A second axis: tuning SABRE directly

The same comparison works by holding the target fixed and tuning the SABRE
cost model instead of switching targets — `--sabre-gamma`, `--sabre-beta`,
`--sabre-lookahead` (defaults `0.3`, `0.5`, `20`):

```bash
$QUONC samples/workflows/routing_sensitivity/routing_sensitivity.qn \
  --target backend/tests/fixtures/device_linear_chain.json \
  --sabre-lookahead 5 --metrics-json -
```

On this particular circuit the result is identical to the `--sabre-lookahead
20` default (`depth=9`, `gate_count=12`) — three CNOTs and a 4-qubit line
don't leave SABRE much room to disagree with itself. That's still a real
(negative) data point: `--sabre-lookahead` is a genuine knob on the routing
search's greediness, but it doesn't move every circuit's metrics. Re-run the
comparison on a larger fixture — e.g.
[`test/verify/spin_glass_qaoa.qn`](../../../test/verify/spin_glass_qaoa.qn)
against a wider device target — to see a case with more room for the two
lookahead settings to diverge.

## CI coverage

`samples/catalog.yaml`'s `workflows/routing-sensitivity` row is `ci: smoke`
against Config B (`--target device_linear_chain.json --metrics`) — that's
the one invocation CI actually runs. Config A's default-target run, every
`--metrics-json -` variant above, and the `--sabre-lookahead 5` tuning
example are prose-verified (re-run manually when touching the routing pass
or this README), not separately CI-gated; they exercise the same compile
path with different output flags, so the smoke entry's job is catching
routing/lowering regressions, not re-checking every flag combination.

## See also

- [`quonc` CLI reference — Target options](../../../website/src/content/docs/reference/quonc.md#target-options) —
  `--target`, `--sabre-gamma`/`--sabre-beta`/`--sabre-lookahead`.
- [`quonc` CLI reference — Metrics options](../../../website/src/content/docs/reference/quonc.md#metrics-and-watch-options) —
  `--metrics`, `--metrics-json`, `--metrics-snapshot compare`.
- [`workflows/pass-introspection`](../pass_introspection/README.md) — reads
  raw MLIR at each checkpoint instead of the metrics summary.
