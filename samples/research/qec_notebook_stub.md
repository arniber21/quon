# Research notebook stub (pack owner: #190)

This is a placeholder seed for the `research/` category, which is owned by
issue #190: literate QEC/resource-estimation notebooks paired with `.py`
smoke twins (per ADR-0025's pack-ownership table). It exists so
`samples/catalog.yaml` has a registered row for `research/` before #190
lands the real content.

## What #190 will add here

- A literate notebook (or `.md` walkthrough) deriving a QEC resource
  estimate or benchmark narrative, cross-referencing
  `docs/neutral_atom/qec_benchmark_methodology.md` and
  `python/quon_qec_benchmarks.py`.
- A `.py` smoke twin that regenerates the notebook's headline numbers in
  CI, following the pattern in `python/test_quon_qec_benchmarks.py`.

## Do not extend this stub

Coordinate with #190 before adding real narrative content here — one
canonical artifact per story (see `samples/CONTRIBUTING.md`).
