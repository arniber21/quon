# Use Melior for MLIR C API bindings

All MLIR integration goes through [Melior](https://github.com/raviqqe/melior) rather than raw `mlir-sys` bindings or hand-written `extern "C"` declarations. Melior wraps the MLIR C API with safe Rust abstractions for the pass manager, module construction, and IR walking. For custom dialect registration (`quantum.circ`, `quantum.dynamic`), we use Melior's `#[melior::dialect]` proc-macro system.

## Considered Options

**Raw `mlir-sys` bindings** — closer to the spec's original intent; more control. Rejected because writing and maintaining safe wrappers for the full MLIR C API surface is weeks of work that doesn't advance the compiler's core goals.

**Hand-written `extern "C"`** — maximum control, zero dependencies. Rejected for the same reason; the MLIR C API is large and gaps in coverage would surface repeatedly.

## Consequences

Melior's custom dialect proc-macro support is experimental. We should expect to patch or work around gaps in `#[melior::dialect]` as we implement `quantum.circ` and `quantum.dynamic`. If Melior's dialect macros prove unworkable, the fallback is dropping to raw C API calls for dialect registration only while keeping Melior for everything else — Melior and the raw C API coexist since they wrap the same shared library.
