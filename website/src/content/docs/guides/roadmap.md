---
title: Maturation path
description: How Quon is being hardened from a serious compiler base into a production-oriented quantum software toolkit.
---

Quon's roadmap is organized around compiler-tooling maturity, not feature
sprawl. The core direction is to make the existing language, lowering,
backend, verification, and developer workflows easier to install, easier to
inspect, and harder to regress.

## Hardening themes

- **Boring installation.** Keep source builds reproducible with Devbox while
  turning the checked-in release scripts into a routine self-contained binary
  distribution path.
- **Reliable compiler workflows.** Keep `just doctor`, `just test-fast`, and
  `just test-ci` as the local paths that match CI expectations.
- **Inspectability.** Make IR dumps, metrics snapshots, OpenQASM output,
  neutral-atom schedules, and resource reports easier to compare across
  compiler changes.
- **Backend-aware validation.** Connect more source-level invariants to backend
  legality, timing, routing, movement, and resource accounting.
- **Benchmark discipline.** Preserve small readable examples while expanding
  regression coverage for representative quantum workloads.
- **Neutral-atom depth.** Evolve the neutral-atom path around first-class
  schedule inspection, stronger placement/movement comparisons, and clearer
  resource-model validation.
- **Optimization coverage.** Broaden circuit and dynamic optimization passes
  while keeping verifier-backed correctness as the default engineering bar.

## How to read roadmap issues

The issue tracker includes polish items, backend extensions, research ideas,
and stretch work. Those are not all missing core pieces. External readers should
interpret them as the next layers of hardening around a toolkit that already
has a typed frontend, compiler driver, backend target model, OpenQASM emission,
neutral-atom schedule/resource artifacts, and verification workflows.

## What belongs on the public surface

The public docs should lead with features represented in source, tests, and
commands. Roadmap work belongs here, in issues, and in design notes until it
has a runnable path, a test, and a clear command-line workflow.
