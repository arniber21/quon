---
title: Reference
description: The Quon reference — the quonc CLI and the compiler pipeline it runs — kept in sync with the implementation.
---

The reference section is the precise, implementation-aligned lookup for the
compiler driver and the pipeline it runs. It is the page to keep open while you
work: exact flags, subcommands, emission options, and the stage list `quonc`
runs from source text to backend artifact. The prose guides link here for
definitions; this section does not repeat the walkthroughs.

## Who this is for

Anyone who needs an exact fact about a flag, a subcommand, or a pipeline stage —
daily users, guide readers following a link, and contributors checking current
behavior against the docs. No tutorial context is assumed.

## What this covers

- **quonc CLI** — every flag and subcommand the compiler driver accepts, with
  examples for the common emission paths (QASM, neutral-atom schedule, resource
  report, NAViz).
- **Compiler pipeline** — the high-level stage list `quonc` runs (parse,
  typecheck, elaborate, lower to MLIR, optimize, emit), with the
  architecture pages going inside each stage.

## Where to go next

For walkthroughs that use these flags in context, see the [guides](/guides/);
for what each pipeline stage does internally, read the
[architecture](/architecture/compiler-internals/) pages.

→ Start with the [quonc CLI](./quonc/).
