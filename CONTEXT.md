# Quon

Quon is an MLIR-based optimizing compiler for quantum programs. It accepts a functional source language with linear types and emits OpenQASM 3.0 for execution on Qiskit Aer or real hardware.

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

**Linear context**: The typing context `Δ` tracking resources that must be consumed exactly once. Distinct from the unrestricted context `Γ` for classical values. Represented as a `HashMap<Name, Type>` with physical removal on use. The **frontend adapter** of the no-cloning / linear-use judgment — paired with the **IR adapter** `Linearity (SSA)`; the two share vocabulary but not a type, and neither subsumes the other.
_Avoid_: linear environment, usage context

**Linearity (SSA)**: The MLIR-free use-count kernels in `quon_core::linearity` (SPEC §6.2–§6.3) — `LINEAR_USE_COUNT`, `classify_use_count`, `is_reuse_after_measure`, and the `barrier` / `if` / `unitary_region` qubit-threading checks. The **IR adapter** of the no-cloning / linear-use judgment: after lowering, source names are gone and "consume exactly once" is re-expressed as "every `!qubit` SSA value has exactly one use", checked by region verifier passes against these kernels. Paired with the frontend adapter `Linear context` (`Δ`); the two adapters stay as separate types in their owning crates (`frontend` vs `quon_core`) and share only vocabulary. These kernels *are* the enforcement of `quantum.circ`'s "every `!qubit` SSA value has exactly one use" invariant (and of the `quantum.dynamic` measure-reuse rule via `is_reuse_after_measure`); `Linearity (SSA)` is the kernel module, while the `quantum.circ` / `quantum.dynamic` entries name the dialect-level rules — not two separate judgments.
_Avoid_: linear environment, usage context, linearity check (unqualified)

**DepthExpr**: A symbolic arithmetic expression over static `Nat` literals and runtime `Int` variables representing a circuit's gate depth bound. Operators: addition, multiplication, max, saturating subtraction, division (constant divisor only), and exponentiation (constant exponent only) — the latter three exist for recursive value-dependent kernels (QFT's `2^(i+1)` angle, Shor's register arithmetic) rather than the core composition algebra. Defined once in the MLIR-free `quon_core` crate and shared by both `frontend` and `mlir_bridge` — the literal variant is `Nat`, the composition algebra is `seq` (sequential, `+`) / `par` (parallel, `max`) / `repeat` (`*`) / `controlled` (`+1`), and it serializes to S-expressions for MLIR attributes. Distinct from the surface `NatExpr` (full `Nat` arithmetic with holes), which the type checker normalizes into a `DepthExpr`.
_Avoid_: depth expression, depth annotation, depth index

**Clifford classification**: An inferred two-valued label (`Clifford` | `Universal`) on every `Circuit` type. Inferred bottom-up from gate primitives during type checking; never annotated by the user. User-supplied annotations are checked against the inferred value.
_Avoid_: gate class, circuit class

**Borrow block**: A scoped ancilla qubit allocation written `borrow q: Qubit in { body }`. The type checker requires each ancilla to be consumed exactly once inside the block and forbids it from appearing in the block’s result (no escape). Valid cleanup includes `measure`, `reset`, and `discard` (not only a structural `reset`/`discard` terminal — ADR-0003 / older PRD wording is stale; resolution tracked in issue #180).
_Avoid_: ancilla block, borrow scope

**Documentation comment**: Leading `--` line comments or `{- -}` block comments immediately above a top-level `fn` or `type` (only whitespace between the comment run and the declaration). Attached to the symbol for LSP hover and completion (ADR-0010). Not a separate syntax; ordinary comments in that position. `quonfmt` v1 strips comments, so formatting clears these docs from the file.
_Avoid_: doc comment syntax, `///`, javadoc

**Sequential composition**: The `|>` operator. Chains two circuits end-to-end; depth adds.
_Avoid_: compose, then, pipe

**Parallel composition**: The `par` keyword. Tensor-products two circuits on disjoint qubit sets; depth is the max of both.
_Avoid_: tensor, parallel, par

**Parametric specialization**: The mechanism (`frontend/src/elaborate.rs`) that turns a `Nat`/`Int`/`Float`-parameterized circuit function into a fully monomorphic, first-order gate tree at a concrete call site — partial evaluation of the classical parameters (loop bounds, angles, register widths), not a separate parametric IR. Memoized per call site in `LoweringCtx::specialized` so the same instantiation (e.g. `qft(3)` reached twice) is lowered once. Distinct from Z3-checked symbolic `DepthExpr`s in the type system, which stay symbolic; specialization only runs when lowering to MLIR needs a concrete gate sequence.
_Avoid_: monomorphization, template instantiation


**CircIr**: The Melior-free flat gate+wire IR produced by extracting a `quantum.circ` region and consumed by unitary optimization kernels (ZX rewriting, Clifford+T). Rebuilt back to verified `quantum.circ` ops. Owned by the `quon_circ` crate; `mlir_bridge` only adapts Melior ↔ CircIr. Distinct from the source-elaboration **SpecializedCircuit** and from **ZX-graph** (spider algebra on petgraph).
_Avoid_: extracted circuit, pass IR, gate list (unqualified)

**SpecializedCircuit**: The Melior-free first-order gate tree (enum: Compose / GateApp / Adjoint / Par / …) produced by parametric specialization in the frontend. Lower’s input after classical parameters are gone. Lives in the frontend crate; not CircIr.
_Avoid_: monomorphized Expr, elaborated AST (unqualified)

### Workspace crates

**quon_core**: The MLIR-free shared kernel of the Quon workspace — the single home for domain types that cross the frontend↔IR seam and must not pull in Melior/LLVM. Organized as named domain modules, not a junk drawer: `depth` (`DepthExpr`, the Circuit index algebra and the crate's **center of gravity**), `optimization` (depth-algebra invariant kernels — companions to `depth`), `gates` (the single gate-metadata source — see `Gate registry`), `qasm` (the emit-domain OpenQASM 3.0 syntax tree + total renderer, MLIR-free by necessity since emit is a pure string fold), `linearity` (the SSA use-count adapter — see `Linearity (SSA)`), and `metrics` (the snapshot/regression DTO; the collector itself lives in `mlir_bridge`). A type belongs here iff both `frontend` and `mlir_bridge` (or another non-MLIR crate) need to construct or inspect it *and* it has no Melior dependency. `qasm` and `metrics` stay here rather than splitting into a `quon_qasm` / metrics crate: neither split would remove a dependency, and the workspace keeps `quon_core` as the crossing-seam home. `CircIr` (owned by `quon_circ`) and `SpecializedCircuit` (owned by `frontend`) deliberately do *not* live here.
_Avoid_: core (unqualified), shared types crate, common (for this kernel)

### IR dialects

**quantum.circ**: The purely unitary MLIR dialect. All ops are unitary; no measurement. Every `!qubit` SSA value has exactly one use (enforced by a standalone region verifier pass). ZX-calculus rewriting and Clifford+T optimization run here.
_Avoid_: circ dialect, unitary dialect

**quantum.dynamic**: The dynamic circuit MLIR dialect. Adds measurement (`!qubit → !bit`), reset, feed-forward (`cf.cond_br` on `!bit`), and barrier ops. Unitary sub-circuits are embedded as `unitary_region` blocks containing only `quantum.circ` ops.
_Avoid_: dynamic dialect, measurement dialect

**quantum.physical**: Not a separate dialect — `quantum.dynamic` ops annotated with hardware attributes (`phys_qubit`, `native_gate`, `fidelity`). Routing and scheduling modify these attributes in place.
_Avoid_: physical dialect, hardware dialect

### Backend

**BackendTarget**: A hardware descriptor, discriminated by `TargetKind`. The `Fixed` kind (gate-model hardware) combines a connectivity graph, native gate set, noise model (per-gate fidelity, T1/T2 times, readout error), and capability flags (`supports_mid_circuit_meas`, `supports_feed_forward`). Other kinds (e.g. `NeutralAtomReconfigurable`) carry an independent field set with no forced overlap — only an `id` is shared across all kinds.
_Avoid_: target, device, backend

**TargetKind**: The discriminant on `BackendTarget` separating architecture families (gate-model `Fixed`, `NeutralAtomReconfigurable`, and future families) whose descriptors have genuinely different shapes. Each kind owns its own payload rather than sharing fields — deliberately, so adding a new architecture family never forces awkward unused fields onto existing kinds.
_Avoid_: architecture kind, target type

**Native gate**: A gate in the BackendTarget's supported gate set. Gates not in the native set must be decomposed before emission. Tracked via the `native_gate : BoolAttr` attribute on `quantum.circ.gate` ops.
_Avoid_: supported gate, hardware gate

**Gate registry**: The MLIR-free table in `quon_core::gates` that is the single source of truth for Quon/OpenQASM gate metadata — canonical id, surface aliases (`CX`/`CNOT`, `S†`/`S_dag`), arity, Clifford class, inverse, and OpenQASM spelling. Typecheck, backend `std_gates`, adjoint/inverse helpers, gate cancellation, and OpenQASM emit (`qasm::from_gate_info` → registry-backed `QasmGate::Std*`) all consume it; adding a fixture gate means appending one `GateInfo` row with `openqasm: Some(kw)` (see the module docs). Multi-angle `u2`/`u3` still use typed constructors. Adapters may still map id → Melior attributes or ZX nodes.
_Avoid_: gate table, STD_GATES, native-gate map

### Optimization

**T-count**: The number of non-Clifford `T` / `T†` gates in a circuit (after normalizing exact `Rz(k·π/4)` into discrete Clifford+T when applicable). The primary cost metric for fault-tolerant Clifford+T optimization; distinct from total gate count and from `DepthExpr`.
_Avoid_: T-gate count, non-Clifford count, magic-state count

**ZX-graph**: An auxiliary graph representation of a `quantum.circ` circuit used for non-local algebraic simplification. Nodes are Z- or X-spiders with phase angles; edges are wires or Hadamard boxes. Built on `petgraph::StableGraph`.
_Avoid_: ZX-diagram, spider graph

### QEC (source language)

**QecBlock**: A source-level linear quantum resource of type `QecBlock<F, d>` — one logical qubit encoded under code family `F` at distance `d`, optionally prepared in a logical Pauli basis at construction. Distinct from a bare `Qubit`/`QReg`; QEC programs opt into it explicitly rather than annotating ordinary qubits with error-correction properties.
_Avoid_: QEC qubit, encoded qubit, logical qubit (for the source type)

**QEC workload IR**: The MLIR-free data structure collected from QEC builtins inside a `Q` / `run { }` program (constructors, memory rounds, logical ops, measurements) — family, distance, logical ids, rounds, and operation order. Owned by the shared QEC layer and consumed by neutral-atom scheduling / experiment emit. Not a source-language type or a second monad; QEC programs stay in `Q<τ>` with `QecBlock` as the linear resource.
_Avoid_: QecWorkload (as a source type), QEC circuit, fault-tolerant circuit

**Code family**: A discrete type-level tag naming an error-correcting code kind used as the `F` parameter of `QecBlock`. v1 inhabitants are the closed builtin set `Repetition` and `Surface`; new families require a compiler change. Not a `Nat`; not interchangeable with distance.
_Avoid_: code type, QEC family enum (for the source concept)

**Kinded type parameter**: A type or `Nat` parameter annotated with a kind in a function or type declaration (e.g. `F: CodeFamily`, `d: Nat`). Distinct from today's Nat-only alias parameters; user functions may be generic over both `CodeFamily` and `Nat` kinds.
_Avoid_: type variable (unqualified), template parameter, monomorphization parameter

**Memory round**: One syndrome-extraction cycle on a `QecBlock` — entangle checks, measure ancillas, reset — returning the same block for further rounds or logical measurement. A QEC builtin in `Q`, not a bare physical `measure` on data atoms. Expands to a scheduled round with a barrier: NA planners may optimize inside the round, but must not reorder or compact across round boundaries.
_Avoid_: syndrome round (as the source name), stabilizer round (unqualified)

**Logical measurement**: A QEC builtin that consumes a `QecBlock` and returns a classical `Bit` for a chosen logical Pauli (v1: `measure_logical_x`, `measure_logical_z`). Distinct from measuring an individual physical atom in the schedule.
_Avoid_: logical readout, block measurement

**Physical error model**: Per-target calibrated (or assumed) physical error parameters for neutral-atom QEC reporting and experiment emit — Rydberg, measurement, reset, movement, transfer, and idle — distinct from gate `fidelity` fields used by non-QEC cost hooks. Absent when QEC error artifacts are requested is a hard compiler failure, not a defaulted guess.
_Avoid_: noise model (for the NA QEC fields), fidelity (for these parameters)

**QEC experiment artifact**: The paired compiler outputs for external QEC evaluation — versioned semantic experiment JSON plus a generated Stim circuit — both derived from the same QEC workload IR. The JSON carries Quon/QEC metadata and schedule references; the Stim circuit is what Sinter runs. Distinct from the compiler resource report; sampled Sinter results are not folded back into compiler artifacts.
_Avoid_: qec.json alone (as the full artifact), schedule JSON (as the experiment), fused QEC report

**Resource report (analytic)**: The compiler `ResourceReport` from `--emit-resource-report` — schedule metrics, QEC sizing metadata, analytic physical error-budget contributions (`rate × schedule count`, ADR-0017), and an analytic end-to-end fidelity estimate ([Enola] Eq. (1): `gate_fidelity_product` / `estimated_fidelity`, issue #305) computed from the target's `fidelity` model over the compiled schedule's actions plus per-atom idle decay. `error_budget` and the fidelity estimate are both analytic but computed from different target fields (`error_model` vs `fidelity`) and must not be conflated. Separate primary artifact from the Python/Sinter sampled CSV; the two are never fused into one undifferentiated claim summary, and neither is a threshold claim (ADR-0020). The #254 ablation harness may emit an optional labeled join CSV for comparisons while still writing a separate Sinter CSV and keeping report / dual-emit primaries (ADR-0020 amendment).
_Avoid_: fused QEC report, threshold estimate (for this artifact), Sinter CSV (as the resource report)

**QEC compiler-ablation benchmark**: The #254 workload × `--na-placer` / `--na-backend` / compaction grid with nested tiny Sinter samples (`python/quon_qec_benchmarks.py`). Nested Sinter is schedule-agnostic under ADR-0024 (noise from `error_model` proxies); analytic columns track ablations. Distinct experiment class from physical-NA #111 / RAP Table I — same schedule headline field *names* as methodology style only; no RAP numeric claims; no thresholds (ADR-0023).
_Avoid_: RAP Table I reproduction (for QEC rows), threshold sweep, physical-NA benchmark (#111)

**Lattice-surgery CX**: The v1 lowering of `logical_cx` between two same-distance surface-code blocks — a fixed L-shaped three-patch merge/split gadget (control|ancilla over target) with outcome-conditioned Pauli-frame byproducts. Not a general patch router; not a bare physical transversal CX.
_Avoid_: transversal CX (for this op), lattice surgery (unqualified as the whole compiler)

### Neutral-atom backend

**Logical qubit**: A backend/IR-level identifier for one encoded logical qubit after QEC lowering, used to track its expansion into atoms. Owned as `quon_qec::LogicalQubitId` and re-exported from `quon_na` (graph / qec). Distinct from the source-level `QecBlock` (the typed resource) and from bare `Qubit`.
_Avoid_: QEC qubit

**Atom**: A single physical site occupant in the neutral-atom architecture-aware schedule — the physical unit that a code block expands into. Exists only in the neutral-atom backend (`quon_na`), below the frontend's linear type system. The hybrid QEC interaction graph (`qec_schedule`) is atom-indexed via `AtomVertexId`, a newtype distinct from `LogicalQubitId` so placers/schedulers name physical atoms rather than casting (ADR-0029, #318).
_Avoid_: physical qubit

**Code block**: The backend expansion of a source `QecBlock` into a group of atoms jointly implementing one or more logical qubits under a given code family. Produced during neutral-atom/QEC lowering; not a source-language type.
_Avoid_: code patch, logical block, QecBlock (for the backend expansion)

**AOD movement**: The neutral-atom movement model where atoms move in row/column-coupled groups (as driven by acousto-optic deflectors), not freely and independently. The movement constraint that placement-routing scheduling in `quon_na` is built against — deliberately not a free-grid Manhattan-distance simplification, to stay faithful to the reproduced literature.
_Avoid_: grid movement, Manhattan movement

**Schedule IR**: The `quantum.na` MLIR form of a scheduled neutral-atom program, built via `ScheduleSpec` in `quon_na/src/dialect.rs` — the driver's canonical schedule artifact (ADR-0007, ADR-0011). The JSON schedule emit is a debug/visualization view of it, never a second source of truth.
_Avoid_: schedule JSON (as the IR), ScheduleSpec (for the concept)

### Sample corpus

**Sample**: A narrative, regenerable demo under `samples/` (catalogued in `catalog.yaml`) aimed at learners, researchers, or toolkit consumers — distinct from CI fixtures in `test/` and from compiler-oriented `examples/na_qec/`.
_Avoid_: fixture (for narrative demos), cookbook page (the website may deep-link samples)

**Sample catalog**: The machine-readable `samples/catalog.yaml` index of every sample (`id`, path, tags, difficulty, `quonc` args, artifacts, optional `ci: smoke|none`). Contribution requires a catalog row.
_Avoid_: samples README alone (as the index of record)

**ScheduleLayer**: The planners' in-memory working representation of one time slice of a neutral-atom schedule (`quon_na/src/schedule.rs`) — what extract / place / entangle / compact produce and validate. Planner-internal: converted to the schedule IR at the driver boundary by the single named `ScheduleLayer → ScheduleSpec` converter (ADR-0011); never serialized as a primary artifact.
_Avoid_: schedule IR (for this type), layer spec
