# Quon

Quon is an MLIR-based optimizing compiler for quantum programs. It accepts a functional source language with linear types and lowers programs onto **hardware targets** (`BackendTarget` / `TargetKind`: fixed gate-model, neutral-atom reconfigurable, and future families). OpenQASM 3.0 is a convenient intermediary emit form for fixed targets (Aer verification and tooling interop), not the compiler's primary backend identity — see ADR-0010.

## Language

### Source language

**Circuit**: A value of type `Circuit<n, m, d, C>` — a unitary quantum morphism consuming `n` qubits, producing `m` qubits, with gate depth bounded by `d` and Clifford classification `C`. The central type in Quon.
_Avoid_: gate, unitary, operation

**Qubit**: A single linear quantum register element. Must be consumed exactly once; cannot be copied (no-cloning).
_Avoid_: quant, q

**QReg**: A linear qubit register of statically known size `n`, written `QReg<n>`. A single linear value; must be destructured to access individual qubits.
_Avoid_: register, qubit array

**Quantum Monad**: The type `Q<τ>` — a quantum computation that may perform mid-circuit measurement and return a value of type `τ`. Computations in `Q` are written using `run { }` blocks.
_Avoid_: Q type, monadic computation

**Linear context**: The typing context `Δ` tracking resources that must be consumed exactly once. Distinct from the unrestricted context `Γ` for classical values. Represented as a `HashMap<Name, Type>` with physical removal on use.
_Avoid_: linear environment, usage context

**DepthExpr**: A symbolic arithmetic expression over static `Nat` literals and runtime `Int` variables representing a circuit's gate depth bound. Operators: addition, multiplication, max, saturating subtraction, division (constant divisor only), and exponentiation (constant exponent only) — the latter three exist for recursive value-dependent kernels (QFT's `2^(i+1)` angle, Shor's register arithmetic) rather than the core composition algebra. Defined once in the MLIR-free `quon_core` crate and shared by both `frontend` and `mlir_bridge` — the literal variant is `Nat`, the composition algebra is `seq` (sequential, `+`) / `par` (parallel, `max`) / `repeat` (`*`) / `controlled` (`+1`), and it serializes to S-expressions for MLIR attributes. Distinct from the surface `NatExpr` (full `Nat` arithmetic with holes), which the type checker normalizes into a `DepthExpr`.
_Avoid_: depth expression, depth annotation, depth index

**Clifford classification**: An inferred two-valued label (`Clifford` | `Universal`) on every `Circuit` type. Inferred bottom-up from gate primitives during type checking; never annotated by the user. User-supplied annotations are checked against the inferred value.
_Avoid_: gate class, circuit class

**Borrow block**: A scoped ancilla qubit allocation written `borrow q: Qubit in { body }`. The body must terminate with `reset(q)` or `discard(q)` — the type checker enforces this as a structural constraint on the final use of `q`.
_Avoid_: ancilla block, borrow scope

**Sequential composition**: The `|>` operator. Chains two circuits end-to-end; depth adds.
_Avoid_: compose, then, pipe

**Parallel composition**: The `par` keyword. Tensor-products two circuits on disjoint qubit sets; depth is the max of both.
_Avoid_: tensor, parallel, par

**Parametric specialization**: The mechanism (`frontend/src/elaborate.rs`) that turns a `Nat`/`Int`/`Float`-parameterized circuit function into a fully monomorphic, first-order gate tree at a concrete call site — partial evaluation of the classical parameters (loop bounds, angles, register widths), not a separate parametric IR. Memoized per call site in `LoweringCtx::specialized` so the same instantiation (e.g. `qft(3)` reached twice) is lowered once. Distinct from Z3-checked symbolic `DepthExpr`s in the type system, which stay symbolic; specialization only runs when lowering to MLIR needs a concrete gate sequence.
_Avoid_: monomorphization, template instantiation

### IR dialects

**quantum.circ**: The purely unitary MLIR dialect. All ops are unitary; no measurement. Every `!qubit` SSA value has exactly one use (enforced by a standalone region verifier pass). ZX-calculus rewriting and Clifford+T optimization run here.
_Avoid_: circ dialect, unitary dialect

**quantum.dynamic**: The dynamic circuit MLIR dialect. Adds measurement (`!qubit → !bit`), reset, feed-forward (`cf.cond_br` on `!bit`), and barrier ops. Unitary sub-circuits are embedded as `unitary_region` blocks containing only `quantum.circ` ops.
_Avoid_: dynamic dialect, measurement dialect

**quantum.physical**: Not a separate dialect — `quantum.dynamic` ops annotated with hardware attributes (`phys_qubit`, `native_gate`, `fidelity`). Routing and scheduling modify these attributes in place.
_Avoid_: physical dialect, hardware dialect

### Backend

**BackendTarget**: A hardware descriptor, discriminated by `TargetKind`. The primary compile surface: choose a target, then emit architecture-appropriate artifacts (schedules, resource reports, hardware IR, or — for fixed targets — OpenQASM as an intermediary). The `Fixed` kind (gate-model hardware) combines a connectivity graph, native gate set, noise model (per-gate fidelity, T1/T2 times, readout error), and capability flags (`supports_mid_circuit_meas`, `supports_feed_forward`). Other kinds (e.g. `NeutralAtomReconfigurable`) carry an independent field set with no forced overlap — only an `id` is shared across all kinds.
_Avoid_: target, device, backend

**TargetKind**: The discriminant on `BackendTarget` separating architecture families (gate-model `Fixed`, `NeutralAtomReconfigurable`, and future families) whose descriptors have genuinely different shapes. Each kind owns its own payload rather than sharing fields — deliberately, so adding a new architecture family never forces awkward unused fields onto existing kinds.
_Avoid_: architecture kind, target type

**Native gate**: A gate in the BackendTarget's supported gate set. Gates not in the native set must be decomposed before emission. Tracked via the `native_gate : BoolAttr` attribute on `quantum.circ.gate` ops.
_Avoid_: supported gate, hardware gate

**OpenQASM intermediary**: OpenQASM 3.0 text emitted from a **fixed** target's physicalized `quantum.dynamic` IR (`--emit-qasm`). Used for Aer simulation and external tooling — not the definition of Quon's backend, and not the emit form for neutral-atom targets (those use schedule / `quantum.na` / resource reports; see ADR-0010, issue #167).
_Avoid_: primary output, universal backend target

### Optimization

**ZX-graph**: An auxiliary graph representation of a `quantum.circ` circuit used for non-local algebraic simplification. Nodes are Z- or X-spiders with phase angles; edges are wires or Hadamard boxes. Built on `petgraph::StableGraph`.
_Avoid_: ZX-diagram, spider graph

### Neutral-atom backend

**Logical qubit**: A backend-only, IR-level identifier assigned to a `quantum.dynamic` qubit after lowering, used to track its expansion into a code block of atoms. Has no representation in Quon source syntax — a user cannot annotate a source-level `Qubit` with error-correction properties. Distinct from the source-level `Qubit`, which is checked for linearity by the frontend independently of any backend expansion.
_Avoid_: QEC qubit

**Atom**: A single physical site occupant in the neutral-atom architecture-aware schedule — the physical unit that a logical qubit's code block expands into. Exists only in the neutral-atom backend (`quon_na`), below the frontend's linear type system.
_Avoid_: physical qubit

**Code block**: A group of atoms jointly implementing one or more logical qubits under a given error-correcting code family. A backend-only concept produced during neutral-atom lowering, never visible at the source-language level.
_Avoid_: code patch, logical block

**AOD movement**: The neutral-atom movement model where atoms move in row/column-coupled groups (as driven by acousto-optic deflectors), not freely and independently. The movement constraint that placement-routing scheduling in `quon_na` is built against — deliberately not a free-grid Manhattan-distance simplification, to stay faithful to the reproduced literature.
_Avoid_: grid movement, Manhattan movement
