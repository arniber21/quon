# Quon circuit stdlib

Experimental library of `.qn` circuit modules (issue #195). These are ordinary
Quon sources. They are not compiler intrinsics and they are not a module system.

## Decision

Quon has no `import`. The lexer keywords do not include `import`, `include`, or
`mod`. `parse_program` takes one string, and `quonc` compiles one entry file.
Multi-file language analysis is tracked separately (issue #177) and is not this
change.

**Interim strategy:** `quonc --include PATH` (repeatable) reads those files in
order, then the entry source, and concatenates them into one program.

- One flat top-level scope. A name defined in an include is visible to every
  later file, including the entry.
- Duplicate top-level `fn` or `type` names are an error when any `--include` is
  set. Listing the same path twice is an error.
- Diagnostics are attributed to the file whose byte range contains the span.
- `--include` does not apply to OpenQASM input (`--from-qasm` or a `.qasm` path).
- Includes are not nested. A file cannot itself request another file; the
  command line lists every input.
- `--watch` also watches each included path.

This is reversible. A future `import` with namespaces would replace the flag.
Samples that pass `--include stdlib/...` are the call sites to update.

What this is not: separate compilation, visibility, package versions inside the
compiler, or a second copy of the prelude. Gates, `identity`, `adjoint`, and
the other SPEC §5 intrinsics stay in the compiler. This directory is user-level
circuits factored so samples do not each paste QFT or a Trotter step.

## Stability

**Experimental.** Names, depth bounds, and which functions exist may change
with no deprecation window. Do not treat these signatures as a stable API.

| Module | What it provides | Samples |
| --- | --- | --- |
| `stdlib/qft.qn` | Parametric `qft`, `qft_roundtrip` | `samples/algorithms/stdlib_qft_roundtrip.qn`, `stdlib_qft_n2.qn` |
| `stdlib/oracles.qn` | Fixed BV, Deutsch–Jozsa, and Grover phase oracles | `samples/algorithms/stdlib_bernstein_vazirani.qn`, `stdlib_grover_mark.qn` |
| `stdlib/qaoa.qn` | Unweighted `cost_layer`, `mixer_layer`, `qaoa_layer` | `samples/applications/stdlib_qaoa_k3.qn`, `stdlib_qaoa_n2.qn` |
| `stdlib/ising.qn` | Open-chain Trotter `zz_layer`, `x_layer`, `ising_evolve` | `samples/applications/stdlib_ising_chain.qn`, `stdlib_ising_one_step.qn` |
| `stdlib/phase_estimation.qn` | 1-bit QPE and `phase_kickback_cz` | `samples/algorithms/stdlib_qpe_1bit.qn`, `stdlib_phase_kickback.qn` |

## Compile

```bash
quonc --include stdlib/qft.qn --emit-qasm samples/algorithms/stdlib_qft_roundtrip.qn
```

Repeat `--include` to prepend several modules. Order is the order of the flags,
then the entry file.

## Limitations baked into the modules

- Oracle functions are fixed circuits. A secret string is not a parameter.
- QAOA here is unweighted (`pairs(n)`), one layer. Weighted `Matrix` costs and
  a circuit-valued `fold` over a parameter list are not elaborated.
- Ising evolution uses `repeat`, not a circuit-valued `fold`. The chain is
  open; a ring bond is written by the caller.
- Phase estimation is t=1. Multi-bit inverse QFT is not provided: `qft` uses a
  controlled-Rz convention that does not match standard QPE kickback.
