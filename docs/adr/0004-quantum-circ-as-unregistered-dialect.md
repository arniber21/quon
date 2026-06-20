# Register `quantum.circ` as an MLIR unregistered dialect, not via `#[melior::dialect]`

The `quantum.circ` dialect is registered by enabling **unregistered dialects** on the
Melior `Context` (`mlirContextSetAllowUnregisteredDialects`). Its ops live in MLIR's
generic operation form (`"quantum.circ.gate"(%q) {…} : (!quantum.qubit) -> !quantum.qubit`),
its qubit/circuit values use opaque dialect types (`!quantum.qubit`, `!quantum.circ`),
and its op verifiers run as explicit Rust callbacks (`quantum_circ::verify`) invoked by the
op builders — not as C++ verification hooks. The linearity invariant runs as a separate
external pass.

ADR-0001 named Melior's `#[melior::dialect]` proc-macro as the registration mechanism. In
Melior 0.27 that macro generates *typed Rust wrappers* for a dialect whose ops and types are
already defined in C++/TableGen and loaded into the context; it does not introduce a new
dialect from pure Rust. ADR-0001 anticipated this and recorded the fallback: "dropping to
raw C API calls for dialect registration only while keeping Melior for everything else." This
ADR takes that fallback.

## Considered Options

**Vendor a C++/TableGen dialect** built into a shared library and loaded via the C API. This
is the canonical way to get real registered ops, custom types, and native verifier hooks.
Rejected for now: it adds a C++/CMake build to an otherwise pure-Rust workspace and a TableGen
toolchain to CI, for ops whose semantics we already enforce in Rust. It remains the upgrade
path if we need MLIR-native verification, custom assembly formats, or type parameterization.

**IRDL dynamic dialect registration** via the C API. Rejected: Melior 0.27 does not expose
IRDL, and it would still not give us Rust verifier callbacks.

## Consequences

- Ops print and parse in the generic form. Round-trips are exact (verified by a FileCheck
  test and a Rust integration test), but the IR is more verbose than a dialect with a custom
  assembly format.
- `Operation::verify()` (MLIR's built-in verifier) does **not** run our op invariants, because
  unregistered ops carry no verifier. We compensate by running `quantum_circ::verify` inside
  every op builder, so a malformed op cannot be constructed through the public API, and by
  exposing `verify` for callers that build ops by hand.
- Diagnostic *emission* is not wrapped by Melior, so all error reporting flows through the
  `diagnostics` module, which isolates the single `unsafe` `mlirEmitError` boundary behind a
  `Result`/Writer-style abstraction.
- Custom types are opaque strings (`!quantum.qubit`, `!quantum.circ`); type checks compare the
  printed form rather than a registered `TypeID`.
