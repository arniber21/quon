# Auto-run `--verify-na` for QEC-backed NA compiles

Neutral-atom schedule verification is exposed as `quonc --verify-na` for emitted or input `quantum.na`. When the entrypoint is QEC-backed (uses `QecBlock` builtins), verification runs automatically on **any** QEC-backed NA compile — not only `--emit-na-mlir` — even without the flag. Physical (non-QEC) NA programs verify only when `--verify-na` is requested.

Checks cover occupancy, Rydberg range / min spacing, AOD movement/transfer consistency, measurement ordering, reset ordering, and Wait hard schedule barriers (any `quantum.na.wait` forces a later cycle). **Feed-forward / mid-circuit correction ordering is compaction-only** (`feed_forward_dependencies` in `compaction.rs`) and is **out of scope for #256** — the verifier does not claim full FF barrier coverage.

QEC round barriers and measurement dependencies are exactly where silent compaction bugs would invalidate experiments; always-on verify for every physical NA toy example would slow ordinary iteration without the same payoff.
