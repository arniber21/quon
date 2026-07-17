# flux-infer ICE: iterator-closure projection chains

## Summary

`flux-infer` (the refinement-type checking backend for `cargo-flux`) hits an
internal compiler error when analyzing functions that chain iterator adapters
with closures — e.g. `.iter().filter(|x| …).collect()`, `.iter().find(|x| …)`,
`.iter().map(|x| …).collect()`.

The panic occurs in `flux-infer::projections` with the message
`"impossible case reached"`.

## Affected functions

Two functions in `quon_qec/src/experiment.rs` retain
`#[cfg_attr(feature = "flux", flux_rs::trusted)]` because of this ICE:

1. **`emit_stim_single_block_memory`** — uses `.iter().filter().collect()`
   and `.iter().find()` chains.
2. **`emit_stim_lattice_surgery_cx`** — uses `.iter()`, `.find()`,
   `.filter()`, `.map()` chains.

Neither function carries flux refinement specs, so marking them `trusted`
skips their bodies entirely — no verification coverage is lost.

## Reproduction

```sh
# Requires nightly Rust + cargo-flux + z3
cargo flux -p quon_qec --features flux
```

Without the `trusted` attrs, flux-infer panics during projection of the
closure-based iterator adapters.

## Workaround

Keep `#[cfg_attr(feature = "flux", flux_rs::trusted)]` on these two
functions with a comment referencing this document. Remove once the
upstream bug is fixed.

## Upstream tracking

- flux-rs/flux repository: <https://github.com/flux-rs/flux>
- The ICE is in the projection pass for closure-based iterator adapters.
