---
title: Results and plotting helpers
description: Pretty-print Aer counts, compiler metrics, and neutral-atom reports with the python/quon_viz.py helper instead of raw JSON.
---

Quon's compiler emits machine-readable JSON (counts from the Aer bridge,
`--emit-resource-report`, `--emit-na-schedule`), but reading raw JSON by eye
is slow. `python/quon_viz.py` (issue #196) is a small, dependency-light
presentation layer that turns those artifacts into bar charts and readable
tables. It mirrors the names you already know from Qiskit so a migration is
a one-line swap.

Install the deps once (matplotlib is the only new one):

```sh
pip install -r python/requirements.txt
```

## Instead of `qiskit.visualization.plot_histogram`

```python
import sys
sys.path.insert(0, "python")
import quon_viz

counts = {"00": 2050, "11": 2022}
quon_viz.plot_histogram(counts, title="Bell state", bar_labels=True,
                        filename="bell.png")
```

`plot_histogram` accepts the same keyword shape as Qiskit's — `figsize`,
`color`, `title`, `legend_keys`, `bar_labels`, `number_to_keep`, `sort` —
but is safe to call headless: it forces the non-interactive `Agg` backend and
**never calls `plt.show()`**. Pass `filename=` to save a PNG and return
`None`; omit it to get the `Figure` back for further tweaking. The verifier
scripts `test/verify/bell.py` and `test/verify/grover.py` already use it —
each writes a histogram under `$QUON_VIZ_DIR` (or `$TMPDIR/quon_viz`) when run.

## Pretty-print compiler metrics

`metrics_table` formats any flat (or lightly nested) metrics dict as an
aligned two-column table — useful for a quick depth / gate-count / SWAP
readout from your own benchmark loop:

```python
print(quon_viz.metrics_table({"depth": 12, "swaps": 3, "cx": 8,
                               "single_qubit_gates": 7, "fidelity": 0.9895}))
```

## Neutral-atom resource reports

`quonc --emit-resource-report` emits a flat JSON object. `summarize_na_report`
groups it into Resource / Qubits / Single-qubit gates / Fidelity blocks and
appends the `error_budget` and `temporal_atom_metrics` sub-objects:

```python
import json, quon_viz
report = json.load(open("report.json"))
print(quon_viz.summarize_na_report(report))
```

It accepts a parsed dict, a path to a JSON file, or a JSON string, so the
shortest form is just `quon_viz.summarize_na_report("report.json")`.

## Neutral-atom schedule timelines

`summarize_na_schedule` reads the `na_schedule_view` envelope from
`--emit-na-schedule` and prints one compact line per cycle
(`Move(2)`, `Entangle2(a0,a1)`, `Transfer(a0,SlmToAod)`, `rz(a1)`, …), with a
header summarizing zones and headline metrics. `max_layers=` truncates the
per-cycle listing while keeping the full-schedule metrics block:

```python
print(quon_viz.summarize_na_schedule("schedule.json", max_layers=10))
```

## End-to-end NA summary

`samples/research/na_resource_summary.py` compiles a `.qn` program with
`quonc` and prints both summaries in one shot — a runnable readout of what a
neutral-atom compile produced:

```sh
QUONC=target/release/quonc python samples/research/na_resource_summary.py \
    test/na/qaoa_graph.qn
```

## Optional: Bloch sphere

`plot_bloch(statevector, ...)` thin-wraps `qiskit.visualization.plot_bloch_multivector`
for the headless case (forces `Agg`, saves to `filename=`). It is **optional**:
if `qiskit.visualization` is unavailable it prints a notice and returns `None`
rather than failing.

```python
quon_viz.plot_bloch([1, 0], title="|0>", filename="bloch.png")
```

## CLI shortcut

The module is also runnable as a pretty-printer for a report or schedule file:

```sh
python python/quon_viz.py report path/to/report.json
python python/quon_viz.py schedule path/to/schedule.json --max-layers 8
```
