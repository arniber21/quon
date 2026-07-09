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

Comments are stripped on format. Uses `frontend` with the `parser-only` feature — no
MLIR/LLVM toolchain required.
