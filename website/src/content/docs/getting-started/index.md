---
title: Getting Started
description: Install Quon from source, compile your first Bell pair, and move on to a teleportation program — the shortest path from checkout to a verified circuit.
---

Getting started is the shortest path from a source checkout to a compiled,
simulated Quon program. Three pages take you from toolchain setup to a
teleportation circuit that exercises the Quantum Monad and classical
feed-forward — enough to confirm the install works and see the shape of the
language before reading the language guide.

## Who this is for

First-time users with a Rust toolchain (or Devbox) who want to build the
compiler, compile a program, and sample it on a simulator. No prior Quon
knowledge is assumed; the quickstart uses a checked-in program so you do not
write any `.qn` yourself yet.

## What this covers

- **Install Quon** — set up the pinned compiler toolchain and the optional
  verification dependencies (Qiskit Aer, lit) from source.
- **Quickstart** — compile a Bell pair to OpenQASM 3, sample it on Qiskit Aer,
  and emit a neutral-atom schedule in four commands.
- **Your second program** — a teleportation circuit with `QReg` destructuring,
  the Quantum Monad, measurement, and classical feed-forward control.

## Where to go next

Once the install works, the [language guide](/language/introduction/) explains
each construct used here in depth, and the [cookbook](/cookbook/) walks complete
CI-verified programs.

→ Start with [Install Quon](./install/).
