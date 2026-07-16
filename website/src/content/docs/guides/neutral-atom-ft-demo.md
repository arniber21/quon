---
title: Neutral-atom FT compiler demo
description: Compile a typed QEC program into verified neutral-atom schedule IR, QEC experiment JSON, Stim, and an analytic resource report.
---

This demo is the fastest way to inspect Quon's current QEC-aware
neutral-atom path. It starts from a typed surface-code program, lowers through
the compiler, emits canonical `quantum.na` schedule IR, writes QEC experiment
artifacts, and verifies the neutral-atom schedule.

Run commands from the repository root after completing the
[installation guide](/getting-started/install/). The command uses Devbox so the
LLVM/MLIR, Python, Node, and native-library environment matches CI.

## Run the demo

```bash
devbox run -- cargo run -p quonc -- examples/na_qec/surface_d3_cx.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-mlir /tmp/quon-surface-d3-cx.mlir \
  --emit-resource-report /tmp/quon-surface-d3-cx.report.json \
  --emit-qec-experiment /tmp/quon-surface-d3-cx.qec.json \
  --verify-na \
  --quiet
```

The command writes four artifacts:

| Artifact | Purpose |
| --- | --- |
| `/tmp/quon-surface-d3-cx.mlir` | Canonical neutral-atom schedule IR (`quantum.na`). |
| `/tmp/quon-surface-d3-cx.report.json` | Analytic resource report for the compiled schedule. |
| `/tmp/quon-surface-d3-cx.qec.json` | Semantic QEC experiment metadata. |
| `/tmp/quon-surface-d3-cx.stim` | Sibling structure-level Stim artifact emitted with the QEC JSON. |

`--verify-na` runs the schedule verifier. QEC-backed neutral-atom compiles also
auto-verify, so the flag is included here to make the validation step visible.

## Source program

The checked-in source is
[`examples/na_qec/surface_d3_cx.qn`](https://github.com/arniber21/quon/blob/main/examples/na_qec/surface_d3_cx.qn):

```kotlin
fn surface_d3_cx(): Q<Bit> = run {
        a <- surface_code<3>()
        b <- surface_code<3>()
        a <- memory_round(a)
        b <- memory_round(b)
        (a, b) <- logical_cx(a, b)
        _ <- measure_logical_z(a)
        measure_logical_z(b)
}
```

This exercises the typed `QecBlock<F, d>` path, memory rounds, a surface-code
logical CX, logical measurement, neutral-atom lowering, resource accounting,
QEC experiment emission, and schedule verification.

## Schedule IR excerpt

The MLIR-like schedule contains explicit neutral-atom operations such as atom
transfers, movement, and entangling layers:

```text
module {
  "quantum.na.schedule"() ({
    "quantum.na.layer"() ({
      "quantum.na.transfer"() {
        aod_id = 0 : i64,
        atom = 0 : i64,
        direction = "slm_to_aod",
        duration_us = 15 : i64
      } : () -> ()
    }) {cycle = 0 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {
        duration_us = 1 : i64,
        pairs = "[...]"
      } : () -> ()
    }) {cycle = 9 : i64} : () -> ()
  }) : () -> ()
}
```

Use this artifact when you want the schedule-level compiler output. The older
`--emit-na-schedule` flag writes a visualization/debug envelope instead.

## Resource report excerpt

The resource report is an analytic compiler artifact. It records schedule and
QEC metadata from the target model; it is not sampled decoder evidence.

```json
{
  "evidence_kind": "analytic",
  "estimated_cycles": 272,
  "physical_atoms": 57,
  "logical_qubits": 3,
  "code_family": "surface_code_like",
  "distance": 3,
  "memory_rounds": 2,
  "error_budget": {
    "rydberg": 0.048,
    "measurement": 0.027,
    "reset": 0.024,
    "movement": 0.036,
    "transfer": 0.168,
    "idle": 1.6e-8
  }
}
```

`error_budget` entries are rate-times-count estimates from the target
descriptor. They are useful for compiler comparisons, but they are not logical
failure rates and not threshold claims.

## QEC experiment excerpt

`--emit-qec-experiment` writes semantic QEC metadata and a sibling Stim file:

```json
{
  "schema_version": 1,
  "kind": "qec_experiment",
  "family": "surface",
  "code_family": "surface_code_like",
  "distance": 3,
  "rounds": 2,
  "logical_ids": [0, 1, 2],
  "measurement_schedule_len": 13,
  "logical_observables_len": 2,
  "na_refs_len": 13,
  "stim_file": "quon-surface-d3-cx.stim"
}
```

The Stim artifact is structure-level:

```text
# Quon QEC experiment - lattice-surgery CX structure (no noise; ADR-0019/0024)
# family=surface distance=3 blocks=3 (L-shaped: control|ancilla / target)
# Merge/ancilla outcomes -> OBSERVABLE_INCLUDE via frame; not bare DETECTORs.
# Note: simplified merge/split model; not Stim FT-distance claim.
QUBIT_COORDS(1, 1) 0
QUBIT_COORDS(3, 1) 1
```

Sampled Stim/Sinter results are separate from the compiler resource report
today. When you run the Python Sinter tooling, keep sampled logical-failure
evidence separate from analytic compiler metrics unless a workflow explicitly
joins them with provenance.

## What this proves

- The frontend accepts QEC-specific typed source.
- The compiler lowers the QEC workload into neutral-atom schedule artifacts.
- The neutral-atom verifier checks the emitted schedule.
- The resource report exposes target-derived analytic cost/error fields.
- QEC experiment JSON and Stim artifacts are emitted from the same compiled
  workload.

## Current limitations

- This is not a threshold claim.
- The surface-code schedule is intentionally scoped and is not presented as a
  full fault-tolerant distance proof.
- The lattice-surgery CX path is currently a fixed-layout model, not a general
  patch-operation planner.
- The target descriptor is a generic public model, not a proprietary hardware
  calibration.
- Logical non-Clifford operations such as magic-state-consuming T and CCZ are
  not implemented yet.
- Sampled Stim/Sinter results are not fused into the compiler report yet.

For broader backend details, see [Backends and verification](/guides/backends/)
and the [`quonc` CLI reference](/reference/quonc/).
