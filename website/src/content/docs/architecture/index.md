---
title: Architecture
description: How the Quon compiler transforms typed source into hardware-specific artifacts — the pipeline stages, the invariants each owns, and the neutral-atom backend model.
---

The architecture section is for readers who will open the compiler's source or
extend a pass. It documents *what runs where*: each stage of the pipeline, the
invariant that stage owns, and the neutral-atom hardware model the backend
lowering targets. The goal is to make the compiler legible — to turn "Quon
lowers typed source to OpenQASM 3 and neutral-atom schedules" into a map you can
follow in the codebase.

## Who this is for

Contributors and advanced users who want to read or modify the compiler — not
just invoke it. The pages assume you are comfortable with the language (see the
[language guide](/language/introduction/)) and want to understand what happens
after the typechecker accepts a program.

## What this covers

- **Compiler internals** — the pipeline from source text to backend artifact:
  parsing, typechecking, MLIR lowering, optimization passes, and
  target-specific emission, stage by stage with the file each lives in.
- **Neutral-atom model** — the hardware model, the target schema, and the
  scheduling approach (interaction graphs, Misra–Gries edge-coloring, AOD
  movement planning, compaction) behind the neutral-atom backend.

## Where to go next

The high-level pipeline is summarized in the
[compiler pipeline reference](/reference/compiler/); this section goes inside
it. For runnable end-to-end examples of the pipeline, see the
[cookbook](/cookbook/).

→ Start with [Compiler internals](./compiler-internals/).
