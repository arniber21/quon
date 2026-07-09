---
title: Quickstart
description: Compile a Bell pair for a fixed target, emit OpenQASM as an intermediary, and simulate with Qiskit Aer.
---

This walkthrough prepares two entangled qubits, compiles them with Quon's
**fixed** hardware path, emits OpenQASM 3 as an intermediary, and samples
measurements with Qiskit Aer. (Neutral-atom targets use schedule / resource-report
emit instead — see the [backends guide](/guides/backends/).)

Complete the [installation guide](/getting-started/install/) first. Run these commands from the Quon repository root with the Python virtual environment active:

```bash
source .venv/bin/activate
```

## 1. Write a Bell program

Create `bell.qn`:

```text
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}

fn main(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
```

`H @0` puts the first qubit into an equal superposition. `CNOT @(0, 1)` uses it as the control for the second qubit, producing the Bell state `(|00⟩ + |11⟩) / √2`.

The `main` function allocates two qubits, applies the circuit, and measures both. Quon's linear type checker ensures each qubit is consumed exactly once.

## 2. Compile (OpenQASM intermediary)

Compile the program with Quon's built-in all-to-all **fixed** target. `--emit-qasm`
writes the OpenQASM intermediary used by Aer — not Quon's only backend form:

```bash
./target/release/quonc bell.qn --emit-qasm > bell.qasm
```

Inspect the generated OpenQASM:

```bash
less bell.qasm
```

The output declares two qubits and two classical bits, applies the Bell gates, and measures both qubits. The generic target is the default, so this quickstart does not need a device descriptor or `--target`.

## 3. Simulate with Aer

Pipe the compiler output directly into Quon's Aer bridge:

```bash
./target/release/quonc bell.qn --emit-qasm \
  | python python/quon_aer.py --shots 1024 --seed 7
```

The output will resemble:

```text
00: 520
11: 504
```

Shot counts are samples, so the exact numbers can differ across dependency versions. The important result is that only `00` and `11` appear, each close to half of the 1,024 shots: the two measurements are correlated even though either value is individually random.

You have now completed Quon's **fixed-target** end-to-end path (OpenQASM as intermediary):

```text
Quon source → quonc (--target fixed) → OpenQASM 3 → Qiskit Aer
```

For the neutral-atom hardware path (`--emit-na-schedule` / `--emit-resource-report`),
see the [backends guide](/guides/backends/).
## Run the bridge without a pipe

The bridge can also invoke `quonc` itself. Point it at the release binary and pass the source file:

```bash
QUONC=./target/release/quonc \
  python python/quon_aer.py bell.qn --shots 1024 --seed 7
```

Both forms use the same compiler and simulator. The piped form makes the OpenQASM boundary explicit; the direct form is convenient for repeated experiments.
