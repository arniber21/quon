# Auto-run `--verify-na` for QEC-backed NA emits

Neutral-atom schedule verification is exposed as `quonc --verify-na` for emitted or input `quantum.na`. When the entrypoint is QEC-backed (uses `QecBlock` builtins), verification runs automatically on emit even without the flag. Physical (non-QEC) NA programs verify only when `--verify-na` is requested.

QEC round barriers and measurement/feedforward dependencies are exactly where silent compaction bugs would invalidate experiments; always-on verify for every physical NA toy example would slow ordinary iteration without the same payoff.
