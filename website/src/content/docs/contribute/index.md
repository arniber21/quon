---
title: Documenting Quon
description: One path for adding or revising learning material, reference entries, recipes, architecture notes, and diagnostics — page taxonomy, canonical sources, preview, and review.
---

Quon's documentation is the first thing a new user, a contributor, or an
integrator meets. This page is the single entry point for anyone writing or
revising it. It defines **what kind of content goes where**, **which source is
canonical**, **who owns each section**, how to **preview and validate** a change
locally, and what **review** expects. The companion
[style guide](/contribute/style-guide/) defines how to write the prose, code,
and admonitions once you know where they belong.

The guiding principle, adapted from Swift, is *clarity at the point of use*:
every public symbol, command, artifact, and concept should carry a concise
purpose statement before any deeper detail. A reader who stops at the first
paragraph of a page should already know what the page is for.

## Page taxonomy

The site is an Astro Starlight project rooted in
`website/src/content/docs/`. Each top-level directory is a *page type* with a
distinct audience and a distinct contract. Pick the type from the table below;
do not invent a new directory without discussing it in an issue first.

| Page type | Directory | Audience | One-line contract |
| --- | --- | --- | --- |
| Getting started | `getting-started/` | First-run users | Get from clone to a compiled, sampled program. |
| Learning track | `learn/` | Newcomers learning concepts | Teach one concept per page, in order, assuming nothing. |
| Language guide | `language/` | Programmers writing `.qn` | Specify the language precisely, with checked examples. |
| Cookbook | `cookbook/` | Practitioners wanting runnable recipes | One complete, compilable program per page, with output. |
| Architecture | `architecture/` | Contributors reading internals | Explain how the compiler is built and why. |
| Guides | `guides/` | Operators and integrators | How to drive a tool, backend, or workflow end to end. |
| Reference | `reference/` | Daily reference readers | Authoritative listing of CLI flags and pipeline stages. |
| Why Quon | `why-quon/` | Design-rationale readers | The "why" behind design decisions. |

When a topic could live in two places, the table above resolves the ambiguity:
the **most specific** matching type wins. "How the typechecker proves depth
bounds" is Architecture, not Language guide, even though depth is a language
concept — the *how* is internal. "What a depth bound is and how to write one"
is Language guide.

## Canonical-source rules

Each fact about Quon has exactly one canonical home. Other pages may
**reference** it with a link, but must not **restate** it in a way that can
drift.

| Fact | Canonical source | Notes |
| --- | --- | --- |
| `quonc` flags and behavior | [`reference/quonc`](/reference/quonc/) | Guides link here; they do not re-document flags. |
| Compiler pipeline stages | [`reference/compiler`](/reference/compiler/) | The stable shape. `architecture/compiler-internals` goes deeper but defers to this page for the outline. |
| Domain terminology | [`CONTEXT.md`](https://github.com/arniber21/quon/blob/main/CONTEXT.md) | The repo glossary. Pages use these terms verbatim. |
| Architectural decisions | [`docs/adr/`](https://github.com/arniber21/quon/blob/main/docs/adr/) | ADRs are immutable records; docs summarize and link. |
| Sample programs | [`samples/`](https://github.com/arniber21/quon/blob/main/samples/) + `samples/catalog.yaml` | Cookbook recipes that ship a sample must point at the sample, not inline a divergent copy. |
| Formatter style | [`docs/quonfmt-style.md`](https://github.com/arniber21/quon/blob/main/docs/quonfmt-style.md) | Referenced from `guides/tooling`; not duplicated. |
| Validation commands | [`docs/agents/validation.md`](https://github.com/arniber21/quon/blob/main/docs/agents/validation.md) | The Justfile is the source of truth; this page adapts it. |

:::note[One source, many links]
If you find yourself restating a flag, a stage, or a term, stop and link
instead. Drift between two statements of the same fact is the most common
documentation bug — and the hardest to notice in review.
:::

### Where new content belongs

- A **new CLI flag** lands in `reference/quonc` first. If it changes a workflow,
  add a cross-link from the relevant guide; do not document the flag in the
  guide.
- A **new language feature** lands in the language guide at the right point in
  the reading order, with a checked example. Add it to the learning track only
  if it is a concept a newcomer needs.
- A **new sample** goes under `samples/` with a catalog row (see
  [`samples/CONTRIBUTING.md`](https://github.com/arniber21/quon/blob/main/samples/CONTRIBUTING.md)),
  then a cookbook recipe can reference it.
- A **new architectural decision** is recorded as an ADR; the architecture page
  summarizes and links once the ADR is merged.
- A **new backend or target** gets a guide under `guides/` and any new
  `--emit-*` flags documented in `reference/quonc`.

## Ownership

Every page type has an owning concern. Ownership here means "the group that
reviews changes," not a single gatekeeper.

| Page type | Owned by | Review signal |
| --- | --- | --- |
| Language guide | Language/compiler maintainers | Examples must typecheck. |
| Reference | Compiler maintainers | Must match the current `quonc` build. |
| Architecture | Compiler maintainers | Must match ADRs and the current pipeline. |
| Cookbook | Sample-corpus maintainers | Must match a `samples/` entry. |
| Guides | Tooling/integration maintainers | Commands must run against a source checkout. |
| Learning track | Education maintainers | Must stay in reading order. |
| Getting started | Release maintainers | Must match the current install path. |
| Why Quon | Project lead | Design rationale is rarely churned. |

If you are unsure who owns a page, open an issue and ask before writing a large
change. Small fixes — typos, broken links, clarified sentences — are welcome
from anyone and need no prior coordination.

## Local preview and validation

The docs site is a Starlight project under `website/`. Preview locally before
opening a PR.

### Preview

```bash
cd website
pnpm install      # first time only
pnpm dev          # http://localhost:4321
```

`pnpm dev` hot-reloads on save. Leave it running while you edit.

### Build parity with CI

CI builds the site on every push and PR (the `docs` job in `ci.yml`). Match it
locally with the workspace recipe:

```bash
just ci-website    # Starlight pnpm build under website/
```

A clean `pnpm build` is the gate: if the production build fails, CI fails.
`pnpm dev` tolerates some errors that the build will not.

### Validation checklist before a PR

1. `pnpm build` (or `just ci-website`) succeeds with no broken links.
2. Every code block that claims to be Quon compiles, or is explicitly marked as
   illustrative. See the [style guide](/contribute/style-guide/#examples).
3. Every cross-link resolves to an existing page (Starlight fails the build on
   dead internal links).
4. New terms appear in `CONTEXT.md`, or you have opened an issue proposing the
   addition.
5. Restated facts have been replaced with links to their canonical source.

### Asserting validation docs

The `just ci-docs-assert` recipe runs `scripts/assert-validation-docs.sh`,
which keeps the validation matrix in
[`docs/agents/validation.md`](https://github.com/arniber21/quon/blob/main/docs/agents/validation.md)
in sync with the Justfile and CI. Documentation that cites a command should run
this check locally if the change touches validation claims; CI runs it on every
PR.

## Review expectations

A documentation PR is reviewed against five questions:

1. **Right place.** Does the content live in the page type its contract
   describes, or has it drifted into another type's territory?
2. **Canonical.** Does it link to the canonical source for each fact it
   touches, rather than restating it?
3. **Checked.** Do the examples compile and the commands run against a current
   source checkout?
4. **Clear at the point of use.** Does the page (and each section) open with a
   concise purpose statement a reader can act on?
5. **Consistent voice.** Does it follow the [style guide](/contribute/style-guide/)?

Reviewers are not copy editors. Bring prose that already follows the style
guide; review is for correctness, placement, and completeness.

## Reporting a documentation bug

When the docs are wrong — a flag has changed, an example no longer compiles, a
link is dead, a statement contradicts the compiler — file a documentation bug
using the reproducer template. A good report names the **command**, the
**target**, a **source snippet**, the **observed artifact or diagnostic**, the
**expected behavior**, and the **environment**, so a maintainer can reproduce
in one pass.

Use the
[documentation bug issue form](https://github.com/arniber21/quon/issues/new?template=documentation_bug.yml)
on the issue tracker. The form mirrors the structure below; copy it into a
free-form issue if the form is unavailable.

<details>
<summary>Documentation bug reproducer (copy-paste)</summary>

````markdown
**Command**

`quonc ... --emit-qasm`

**Target**

`generic_openqasm` (or the `--target` JSON path, or "n/a")

**Source snippet**

```qn
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}
```

**Observed artifact / diagnostic**

Paste the emitted output, the diagnostic, or the docs sentence that is wrong.

**Expected behavior**

What the docs should say, or what the command should produce.

**Environment**

- `quonc --version`:
- OS / toolchain (or `devbox shell`):
- Page URL or file path:
````

</details>

## Next

→ [Documentation style guide](/contribute/style-guide/)
