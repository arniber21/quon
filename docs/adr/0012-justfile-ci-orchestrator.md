# Justfile is the CI / developer-orchestrator source of truth

Contributor and CI entrypoints go through the root Justfile (`just doctor`,
`just test-ci`, `just ci-rust`, …). Devbox owns the Nix toolchain and
`init_hook`; Actions runs `devbox run -- just <recipe>` (or a lightweight
`just` install for docs-only jobs). Rejected alternatives: a custom
`scripts/quon-dev` dispatcher, cargo-xtask (chicken-egg with a broken MLIR
env), and keeping Actions YAML as the checklist with Just as a local mirror
only — the last drifts the same way the old validation docs did.
