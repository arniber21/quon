---
title: Why Quon
description: The design rationale behind Quon — why linear types, MLIR, Rust, and a functional source language are the right wager for a typed quantum compiler.
---

The "Why Quon" section explains the design tradeoffs behind the compiler rather
than the compiler itself. It is for readers deciding *whether* Quon's approach is
worth investing in: the wager that linear types, a functional source language,
and an MLIR-backed lowering path together make quantum programs safer to write
and cheaper to verify than a circuit-building API on top of a matrix library.

## Who this is for

Evaluators, contributors, and curious readers who want the "why" behind the
"what" — the decisions that made each call in the compiler, and the
alternatives that were rejected. No tooling is required to read these pages;
they are prose, not tutorials.

## What this covers

- **Design philosophy** — the load-bearing ideas (a circuit is a typed value, a
  qubit is a linear value), the chosen stack (Rust, MLIR, functional source),
  and the tradeoffs each choice carries.

## Where to go next

For the language those decisions produce, read the
[language guide](/language/introduction/); for the compiler they shape, read the
[architecture](/architecture/compiler-internals/) pages.

→ Start with [Design philosophy](./philosophy/).
