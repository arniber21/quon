# Swift documentation benchmark for Quon

**Scope.** This is an evidence-based comparison of Swift's public documentation
information architecture with Quon's public site. Statements labelled
**Observed** describe the cited Swift or checked-in Quon documentation; items
labelled **Proposal** are adaptations, not claims about Swift or completed
Quon work. It assesses documentation structure, learning, examples, reference,
diagnostics, and contribution/testing—not language or compiler behavior.

## Observed Swift practices

1. **A compact portal routes readers by intent.** Swift's documentation landing
   page separates the language book, API-design guidance, standard and core
   libraries, package manager, REPL/debugger, articles, and contributor-facing
   material (evolution, source, CI, compatibility, and compiler architecture).
   This is an explicit top-level distinction between learning, library/API use,
   tooling, platform articles, and project participation. [Swift documentation
   landing page](https://www.swift.org/documentation/)
2. **The language book has deliberate depth levels.** Swift identifies *The
   Swift Programming Language* as one book containing a guided tour, a
   comprehensive guide, and a formal reference. [TSPL overview](https://www.swift.org/documentation/tspl/)
   The book's first tour begins with a runnable one-line program, states that it
   is sufficient to start writing code, and defers detailed explanation to the
   rest of the book. Its source also embeds executable `swifttest` examples and
   asks bounded experiments, including intentionally removing a conversion to
   inspect the error. [A Swift Tour](https://raw.githubusercontent.com/swiftlang/swift-book/main/TSPL.docc/GuidedTour/GuidedTour.md)
3. **Reference and narrative are separate responsibilities.** The portal sends
   language readers to TSPL and points library users to Standard Library/Core
   Libraries separately, rather than treating a conceptual guide as the API
   index. [Swift documentation landing page](https://www.swift.org/documentation/)
4. **Contributor guidance is an operational path.** Swift's contribution guide
   specifies a reproducible bug report, asks for a small reproducer and
   environment details, recommends incremental changes, and requires tests for
   bugs and features. [Swift contributing guide](https://www.swift.org/contributing/)
   Its CI page describes pull-request testing before integration, status posted
   back to PRs, and failure details. [Swift continuous integration](https://www.swift.org/documentation/continuous-integration/)
   Language/API changes additionally have a public evolution process described
   as proposal, discussion, review, approval, and release tracking. [Swift
   Evolution](https://www.swift.org/swift-evolution/)

## Observed Quon architecture

Quon's Starlight sidebar already presents seven reader-facing regions:
Getting Started, Learning track, Why Quon, Language guide, Cookbook,
Architecture, Guides, and Reference. The home page independently routes to the
quickstart, language guide, architecture, and cookbook. See
[`website/astro.config.mjs`](../../website/astro.config.mjs) and
[`website/src/content/docs/index.mdx`](../../website/src/content/docs/index.mdx).

Its learning path is unusually strong: the six ordered lessons pair prose with
small editable samples, provide a build/run command, and hand off to the
language guide and cookbook. [`learn/index.mdx`](../../website/src/content/docs/learn/index.mdx)
The language introduction also states a prerequisite-based reading order and
ends in a next-page link. [`language/introduction.md`](../../website/src/content/docs/language/introduction.md)
The quickstart and second-program walkthrough use complete, end-to-end quantum
programs, generated artifacts, and reproducible Aer commands.
[`getting-started/quickstart.md`](../../website/src/content/docs/getting-started/quickstart.md)
[`getting-started/second-program.md`](../../website/src/content/docs/getting-started/second-program.md)

Quon has a distinct CLI and compiler-pipeline reference, but no separately
labelled source-language syntax/reference section in the sidebar; the language
guide carries both conceptual and reference-depth duties. The site does show a
representative compiler error in the language introduction, documents compiler
diagnostic stages/debug switches in the compiler reference, and documents LSP
compiler/lint diagnostics and quick fixes in the tooling guide.
[`reference/compiler.md`](../../website/src/content/docs/reference/compiler.md)
[`guides/tooling.md`](../../website/src/content/docs/guides/tooling.md)

The repository contributor guide specifies setup, daily checks, a PR workflow,
and test commands, but it does not provide a public docs-contribution route or
a compact diagnostic-reproducer template. [`CONTRIBUTING.md`](../../CONTRIBUTING.md)

## Gap matrix

| Area | Evidence-based comparison | Proposal |
| --- | --- | --- |
| Information architecture | Swift's portal names language, libraries, tools, articles, and project participation independently. Quon has a clear site navigation, but contributor and language-reference discovery are outside or blended into it. | Add a top-level **Contribute** entry and a **Language reference** entry; keep the current guide as explanatory narrative. |
| Progressive disclosure | Swift explicitly offers tour → guide → formal reference. Quon offers quickstart → six lessons → language guide/cookbook, a solid equivalent first half. | Make the handoff explicit as **Quickstart → Learning track → Language guide → Language reference**, with one “when to use this” sentence on each landing page. |
| Examples | Swift's tour uses tiny runnable snippets, expected output, and focused experiments; Quon uses realistic complete programs and reproducible simulator commands. | Preserve end-to-end examples; add small compile-checked fragments and expected diagnostic/output blocks for each language feature. |
| API/reference separation | Swift distinguishes language book and library documentation. Quon separates CLI/pipeline reference, but source-language concepts and lookup material share the language guide. | Publish a generated or maintained language-reference index for syntax, types, built-ins, gates, errors, and command options; link it from guide pages rather than duplicating prose. |
| Diagnostics | Swift's tour makes an error an intentional learning exercise. Quon shows one rich linearity error and documents LSP/compiler diagnostics. | Add a diagnostic catalog organized by error family: minimal invalid program, annotated output, cause, and smallest repair. Ensure examples are checked in CI. |
| Contribution/testing | Swift combines contributor instructions, small-reproducer expectations, mandatory tests, PR CI feedback, and a public process for language/API changes. Quon has contributor setup and CI commands. | Add a public docs-contribution/testing page: local preview, link/example checks, review ownership, a docs-change checklist, and a diagnostic/example reproducer template. |

## Proposed independently implementable documentation slices

These are backlog-sized documentation slices, **not tracker issues**. Each can
land independently without a compiler change.

1. **Docs orientation page.** Add a short “Choose your path” page/card set that
   names the intended reader and next step for Quickstart, Learning track,
   Language guide, Cookbook, Architecture, Reference, and Contribute.
2. **Language reference shell.** Add an index with stable headings for lexical
   syntax, declarations, types, `Circuit`, `Q`, gates, statements/expressions,
   errors, and grammar status. Initially link to existing guide pages; do not
   copy their explanations.
3. **Feature-fragment contract.** For each language-guide topic, add one
   minimal valid fragment with expected emitted/observed result and one invalid
   fragment with the expected diagnostic family. Wire these snippets to the
   existing documentation/example checking mechanism once selected.
4. **Diagnostic catalog.** Add category landing pages for parse, type/refinement,
   linearity, circuit/depth, target, and emission failures. Each entry must have
   a minimal reproducer, output, explanation, and repair—not only prose about
   the compiler stage.
5. **Reference boundary cleanup.** Move command-option lookup and stable
   compiler contracts toward Reference; retain motivation, trade-offs, and
   worked examples in Guides/Architecture. Add reciprocal links at the boundary.
6. **Public documentation contribution guide.** Add docs preview instructions,
   page taxonomy, style/example rules, review expectations, and a compact bug
   report/reproducer template. Link it from the website and root contributor
   guide.
7. **Docs quality gate documentation.** Document the exact checks that validate
   internal links, rendered code blocks, sample commands, and diagnostic
   snapshots; state which checks run locally and in CI. Add this only after the
   project chooses the authoritative commands.

## Source notes

All Swift evidence above is from first-party Swift.org pages or the official
`swiftlang/swift-book` repository. The cited Quon paths are checked-in public
site/repository documentation reviewed for this comparison.
