# Research samples

Literate notebooks (paired with `.py` smoke twins) deriving resource
estimates, benchmark results, or other research-style narratives — deeper
and more exploratory than a fixed demo program.

## Status

Owned by pack **#190**. Holds three literate notebooks (each a real
`nbformat` 4 `.ipynb`, unexecuted — `execution_count: null`, `outputs: []`
throughout, since a full Jupyter kernel is out of scope for this repo) plus
a shared repro-appendix template.

## Notebook vs. verifier: the split this pack keeps

- **Narrative** (this directory): a notebook derives *why* a number should
  be what it is, and its paired `.py` smoke twin regenerates that number
  against a real `quonc` build (and, where numerical, Qiskit Aer) to confirm
  the derivation. Smoke twins are standalone, runnable scripts
  (`python samples/research/<slug>_smoke.py`) — not wired into
  `just ci-rust`'s `test/verify/*.py` loop.
- **Verifier** (`test/verify/`, `test/na/`): compiler correctness oracles
  that already gate these same circuits' compiled semantics in CI
  (`just ci-rust`). This pack imports/links those files — see each
  notebook's "Linked canonical source(s)" note — rather than forking a
  second copy, per [`../CONTRIBUTING.md`](../CONTRIBUTING.md).
- The underlying `.qn` circuits a notebook narrates get real `quonc`
  typechecking two ways: either they already carry a `ci: smoke` catalog row
  under another pack (e.g. `visualization/dense-swap-mismatch` for
  `qaoa.qn`), or this pack registers one directly (e.g.
  `research/bernstein-vazirani-oracle`). The `.ipynb` notebook files
  themselves are `ci: none` — `quonc` cannot typecheck a notebook, and a
  full kernel execution is explicitly out of scope (see the issue).

## Seeds

| Catalog id | Path | Notes |
| --- | --- | --- |
| `research/compiler-experiment-log` | [`compiler_experiment_log.ipynb`](./compiler_experiment_log.ipynb) | Targets/SABRE-lookahead -> depth & gate-count log, over linked `test/verify/qaoa.qn`. Smoke twin: [`compiler_experiment_log_smoke.py`](./compiler_experiment_log_smoke.py). |
| `research/algorithm-correctness-narrative` | [`algorithm_correctness_narrative.ipynb`](./algorithm_correctness_narrative.ipynb) | Derives BV/Grover success probabilities, confirms against Aer. Smoke twin: [`algorithm_correctness_narrative_smoke.py`](./algorithm_correctness_narrative_smoke.py). Links `test/verify/bernstein_vazirani.qn`, `test/verify/grover.qn` (registered here as `research/bernstein-vazirani-oracle`, `research/grover-n4-marked-11`, `ci: smoke`). |
| `research/na-resource-study` | [`na_resource_study.ipynb`](./na_resource_study.ipynb) | Routing-agnostic vs. routing-aware NA placer resource reports; cross-checks #189's checked-in goldens, compiles new routing-aware cells fresh. Smoke twin: [`na_resource_study_smoke.py`](./na_resource_study_smoke.py). Links `test/na/bell.qn`, `test/na/qaoa_graph.qn` (`neutral-atom/bell-pair`, `neutral-atom/qaoa-maxcut`, #192). |
| `research/repro-appendix-template` | [`repro_appendix_template.md`](./repro_appendix_template.md) | Template "Repro appendix" section every notebook above ends with (commit, `quonc --version`, Python deps, smoke-twin invocation, linked sources). |

## Environment / Python deps

- Rust: `cargo build --release -p quonc`; every command in these notebooks
  assumes `QUONC=$PWD/target/release/quonc` (or `quonc` on `PATH`).
- Python: only `algorithm_correctness_narrative.ipynb` needs optional deps
  (`qiskit`, `qiskit-aer`, `qiskit-qasm3-import` — see
  [`../../python/requirements.txt`](../../python/requirements.txt),
  `pip install -r python/requirements.txt`). The other two notebooks' smoke
  twins are pure-stdlib (`json`, `subprocess`) plus `quonc` itself.
- Quon commit / version expectations: pinned per-notebook in each "Repro
  appendix" section — see
  [`repro_appendix_template.md`](./repro_appendix_template.md).

## Do not fork the stub's old placement

This directory previously held only a placeholder seed
(`qec_notebook_stub.md`); #190 has now replaced it with the real notebooks
above. Coordinate here before adding a fourth notebook that would duplicate
one of these three stories — one canonical artifact per story (see
[`../CONTRIBUTING.md`](../CONTRIBUTING.md)).
