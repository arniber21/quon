# Pass introspection

Reading a pass dump: `--list-passes` for the stage list, `--dump-ir` for the
MLIR snapshot at each checkpoint. Useful whenever a program's emitted
metrics or QASM look wrong and you need to see *where* in the pipeline the
shape changed.

## Status

Owned by pack [#188](https://github.com/arniber21/quon/issues/188). Catalog
id: `workflows/pass-introspection`.

## List the pass stages

```bash
export QUONC=$PWD/target/debug/quonc   # cargo build -p quonc first
$QUONC --list-passes
```

```text
quonc pass stages
─────────────────
Shared front-end
  1. lower            Quon → quantum.circ
  2. circ fixpoint    gate_cancellation, rotation_merging,
                      compiler_uncomputation, zx_simplification
                      (clifford_t_opt reserved for #96 — real Clifford+T)
  3. monadic_lowering quantum.circ → quantum.dynamic
  4. dynamic          measurement_deferral, classical_region_fusion

Fixed (OpenQASM) path
  5. native_gate_decomp
  6. sabre_routing
  7. native_gate_decomp (post-SWAP)
  8. depth_scheduling
  9. emit OpenQASM 3.0

Neutral-atom path
  5. extract_interaction_graph
  ...
```

`--list-passes` needs no source file — it just describes the pipeline
`quonc` will run for whichever target family you select.

## Dump the IR at every checkpoint

`pass_introspection.qn` is `H |> H` on a single qubit — the identity, but
written so the *optimizer has to prove that*, not so it's trivially absent
from the source. `--dump-ir` prints the MLIR module at each of the compiler's
five checkpoints (lowering, circuit, monadic, dynamic, physical) to standard
error:

```bash
$QUONC samples/workflows/pass_introspection/pass_introspection.qn --dump-ir
```

The interesting diff is the first two sections. `--- after lowering ---`
still has two `H` gate ops:

```text
%0 = "quantum.circ.gate"(%arg0) {..., gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
%1 = "quantum.circ.gate"(%0) {..., gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
"quantum.circ.return"(%1) : (!quantum.qubit) -> ()
```

`--- after circ passes ---` — stage 2, `circ fixpoint` from `--list-passes`
above — has cancelled both `H`s away entirely, down to `depth = "0"`:

```text
"quantum.circ.return"(%arg0) : (!quantum.qubit) -> ()
```

That's `gate_cancellation` (one of the four passes `circ fixpoint` runs to a
fixed point) proving `H |> H = I` structurally, not just numerically — the
same mechanism `test/verify/qft.py`'s docstring credits for collapsing an
entire `qft |> adjoint(qft)` round trip to nothing. The remaining three
checkpoints (`monadic lowering`, `dynamic passes`, `physical passes`) carry
the now-empty region forward unchanged, since there's nothing left to
optimize.

## Debugging with `--verify-linear`

`--dump-ir` pairs well with `--verify-linear`, which runs the linearity
verifier on circuit IR and again after lowering to dynamic IR — useful when
a pass dump looks structurally fine but you suspect a resource is being
threaded incorrectly:

```bash
$QUONC samples/workflows/pass_introspection/pass_introspection.qn --dump-ir --verify-linear
```

## See also

- [`quonc` CLI reference — Debug options](../../../website/src/content/docs/reference/quonc.md#debug-options) —
  `--dump-ir`, `--list-passes`, `--verify-linear`.
- [Compiler pipeline reference](../../../website/src/content/docs/reference/compiler.md) —
  the full pass architecture behind these checkpoints.
- [`workflows/routing-sensitivity`](../routing_sensitivity/README.md) — reads
  `--metrics`/`--metrics-json` output instead of raw IR to compare two
  compiler configs.
