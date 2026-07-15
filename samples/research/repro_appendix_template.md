# Repro appendix template (pack #190)

Every `samples/research/` notebook ends with a "Repro appendix" section built
from this template, filled in with the values that were actually used for the
run being narrated. Copy the section below verbatim into a new notebook's
final cell and fill in the blanks — don't invent numbers you haven't run.

## Why this exists

A literate notebook's numbers are only as trustworthy as the reader's ability
to regenerate them. Since a full Jupyter kernel is out of scope for this repo
(no `nbconvert --execute` in CI — see `samples/research/README.md`), every
notebook's code cells show real commands but ship with **no fabricated
outputs**: `execution_count: null`, `outputs: []`. The repro appendix is the
one place that pins down the exact environment + commit a reader needs to
run those commands themselves and get the numbers quoted in the prose.

## Template

```markdown
## Repro appendix

- **Quon commit:** `<git rev-parse HEAD>` (branch `<branch>`)
- **`quonc` version:** `<quonc --version>`
- **Build:** `cargo build --release -p quonc`
- **Python:** `<python3 --version>`, deps from
  [`python/requirements.txt`](../../python/requirements.txt)
  (`pip install -r python/requirements.txt`)
- **Smoke twin:** `python samples/research/<slug>_smoke.py` regenerates every
  headline number in this notebook and asserts on them; treat its exit code,
  not this prose, as the source of truth.
- **Linked canonical sources:** `<paths under test/ or examples/na_qec/ this
  notebook narrates, with the catalog `id`s that register them>`
```

## Filled-in example (from `algorithm_correctness_narrative.ipynb`)

```markdown
## Repro appendix

- **Quon commit:** `d26723141d5799a0e8107915b16c55ef3b9fa6f3` (branch
  `samples/190-research-notebooks`)
- **`quonc` version:** `quonc 0.1.0`
- **Build:** `cargo build --release -p quonc`
- **Python:** `Python 3.9+`, deps from
  [`python/requirements.txt`](../../python/requirements.txt)
  (`pip install -r python/requirements.txt`; needs `qiskit`, `qiskit-aer`,
  `qiskit-qasm3-import` — skip gracefully if not installed)
- **Smoke twin:** `python samples/research/algorithm_correctness_narrative_smoke.py`
- **Linked canonical sources:** `test/verify/bernstein_vazirani.qn`
  (`research/bernstein-vazirani-oracle`), `test/verify/grover.qn`
  (`research/grover-n4-marked-11`)
```

## Rules

- Never claim a number the paired `.py` smoke twin doesn't itself compute —
  see [`README.md`](./README.md)'s "Narrative vs verifier" split.
- Pin the commit and `quonc --version` you actually ran against; if you
  re-run the notebook's story against a later commit, update both.
- List every linked canonical source (`test/verify/*.qn`, `test/na/*.qn`,
  `examples/na_qec/*.qn`) the notebook narrates, with its `catalog.yaml` `id`,
  so a reader can jump straight to the `ci: smoke` row that already
  typechecks it.
