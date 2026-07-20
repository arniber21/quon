# Magic-state-consuming logical T and CCZ (issue #283)

Quon models magic-state-consuming logical non-Clifford operations (T, T†, CCZ)
as compiler-visible operations with explicit resource accounting and QEC
experiment metadata. This is a **compiler model of magic-state consumption**,
not a validated distillation factory or threshold claim.

## Source-level operations

- `logical_t(block)` — consumes one magic-state resource, applies T
- `logical_tdag(block)` — consumes one magic-state resource, applies T†
- `logical_ccz(a, b, c)` — consumes one magic-state resource, applies CCZ

All three are surface-code only. Type checking enforces:
- Compatible QEC block families (surface only)
- Equal code distances (for CCZ, all three blocks must match)
- Correct operation arity
- Distinct logical ids (for CCZ)

## Resource reports

Resource reports include:
- `t_count` — number of logical T gates
- `tdag_count` — number of logical T† gates
- `ccz_count` — number of logical CCZ gates
- `magic_state_demand` — total (T + Tdag + CCZ)

## QEC experiment JSON

The QEC experiment JSON exposes the logical non-Clifford operations with
their assumptions. In the Stim circuit, magic-state operations appear as
comments (no physical gates) — this is a compiler model, not a full
distillation factory.

## Limitations

- **No distillation factory**: magic states are consumed but not produced.
  The compiler does not model distillation circuits.
- **No physical gate expansion**: T/Tdag/CCZ are recorded as metadata, not
  expanded to physical CNOT/measure/reset rounds.
- **Surface-code only**: other code families are rejected with actionable
  diagnostics.
- **Not a threshold claim**: magic-state consumption is modeled for resource
  accounting, not for claiming fault-tolerant performance.
- **Not yet reachable from `.qn` source**: `logical_t`, `logical_tdag`, and
  `logical_ccz` exist as `WorkloadBuilder` methods (`quon_qec/src/workload.rs`)
  with expansion (`quon_qec/src/expand.rs`) and resource-report wiring, but
  the frontend typechecker/prelude was not updated to bind them as source
  identifiers. `examples/na_qec/surface_d3_t.qn` and `surface_d3_ccz.qn` sketch
  the intended syntax but do not currently compile through `quonc`
  (`unbound variable`); tracked in #311.
