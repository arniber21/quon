# Quon sample corpus

Narrative, regenerable demos for learners, researchers, and toolkit
consumers — distinct from the CI fixtures in [`test/`](../test/) and from
the compiler/QEC examples in [`examples/na_qec/`](../examples/na_qec/).
Locked layout and rationale: [ADR-0025](../docs/adr/0025-samples-corpus-layout.md);
domain terms ("Sample", "Sample catalog"): [`CONTEXT.md`](../CONTEXT.md#sample-corpus).

## Taxonomy

```text
samples/
  README.md            <- this file
  catalog.yaml          <- machine-readable index of every sample (required)
  CONTRIBUTING.md        <- how to add a sample
  learning/             <- one concept per file, narrated for newcomers
  algorithms/           <- named algorithms / canonical constructions
  workflows/            <- edit -> verify loops, pass dumps, failure clinics (#188)
  visualization/        <- schedule/graph goldens, "what you should see" (#189)
  applications/         <- real-world use-case framing
  research/             <- literate notebooks + .py smoke twins (#190)
  neutral-atom/         <- NA pedagogy: zoned vs flat, RAP placer (#192)
```

Each top-level category has its own `README.md` with a `## Status` section
naming its pack owner (if any) — see the pack-ownership table below.

## Boundaries

- **`test/`** stays CI fixtures only (correctness oracles for the compiler
  itself); it is not a source of narrative content, though samples may
  *link* to a `test/verify/*.qn` fixture rather than forking a copy.
- **`examples/na_qec/`** stays the compiler/QEC examples home; NA samples
  link to it (see [`neutral-atom/README.md`](./neutral-atom/README.md))
  rather than moving or duplicating its `.qn` sources.
- The [website cookbook](../website/src/content/docs/cookbook/) deep-links
  into this catalog rather than owning a second, parallel tree of examples.

## Catalog

[`catalog.yaml`](./catalog.yaml) is the **required**, machine-readable index
of every sample: `id`, `path`, `tags`, `difficulty`, `quonc_args`,
`artifacts`, and `ci` (`smoke` or `none`). A contribution is not complete
until it has a row here. Schema, path-existence, category-coverage, and
required-README-section checks live in
[`quonc/tests/samples_catalog.rs`](../quonc/tests/samples_catalog.rs).

## CI

- Default is `ci: none` — no CI cost beyond the catalog-schema lint.
- Opt in with `ci: smoke` to have `quonc` actually typecheck (and lower)
  your sample in CI. Claim numerical correctness (e.g. "Aer confirms...")
  only if you also add an Aer check, following the pattern in
  [`test/verify/`](../test/verify/).
- Runs as part of `just ci-rust`'s workspace test suite (no separate CI
  step); run it standalone with `just ci-samples`. See
  [`docs/agents/validation.md`](../docs/agents/validation.md).

## Pack ownership

One canonical `.qn` (or notebook) per story — other packs link or
regenerate rather than forking a copy:

| Pack | Owner issue | Owns |
| --- | --- | --- |
| Workflows | [#188](https://github.com/arniber21/quon/issues/188) | edit -> verify loops, pass dump, failure clinic |
| Viz goldens | [#189](https://github.com/arniber21/quon/issues/189) | stress artifacts + "what you should see" |
| Research notebooks | [#190](https://github.com/arniber21/quon/issues/190) | literate notebooks + `.py` smoke twins |
| NA pedagogy | [#192](https://github.com/arniber21/quon/issues/192) | walkthroughs, zoned vs flat, RAP placer story |

`learning/`, `algorithms/`, and `applications/` have no dedicated pack —
contribute directly against those categories.

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md).
