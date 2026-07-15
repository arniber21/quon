# First-class `samples/` corpus with catalog.yaml

Narrative demos live under `samples/` with a fixed top-level taxonomy (`learning/`, `algorithms/`, `workflows/`, `visualization/`, `applications/`, `research/`, `neutral-atom/`) and a required machine-readable `catalog.yaml`. CI fixtures stay in `test/`; QEC compiler examples stay in `examples/na_qec/`; the website cookbook deep-links into samples rather than owning a parallel tree.

We rejected per-pack top-level dirs without a shared layout (drift) and README-only indexing (no consumer hook for regenerate scripts). Pack ownership is exclusive per artifact kind: NA pedagogy (#192), viz stress artifacts (#189), workflows (#188), research notebooks (#190) — one canonical `.qn` per story. Catalog entries may opt into `ci: smoke` (typecheck / Aer when numerical); default is `ci: none`.
