---
title: Visualizing neutral-atom schedules with MQT NAViz
description: Emit .naviz + .namachine from quonc and render the atom shuttling animation in the MQT NAViz visualizer.
---

import ising from '../../../../../test/na/ising.qn?raw';

Quon's neutral-atom compiler produces a `ScheduleLayer` sequence (moves,
transfers, entangling stages, single-qubit gates, measure/reset). The
`--emit-naviz` flag serializes that schedule plus the loaded
`NeutralAtomTarget` into the two files [MQT NAViz](https://github.com/munich-quantum-toolkit/naviz)
expects: a `.naviz` instruction file and a sibling `.namachine` machine
description. Drop them into the NAViz GUI (or the Python `mqt.naviz` package)
to scrub through the atom shuttling animation.

## Emit the pair

A zoned neutral-atom compile writes both files from one flag. The path you give
is the `.naviz` file; the `.namachine` is written to the same stem beside it.

```bash
devbox run -- cargo run -p quonc -- test/na/ising.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-naviz /tmp/ising.naviz
# writes /tmp/ising.naviz and /tmp/ising.namachine
```

`--emit-naviz` requires a filesystem path (not `-`): it always writes two
sibling files, so stdout dual-emit is not supported. It also requires a
`neutral_atom_reconfigurable` target, like every other `--emit-na-*` flag.

## What is emitted

- **`.naviz`** — a `#target` directive (the `.namachine` file-name stem), one
  `atom (x, y) <id>` setup line per initially-trapped atom, then one
  `@+ [ … ]` instruction group per schedule layer. A layer's parallel actions
  start together; NAViz recomputes each instruction's duration from the machine
  config and sequences the groups back-to-back in schedule order. `move`,
  `load`/`store` (SLM↔AOD transfers), `cz` (entangling stages), and `rz`/`ry`
  (single-qubit gates) become native NAViz opcodes. `Measure`, `Reset`,
  `Reuse`, and `Wait` have no NAViz opcode and are written as comments so the
  qubit lifecycle stays visible in the file. `H`/`U3` are approximated as
  single-axis rotations purely for rendering.
- **`.namachine`** — the machine: `movement { max_speed }` (AOD shuttle speed),
  `time { load, store, ry, rz, cz, unit }` (from the target's timing + transfer
  durations), `distance { interaction, unit }` (the Rydberg range), one
  `zone <id> { from, to }` rectangle per target zone, and one
  `trap <id> { position }` per occupied SLM site from the compiled layout.

## Open in MQT NAViz

Build and run the GUI from a checkout of the NAViz repo, then load the
`.naviz` file (NAViz discovers the sibling `.namachine` by stem, or you can
select it from the *Machine* menu):

```bash
git clone https://github.com/munich-quantum-toolkit/naviz
cd naviz && cargo run -p naviz-gui
# File → Open /tmp/ising.naviz
```

Or render a video headlessly with the Python package:

```python
from naviz import export_video, Repository

machine = open("/tmp/ising.namachine").read()
instructions = open("/tmp/ising.naviz").read()
export_video(instructions, "ising.mp4", (1920, 1080), 60, machine, Repository.styles().get("tum"))
```

:::note[Manual verification is a follow-up]
Automated tests pin the emitted text with golden snapshots
(`quon_na` unit tests) and check the CLI wiring end-to-end
(`quonc --test naviz_emit`), but they do **not** render inside NAViz.
Manually confirming that NAViz renders a zoned demo (e.g. a GHZ-8 schedule)
without parse errors is the remaining verification step tracked in issue #303.
:::
