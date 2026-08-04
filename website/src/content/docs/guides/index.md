---
title: Guides
description: Task-oriented Quon guides — developer tooling, backends and verification, plotting, the neutral-atom FT demo, the maturation roadmap, and application and creative samples.
---

The guides are task-oriented: each one takes a goal — set up the language
server, compile for a backend, pretty-print results, run the neutral-atom
fault-tolerance demo — and walks it end to end with the exact commands. They sit
between the language guide (what the language *is*) and the reference (what each
flag *does*), answering "how do I do X with Quon?"

## Who this is for

Users applying Quon to a real workflow — setting up an editor, targeting a
backend, inspecting compiler output, or exploring the sample corpus. The guides
assume you have Quon installed (see
[getting started](/getting-started/install/)) and can compile a program.

## What this covers

- **Developer tooling** — language server, formatter, and linter from a source
  checkout.
- **Backends and verification** — fixed gate-model targets, neutral-atom
  targets, and Qiskit Aer verification.
- **Results and plotting helpers** — pretty-print Aer counts, compiler metrics,
  and neutral-atom reports with `python/quon_viz.py`.
- **Neutral-atom FT demo** — a typed QEC program compiled end-to-end into a
  verified neutral-atom schedule and resource report.
- **Maturation path** — the roadmap for hardening Quon toward production.
- **Application demos** — optimization, simulation, and a TSP sketch.
- **Creative & games** — playful programs that each teach one concept.
- **Visualizing neutral-atom schedules** — emit and render NAViz animations.

## Where to go next

For exact flag and subcommand definitions referenced throughout these guides,
see the [quonc CLI reference](/reference/quonc/). For the compiler stages the
guides invoke, read the [architecture](/architecture/compiler-internals/)
pages.

→ Start with [Developer tooling](./tooling/).
