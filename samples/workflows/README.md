# Workflow samples

Repeatable algorithm-development loops — how you actually work with `quonc`
day to day (edit, typecheck, inspect, verify, compare configs, debug a
rejection), not what a program computes. Narrative pedagogy for *what*
quantum programs mean lives in [`learning/`](../learning/README.md) and
[`algorithms/`](../algorithms/README.md); NA-specific pedagogy lives in
[`neutral-atom/`](../neutral-atom/README.md) (pack #192) — this category
links to both rather than duplicating them.

## Status

Owned by pack [#188](https://github.com/arniber21/quon/issues/188). Every
workflow below has its own subdirectory with its own `README.md` and a
matching row in [`../catalog.yaml`](../catalog.yaml).

## Workflows

| Workflow | What it shows | Catalog id(s) |
| --- | --- | --- |
| [`edit_verify_loop/`](./edit_verify_loop/README.md) | The core edit → typecheck → emit QASM → Aer-verify loop; adds an Aer checker patterned on `test/verify` | `workflows/edit-verify-loop` |
| [`pass_introspection/`](./pass_introspection/README.md) | Reading `--list-passes` and a `--dump-ir` snapshot to see where a pass changed a program's shape | `workflows/pass-introspection` |
| [`routing_sensitivity/`](./routing_sensitivity/README.md) | Comparing two compiler configs (targets, or SABRE params) on the same `.qn` and explaining the `--metrics` diff | `workflows/routing-sensitivity` |
| [`na_path/`](./na_path/README.md) | Compile + schedule JSON + resource report on the neutral-atom path (links #192 for NA narrative) | `workflows/na-path` |
| [`failure_clinic/`](./failure_clinic/README.md) | Broken/fixed pairs for a linearity ("no-cloning") rejection and a borrow-escape rejection | `workflows/failure-clinic-linearity-broken`, `-linearity-fixed`, `-borrow-broken`, `-borrow-fixed` |

That's five workflows, eight catalog rows, and one `ci: smoke` happy path
per workflow-with-a-clean-compile (`edit-verify-loop`, `pass-introspection`,
`routing-sensitivity`, `failure-clinic-linearity-fixed`) — see each
subdirectory's `README.md` for the copy-paste commands.

## Adding a workflow

Follow [`../CONTRIBUTING.md`](../CONTRIBUTING.md): one subdirectory per
workflow story, its own `README.md` with copy-paste commands, and a matching
row (or rows, for a broken/fixed pair) in `../catalog.yaml`. Keep the NA
narrative itself out of this category — call NA flags if the workflow needs
them (as `na_path/` does), but link to
[`neutral-atom/`](../neutral-atom/README.md) (#192) for *why* the NA output
looks the way it does.
