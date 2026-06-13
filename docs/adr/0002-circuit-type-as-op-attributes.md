# Encode Circuit<n,m,d,C> indices as op attributes, not as an MLIR parameterized type

The Quon source type `Circuit<n, m, d, C>` carries four indices. In the `quantum.circ` dialect, these are stored as attributes on the `quantum.circ.func` op (`in_qubits : I64Attr`, `out_qubits : I64Attr`, `depth : DepthExprAttr`, `clifford : BoolAttr`) rather than as parameters on an MLIR parameterized type.

The MLIR value type for circuit values is the unparameterized `!quantum.circ`; all semantic content lives in attributes.

## Considered Options

**Parameterized MLIR type `!quantum.circ<n, m, d, C>`** — would encode the indices in the type system, making composition type-checking a matter of MLIR type unification. Rejected because Melior's support for custom parameterized types is thin, and the depth index `d` is a symbolic `DepthExpr` (not a plain integer), which has no natural representation as an MLIR type parameter without a custom type parser and printer.

## Consequences

Composition type-checking at the IR level (verifying `out_qubits` of the left circuit matches `in_qubits` of the right) happens in Rust verifier callbacks, not in MLIR's type unifier. The `depth : DepthExprAttr` attribute carries a serialized `DepthExpr` S-expression, which optimization passes reconstruct into the `DepthExpr` Rust enum when they need to combine or check depth bounds.
