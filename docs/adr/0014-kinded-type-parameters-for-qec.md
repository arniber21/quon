# Kinded type parameters for QEC (CodeFamily + Nat)

Quon previously only parameterized surface types by `Nat` (plus inferred Clifford class on `Circuit`). Epic #245 needs `QecBlock<F, d>` with a discrete code-family parameter, not just distance. We chose full kinded polymorphism — user functions may declare `F: CodeFamily` and `d: Nat` — over nullary-tag + prelude-only generics or separate `RepetitionBlock`/`SurfaceBlock` types, so library helpers and user code share one generic API and later families do not require a second language change.

v1 `CodeFamily` inhabitants are a closed builtin set (`Repetition`, `Surface`). User code may be generic over `F`, but cannot declare new families; adding a family is a compiler change. Backend sizing-only variants (qLDPC-like, abstract `[[n,k,d]]`) are not source tags in this tranche.

QEC programs use the existing `Q` monad with `QecBlock` as a linear resource (no source `QecWorkload` type). v1 builtins: `repetition_code` / `surface_code` (Z-basis init), `surface_code_x` (X-basis init; no `repetition_code_x` unless bit-flip X-memory is later justified), `memory_round`, `measure_logical_z` / `measure_logical_x`, and `logical_cx` (surface-only). No keyword arguments — separate constructors instead. Logical X/Z frame updates are compiler-internal workload IR operations (byproducts), not user-facing source builtins.

A single entrypoint `Q` program may use QEC builtins or bare `Qubit`/`QReg` ops, not both — mixed encoded/unencoded schedules are out of scope for this tranche and should be a frontend/lowering diagnostic.

Surface syntax: kinded angle-bracket parameters on functions and type aliases, e.g. `fn memory_rounds<F: CodeFamily, d: Nat>(b: QecBlock<F, d>, ...)`. Existing Nat-only aliases (`type Oracle<n> = ...`) remain valid with `n` defaulting to kind `Nat`. `CodeFamily` parameters are type-level only (no runtime value). Emit paths require fully specialized `F` and `d` at the entrypoint.

Source tags `Repetition` / `Surface` map at the QEC lowering boundary onto existing backend variants `RepetitionCodeToy` / `SurfaceCodeLike`. Wire/report strings use `"repetition"` / `"surface"`; the Toy/Like caveat stays in architecture docs and backend type names, not in Quon source.
