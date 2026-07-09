# Hardware targets are the primary backend surface; OpenQASM is an intermediary emit form

Quon's identity is a **multi-architecture hardware compiler**: `BackendTarget` /
`TargetKind` (ADR-0009) select a real device family (fixed gate-model,
neutral-atom reconfigurable, …), and the pipeline lowers toward that family's
hardware IR and schedule. **OpenQASM 3.0 is a convenient intermediary** for
fixed gate-model targets — useful for Qiskit Aer verification, tooling
interop, and all-to-all smoke tests — not the compiler's primary product or
architectural center of gravity.

This supersedes earlier framing (README / SPEC / website copy) that described
Quon as “an MLIR compiler that emits OpenQASM for Aer or hardware.” Emission
to OpenQASM remains supported and tested; it is one emit form among others
(`--emit-qasm`), not the definition of success for the project.

## Considered Options

**Keep OpenQASM-as-primary narrative.** Rejected: it understates the
neutral-atom path (`quon_na`, `quantum.na`, ADR-0007/0008), makes
`--emit-resource-report` / schedule IR look like side features, and fights
ADR-0009's bet on unified multi-architecture infrastructure.

**Drop OpenQASM entirely.** Rejected: Aer verification and the 8 reference
algorithms remain essential correctness anchors for the fixed path; OpenQASM
is the right interchange for that.

## Consequences

- Docs, CLI help, and website lead with **target selection** (`--target`) and
  architecture-specific emit forms (`--emit-na-schedule`, future
  `--emit-na-mlir` / #167, resource reports). `--emit-qasm` is documented as
  the fixed-target intermediary / verification path.
- Default `generic_openqasm` remains a built-in **fixed** target for
  convenience when no descriptor is supplied — still a hardware-shaped
  `TargetKind::Fixed`, not a claim that OpenQASM is the only backend.
- New architecture families add hardware IR / schedule / report surfaces
  first; an OpenQASM (or other interchange) emit is optional per family.
- Issue #167 tracks production `ScheduleLayer` → `quantum.na` MLIR lowering so
  the NA path has a first-class hardware IR, not only JSON.
