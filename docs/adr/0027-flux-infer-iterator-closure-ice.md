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

## Related refinement limitations (issue #404)

While restoring full Flux CI coverage (#404), the same closure-projection
weakness and a second `flux-infer` panic surfaced in more crates. These are
not the original ICE but are tracked here because they share the same
`#[trusted]` workaround convention and upstream root cause.

### `quon_qec/src/lattice_surgery.rs` — length-propagation gaps

Four functions (`right_column_data`, `bottom_row_data`, `rough_merge_round`,
`smooth_merge_round`) carry `#[cfg_attr(feature = "flux", flux_rs::trusted)]`.
flux-infer cannot:

- prove `row.len() == d` for `chunks(d)` sub-slices (`right_column_data`);
- discharge the nonlinear fact `d*d >= d` for a `len - d` slice
  (`bottom_row_data`);
- propagate the runtime `len() == n` guards into loop-body slice indexing
  (`rough_merge_round`, `smooth_merge_round`).

The runtime guards make all four safe; none carry flux specs, so trusting
them skips no verification.

### `quon_qec/src/qldpc.rs` — real underflow fix

`toy_repetition_graph(distance)` computed `n_checks = distance - 1` with no
guard, so Flux (correctly) flagged a possible `u32` underflow on `distance =
0`. This was a genuine bug, not a toolchain limitation: a `distance == 0`
early-return returning an empty graph was added, which also lets Flux see
`distance >= 1` past the guard.

### `quon_na` — pervasive flux-infer ICEs

`cargo flux -p quon_na --no-default-features --features flux` hits *multiple*
`flux-infer` internal compiler errors across the crate:

- `crates/flux-infer/src/infer.rs:483` — "impossible case reached" in
  `InferCtxt::move_to` (shape mode);
- `crates/flux-infer/src/projections.rs:382` — the original closure-projection
  panic, in impl-block methods that chain iterator adapters with closures.

Five `quon_na/src/compaction.rs` functions (`exclusive_cycle_asap`,
`critical_path_report`, `hard_dep_cycle_order_ok`,
`find_first_improving_merge`, `try_merge_pair`) are marked `#[trusted]` to
clear their attributable out-of-bounds refinement errors (the same
length-propagation gap as the lattice-surgery functions above). The remaining
failures are pure toolchain ICEs in impl-block methods and are **not**
individually worked around — see the CI decision below.

### CI handling

The Flux workflow (`.github/workflows/flux.yml`) pins the Flux toolchain to
commit `57773e946c619199a733350023164eaeca49e1a0` so every ICE is
reproducible with actionable diagnostics. The `flux_verify` and `quon_na`
jobs use `continue-on-error: true` while upstream flux-infer panics on these
patterns: the runs stay visible (the ICE output is printed) but do not gate
the workflow. `quon_core`, `backend`, and `quon_qec` pass cleanly and remain
hard gates.

Remove the `continue-on-error` flags (and re-enable verification on the
`#[trusted]` functions) once flux-infer can project length refinements through
closure-based iterator adapters without panicking.
