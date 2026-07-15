# First-class `samples/` corpus with catalog.yaml

**Status:** Accepted (2026-07-15)

Narrative demos live under `samples/` with a fixed top-level taxonomy (`learning/`, `algorithms/`, `workflows/`, `visualization/`, `applications/`, `research/`, `neutral-atom/`) and a required machine-readable `catalog.yaml`. CI fixtures stay in `test/`; QEC compiler examples stay in `examples/na_qec/`; the website cookbook deep-links into samples rather than owning a parallel tree.

We rejected per-pack top-level dirs without a shared layout (drift) and README-only indexing (no consumer hook for regenerate scripts). Pack ownership is exclusive per artifact kind: NA pedagogy (#192), viz stress artifacts (#189), workflows (#188), research notebooks (#190) — one canonical `.qn` per story. Catalog entries may opt into `ci: smoke` (typecheck / Aer when numerical); default is `ci: none`.

## Consequences

- A catalog entry's `id` prefix, not its `path`, determines category
  membership. `quonc/tests/samples_catalog.rs` enforces that every `id`
  prefix is one of the seven taxonomy categories above; it does not require
  `path` to live under the matching `samples/<category>/` directory. This is
  deliberate: `neutral-atom/*` entries may point `path` at
  `examples/na_qec/*.qn` (the QEC compiler examples' canonical home) instead
  of duplicating the file under `samples/neutral-atom/`, per the pack's
  "link, don't fork" rule.
- The catalog is parsed with `serde_yaml` (aliased in `quonc/Cargo.toml` to
  the maintained `serde_yaml_ng` fork — upstream `serde_yaml` is
  archived/deprecated, RUSTSEC-2024-0320). This alias keeps the schema and
  all call sites unchanged; revisit if `serde_yaml_ng` itself stalls.
