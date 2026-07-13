# quonfmt

Canonical formatter for Quon (`.qn`) source.

```bash
# Format to stdout
cargo run -p quonfmt -- program.qn

# Format in place
cargo run -p quonfmt -- -w program.qn

# CI check (exit 1 if would change)
cargo run -p quonfmt -- --check program.qn
```

Style rules: [`docs/quonfmt-style.md`](../docs/quonfmt-style.md).

Comments are stripped on format. **Leading comments used as LSP documentation
(hover / completion) are therefore removed by `quonfmt`** — see
[`docs/quonfmt-style.md`](../docs/quonfmt-style.md) and
[`docs/adr/0010-leading-doc-comments.md`](../docs/adr/0010-leading-doc-comments.md).
Uses `frontend` with `default-features = false` — no MLIR/LLVM toolchain required.
