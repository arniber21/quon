---
title: Documentation style guide
description: How to write Quon documentation — lead summaries, terminology, examples, warnings and non-claims, links, and headings.
---

Quon's documentation aims to be *correct, concise, and checkable*. This guide
defines the conventions that keep it that way. It is the companion to
[Documenting Quon](/contribute/), which covers *where* content goes; this page
covers *how* to write it once you know where.

The governing principle is **clarity at the point of use**: a reader who stops
at the first sentence of a page or section should already understand what it is
for and whether they are in the right place. Everything below serves that
principle.

## Lead summaries

Every page and every section opens with a **lead summary**: one or two
sentences that state what the page or section is for and what the reader will
be able to do after reading it.

- The page summary lives in the frontmatter `description` **and** is echoed in
  the first prose paragraph. The frontmatter feeds search and sidebar
  tooltips; the prose paragraph orients a reader who skipped the tooltip.
- A section summary is the first sentence after the heading. Do not start a
  section with a code block or a sub-heading.
- State the purpose, not the structure. "This page explains how the typechecker
  proves depth bounds" is a purpose. "This page has four sections" is not.

```markdown
## Lead summaries

Every page and every section opens with a **lead summary**: one or two
sentences that state what the page or section is for and what the reader will
be able to do after reading it.
```

The block above is the lead summary of *this* section, written to its own rule.

### What goes in the `description`

The frontmatter `description` should be a single sentence (aim for under 160
characters) that stands alone in search results. It must not assume the page
title. "Set up Quon from source with the pinned compiler toolchain" is good;
"Setup" is not.

## Terminology

Quon has a fixed domain vocabulary. Use it.

- The canonical glossary is
  [`CONTEXT.md`](https://github.com/arniber21/quon/blob/main/CONTEXT.md). Every
  term a reader must learn — *qubit*, *register*, *linear context*, *Circuit
  type*, *Quantum Monad*, *Clifford class*, *depth bound*, *borrow block* — is
  defined there.
- Use terms **verbatim**. Do not coin synonyms. The unit of quantum ownership
  is a *qubit*, not a "quantum bit" or "qu-bit." The dynamic computation type
  is the *Quantum Monad*, not the "Q monad" or "the monad."
- When you introduce a term for the first time on a page, **bold** it and link
  to its definition or to the page that defines it. After the first
  introduction, use the bare term.
- Capitalize proper nouns consistently: *Quon* (the language and the project),
  *OpenQASM 3*, *MLIR*, *LLVM*, *Z3*, *Devbox*, *Graphite*, *Starlight*.
  Generic nouns are lower case: *the compiler*, *the typechecker*, *the
  pipeline*, *the frontend*.

### Terminology ownership

New terms do not get invented in documentation. If a concept needs a name:

1. Propose it in `CONTEXT.md` first (open an issue if you are not sure).
2. Once `CONTEXT.md` records the term, use it in docs.
3. Never let a docs page be the sole authority for a term — `CONTEXT.md` is the
   single source.

If you find a docs page using a term that is not in `CONTEXT.md`, treat it as a
bug: either add the term to `CONTEXT.md` or replace it with the existing one.

## Examples

Examples are the most-read part of any page. They must be **minimal, runnable,
and checked**.

- **Minimal.** An example shows one thing. Strip every gate, parameter, and
  line that is not load-bearing for the point being made. A Bell pair needs two
  gates and two lines; do not pad it with measurement harness unless
  measurement is the point.
- **Runnable.** Prefer examples that compile with the current `quonc`. If an
  example cannot compile (it illustrates an error, or a hypothetical), say so
  explicitly with a sentence above the block: "This program does not compile:"
  followed by the expected diagnostic.
- **Fenced and labeled.** Every code block has a language tag. Quon source uses
  `` ```kotlin `` (the Tree-sitter grammar and the site's syntax highlighter
  both expect this). Shell uses `` ```sh `` or `` ```bash ``. Output uses
  `` ```text `` or no tag only when it is genuinely ambiguous.

```kotlin
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}
```

- **Output follows input.** When you show a command, show its output in the
  immediately following block, trimmed to the relevant lines. Do not paraphrase
  output; paste it.
- **Keep examples in sync with samples.** When an example is also a checked-in
  sample, link to the sample file so there is one runnable copy. The cookbook
  in particular should reference `samples/` rather than maintain parallel
  source.

## Warnings and non-claims

Admonitions call out content that is not part of the main flow but that a
reader must not miss. Starlight supports `note`, `tip`, `caution`, `danger`,
and `important`. Use them sparingly; an admonition per paragraph teaches the
reader to ignore them all.

| Type | Use for |
| --- | --- |
| `note` | Helpful context that is not load-bearing. |
| `tip` | A better way to do what the page just described. |
| `caution` | Something that will surprise or cost the reader if ignored. |
| `danger` | Something that will lose data or break a build. |
| `important` | A constraint the rest of the page depends on. |

:::caution[Admonitions are not formatting]
Do not wrap ordinary paragraphs in an admonition to make them stand out. If
everything is emphasized, nothing is. An admonition must change what a careful
reader does.
:::

### Non-claims

Quon's value proposition rests on what the typechecker *proves*. Documentation
must never claim a guarantee the compiler does not provide. This is the
single most important correctness rule in this guide.

- Say "the typechecker proves every qubit is consumed exactly once" only when
  that is a theorem the typechecker discharges. If it is a runtime or
  best-effort check, say so.
- Prefer the verb that matches the mechanism: *proves* (typechecker),
  *checks* (linter, runtime assertion), *optimizes* (backend pass),
  *emits* (codegen). Do not blur them.
- When a feature is partial or target-dependent, state the limit in the same
  sentence as the capability. "The optimizer applies Clifford+T synthesis on
  Universal circuits; Clifford circuits use a stabilizer tableau instead" —
  both halves matter.
- If you are unsure whether a claim is a theorem, a check, or a heuristic, ask
  before writing it. A wrong claim about a proof is worse than no claim.

## Links

Links keep the documentation a single, navigable whole.

- **Internal links are relative and end with a trailing slash.** Use
  `/language/circuits/`, not `../language/circuits.md` and not
  `https://quon.arnabg.me/language/circuits/`. Starlight resolves the slug;
  the trailing slash avoids a redirect.
- **Link to the canonical source.** When a page depends on a fact owned
  elsewhere, link to that page rather than restating it. See
  [canonical-source rules](/contribute/#canonical-source-rules).
- **Link to source on GitHub for repo-internal files** that are not part of the
  site (`CONTEXT.md`, ADRs, `samples/`, `Justfile`). Use the
  `https://github.com/arniber21/quon/blob/main/...` form so the link works
  outside the site too.
- **Anchor every reference.** Prefer `[/contribute/#canonical-source-rules]`
  style over bare "see Documenting Quon." The reader should land on the exact
  section.
- **Do not link to a specific commit** unless the content is historical. Link
  to `main`; CI keeps `main` correct.

Starlight fails the production build on dead internal links, so `pnpm build` is
the link checker. Run it before every PR.

## Headings

- **One `H1` per page**, implicit in the frontmatter `title`. Do not repeat the
  title as a `#` heading in the body.
- **Use `H2` (`##`) for top-level sections**, `H3` (`###`) for subsections.
  Do not skip levels. A page that jumps from `##` to `####` is a page that has
  lost its structure.
- **Headings are sentence case.** "Lead summaries," not "Lead Summaries."
  Proper nouns keep their capitalization: "The Quantum Monad."
- **Headings describe content, not navigation.** "Canonical-source rules" is a
  heading; "Next steps" is a section that should usually be a lead summary of
  the page it links to.
- **End a page with a `## Next` section** that links to the logical next page,
  when one exists. The learning track and language guide are ordered; reference
  and cookbook pages may omit it.

## Voice and tone

- **Second person.** Address the reader as "you." The compiler and the
  typechecker are "it" or named directly.
- **Present tense.** "The typechecker proves linearity," not "the typechecker
  will prove linearity."
- **Declarative over imperative where possible.** "A circuit is a value with a
  type" teaches; "remember that a circuit is a value" nags.
- **Short sentences for claims, longer for reasoning.** State the fact; then
  explain why. Do not bury the fact in the explanation.
- **No marketing voice in reference or architecture pages.** Those pages exist
  to be correct, not persuasive. The landing page and "Why Quon" may persuade;
  the reference may not.

## Code conventions in prose

- **Backtick every identifier.** `quonc`, `Circuit<2, 2, 2, Clifford>`, `run`,
  `--emit-qasm`. Do not leave a symbol bare in prose.
- **Backtick commands as a unit:** `cargo run -p quonc -- --emit-qasm`, not
  `cargo` `run` `-p` `quonc`.
- **Use the project's terms for the project's artifacts.** The compiler emits
  *OpenQASM 3* and *neutral-atom schedules*; it does not emit "code" or
  "output" when the specific artifact matters.

## What not to do

- Do not copy a flag table from `reference/quonc` into a guide. Link to it.
- Do not introduce a term that is not in `CONTEXT.md`.
- Do not claim a proof the typechecker does not discharge.
- Do not start a section with a code block.
- Do not wrap ordinary prose in an admonition.
- Do not use a bare URL where a labeled link works.
- Do not leave a `TODO`, `FIXME`, or placeholder in committed documentation.

## Next

This is the style reference; return to
[Documenting Quon](/contribute/) for where content goes and how to ship it.
