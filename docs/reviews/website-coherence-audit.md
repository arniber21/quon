# Quon public documentation coherence audit

**Scope.** This audit compares the deployed public site with the checked-in
Starlight configuration and public docs source. It evaluates reader routing,
navigation, ordering, duplication, terminology, visual information density, and
section transitions. **Observed** statements are backed by the cited deployed
route or source path. **Recommendation** statements are proposed remediation,
not claims about current behavior. Live observations were made on 2026-08-04 at
1440 × 1000 CSS pixels.

This report intentionally does not repeat the language-reference and public
contribution-route gap analysis already documented in the [Swift documentation
benchmark](./swift-documentation-benchmark.md#gap-matrix). The findings below
cover the deployed/source coherence and reader-journey defects that must be
resolved before those larger information-architecture improvements can work
reliably.

## Evidence snapshot

| Required area | Deployed observation | Checked-in public source |
| --- | --- | --- |
| Home | [`/`](https://quon.arnabg.me/) returned 200. Its primary action is **Run the quickstart**; the other audience cards are language, architecture, and cookbook. | [`index.mdx`](../../website/src/content/docs/index.mdx#L5-L20) defines those three hero actions; the cards repeat the four routes at [lines 52–75](../../website/src/content/docs/index.mdx#L52-L75). |
| Getting Started | [`/getting-started/quickstart/`](https://quon.arnabg.me/getting-started/quickstart/) returned 200. It is a long end-to-end Bell/Aer/neutral-atom walkthrough. | [`quickstart.md`](../../website/src/content/docs/getting-started/quickstart.md#L6-L10) says it assumes the installation guide; its four stages run through [line 171](../../website/src/content/docs/getting-started/quickstart.md#L11-L171). |
| Learning | [`/learn/`](https://quon.arnabg.me/learn/) returned **404**. The rendered desktop sidebar has no Learning track group. | [`astro.config.mjs`](../../website/astro.config.mjs#L22-L33) declares a Learning track and seven lesson routes; [`learn/index.mdx`](../../website/src/content/docs/learn/index.mdx#L8-L80) declares it the beginner on-ramp. |
| Language | [`/language/introduction/`](https://quon.arnabg.me/language/introduction/) returned 200. The route is a dense conceptual introduction, not a section index. | [`introduction.md`](../../website/src/content/docs/language/introduction.md#L233-L254) says to read ordered pages and ends only with the next language page. |
| Cookbook | [`/cookbook/`](https://quon.arnabg.me/cookbook/) returned 200. It asks readers to read recipes in order and assumes prior circuit/linearity/`run` knowledge. | [`cookbook/index.mdx`](../../website/src/content/docs/cookbook/index.mdx#L69-L75) specifies progression; [lines 179–188](../../website/src/content/docs/cookbook/index.mdx#L179-L188) specify prerequisites and its first recipe. |
| Guides | [`/guides/tooling/`](https://quon.arnabg.me/guides/tooling/) returned 200; [`/guides/`](https://quon.arnabg.me/guides/) returned **404**. | The sidebar lists individual Guides but no section landing page in [`astro.config.mjs`](../../website/astro.config.mjs#L70-L80). |
| Architecture | [`/architecture/compiler-internals/`](https://quon.arnabg.me/architecture/compiler-internals/) returned 200; [`/architecture/`](https://quon.arnabg.me/architecture/) returned **404**. | The page says its audience is people reading source or extending a pass and then presents the seven-stage pipeline at [`compiler-internals.md`](../../website/src/content/docs/architecture/compiler-internals.md#L6-L50). |
| Reference | [`/reference/quonc/`](https://quon.arnabg.me/reference/quonc/) returned 200; [`/reference/`](https://quon.arnabg.me/reference/) returned **404**. | Reference consists only of CLI and pipeline pages in [`astro.config.mjs`](../../website/astro.config.mjs#L82-L88). The CLI page is a 364-line option catalogue beginning at [`quonc.md`](../../website/src/content/docs/reference/quonc.md#L5-L31). |

**Rendered navigation observation.** On the live Quickstart page, the desktop
sidebar exposed seven groups and 35 documentation links: Getting Started, Why
Quon, Language guide, Cookbook, Architecture, Guides, and Reference. It did
not expose the Learning track declared in source. The page was 4,299 CSS pixels
high at the stated viewport. This is evidence of a high-density navigation and
content surface; it is not a claim that any particular screen-reader behavior
is defective.

## Findings

### C1 — Deployment and source navigation disagree; the beginner track is absent

- **Severity:** Critical
- **Affected audience:** Newcomers and educators; secondarily maintainers who
  rely on the source configuration as the public information architecture.
- **Observed evidence:** Source declares an eight-group sidebar including
  **Learning track** ([configuration](../../website/astro.config.mjs#L13-L89)).
  The live sidebar has seven groups and no Learning entry, and the intended
  landing route [`/learn/`](https://quon.arnabg.me/learn/) returns 404. The
  missing page calls itself the “beginner on-ramp” and is meant to hand readers
  to Language and Cookbook ([source](../../website/src/content/docs/learn/index.mdx#L72-L80)).
- **Cause:** The deployed artifact is stale relative to the checked-in content
  and navigation, or the deployment is built/published from a different input.
  The audit does not determine which.
- **Impact:** The site’s designed progressive route is inaccessible. A reader
  entering through Home instead lands on a source-build quickstart, a language
  essay, architecture, or cookbook without the declared six-lesson bridge.
- **Recommendation:** Restore deployment parity first: deploy the declared
  learning routes and sidebar atomically, then add a release check that fetches
  the public route list/sitemap and compares it with the built docs route list.

### C2 — A public prerequisite link resolves to a nonexistent section root

- **Severity:** High
- **Affected audience:** Cookbook readers who follow its stated prerequisite;
  particularly readers entering through a shared cookbook URL.
- **Observed evidence:** The Cookbook says its examples assume circuits,
  linear values, and `run` blocks, and links that phrase to `/language/`
  ([source](../../website/src/content/docs/cookbook/index.mdx#L179-L185)).
  [`/language/`](https://quon.arnabg.me/language/) returns 404; the actual
  introduction route is [`/language/introduction/`](https://quon.arnabg.me/language/introduction/).
  The same audit found `/guides/`, `/architecture/`, `/reference/`, and
  `/why-quon/` section roots also return 404.
- **Cause:** The source tree uses leaf pages without section-index documents,
  but prose and category labels imply stable section destinations.
- **Impact:** A reader following the cookbook’s explicit preparation path
  leaves the site at the exact point the page establishes a prerequisite.
  Category names are also not safe targets for future cross-links or external
  sharing.
- **Recommendation:** Either create short section landing pages for every
  public sidebar category or make every public prose/category link target a
  named leaf page. Correct the Cookbook prerequisite link in the same change;
  do not leave a root alias without a page purpose.

### C3 — The primary home journey bypasses the prerequisite it later requires

- **Severity:** High
- **Affected audience:** First-time users who arrive at the public home page
  and want to run Quon.
- **Observed evidence:** The Home hero’s primary action is **Run the
  quickstart** ([source](../../website/src/content/docs/index.mdx#L5-L15)); its
  first action card repeats that path rather than installation
  ([lines 59–63](../../website/src/content/docs/index.mdx#L59-L63)). The
  Quickstart opens by telling readers to run commands only after completing the
  installation guide ([source](../../website/src/content/docs/getting-started/quickstart.md#L6-L10)).
  The sidebar itself orders Install before Quickstart
  ([configuration](../../website/astro.config.mjs#L14-L20)).
- **Cause:** The global acquisition journey and local sidebar sequence are
  designed independently.
- **Impact:** The most prominent call to action can send an unprepared reader
  to commands that require native dependencies, Devbox, and a built compiler.
  The route works only after the reader notices and reverses to the inline
  installation link.
- **Recommendation:** Make Home’s first-run path explicit: either link the
  primary action to Install with a “then run the Bell quickstart” continuation,
  or retitle/link Quickstart as a two-step install-and-run flow. Keep the
  sidebar’s Install → Quickstart → second-program order as the canonical
  journey.

### C4 — Sidebar categories are not reader routes, and the live menu is too dense to explain them

- **Severity:** High
- **Affected audience:** Readers who know a goal but not Quon’s internal
  taxonomy: prospective users, tool users, researchers, and contributors.
- **Observed evidence:** The live sidebar exposes 35 leaf links over seven
  groups in one navigation surface. The source config gives each category a
  label but only Cookbook has an explicit index page; Guides, Architecture,
  Reference, and Why Quon point directly to leaf material
  ([configuration](../../website/astro.config.mjs#L34-L88)). Their root URLs
  return 404 (C2). The Home offers only language, quickstart, architecture, and
  cookbook cards ([source](../../website/src/content/docs/index.mdx#L52-L75)),
  omitting routes for tools, reference lookup, and the beginner track.
- **Cause:** The sidebar is being used both as a complete content inventory and
  as the first audience-routing mechanism, without category landing pages or
  “when to use this” guidance.
- **Impact:** Readers must infer whether “Guides,” “Architecture,” and
  “Reference” serve task completion, conceptual learning, implementation work,
  or option lookup. The tall, dense menu gives no decision aid before opening a
  technical leaf page.
- **Recommendation:** Add a compact “Choose your path” router and one sentence
  of audience/outcome copy on every category landing page. Route by intent:
  install/run, learn the language, find a CLI option, configure tools/backends,
  understand internals, and evaluate the project. Once those pages exist, keep
  the sidebar as an inventory rather than the only explanation.

### C5 — The canonical Bell example is repeated across entry points without a clear ownership boundary

- **Severity:** Medium
- **Affected audience:** Readers moving from Home to Quickstart, Language, or
  Cookbook; writers who must keep the same explanation accurate.
- **Observed evidence:** The same `bell_state(): Circuit<2, 2, 2, Clifford>`
  definition appears twice on Home ([lines 40–50](../../website/src/content/docs/index.mdx#L40-L50),
  [85–104](../../website/src/content/docs/index.mdx#L85-L104)), then again in
  Quickstart ([lines 11–44](../../website/src/content/docs/getting-started/quickstart.md#L11-L44)),
  Language introduction ([lines 25–80](../../website/src/content/docs/language/introduction.md#L25-L80)),
  and the Bell cookbook recipe ([lines 40–80](../../website/src/content/docs/cookbook/bell.mdx#L40-L80)).
  Each also explains overlapping type/depth/linearity ideas.
- **Cause:** A useful canonical example has been independently expanded at
  multiple layer levels rather than assigned a distinct role per page.
- **Impact:** The first several routes repeat the same code and conceptual
  explanation before they differentiate. This increases maintenance surface and
  makes a reader unsure whether Quickstart, Language, or Cookbook is the next
  appropriate depth.
- **Recommendation:** Retain the example, but give each occurrence one job:
  Home = a minimal teaser; Install/Quickstart = execute it; Learning/Language =
  explain one concept at a time; Cookbook = analyze the verified artifact.
  Link to the owner for all other detail instead of re-explaining the full
  contract.

### C6 — “Language guide,” “Reference,” and “Architecture” have overlapping depth but weak reciprocal handoffs

- **Severity:** Medium
- **Affected audience:** Intermediate language users and compiler contributors
  trying to switch from concepts to lookup material or implementation detail.
- **Observed evidence:** The Language introduction promises concept pages that
  “afterward … stand on [their] own as a reference”
  ([source](../../website/src/content/docs/language/introduction.md#L233-L245)),
  while the sidebar’s separate Reference category contains only `quonc CLI` and
  `Compiler pipeline` ([configuration](../../website/astro.config.mjs#L82-L88)).
  The compiler pipeline reference contains typechecker architecture,
  extraction, physical-layout, and neutral-atom architecture details
  ([source](../../website/src/content/docs/reference/compiler.md#L52-L128)),
  which overlap the internally oriented Architecture page’s seven-stage
  pipeline and modules ([source](../../website/src/content/docs/architecture/compiler-internals.md#L20-L107)).
  Architecture links to Reference at its start, but the reference page’s
  closing links go to CLI and the roadmap, not back to the deeper Architecture
  page ([source](../../website/src/content/docs/reference/compiler.md#L141-L152)).
- **Cause:** The distinction is described locally (“high-level” versus
  “internals”) but is not encoded as an explicit, reciprocal boundary in the
  navigation or entry copy. The source-language reference gap is separately
  documented in the existing benchmark.
- **Impact:** A reader cannot predict whether a topic belongs under Language,
  Reference, or Architecture, and can encounter near-duplicate pipeline detail
  before discovering the intended depth.
- **Recommendation:** State and enforce the boundary: Language = concepts and
  syntax; Reference = stable lookup/contracts; Architecture = implementation
  rationale and extension seams. Put a short “use this when / continue to” box
  at each boundary, make the handoffs reciprocal, and move/cross-link repeated
  pipeline summaries rather than growing both pages.

### C7 — Cookbook presents two conflicting orders: tutorial continuation versus sidebar order

- **Severity:** Medium
- **Affected audience:** Learners following the cookbook progression.
- **Observed evidence:** Cookbook explicitly says “Read the pages in order” and
  frames the sequence as progressively adding concepts
  ([source](../../website/src/content/docs/cookbook/index.mdx#L69-L81)). Its
  declared sidebar orders put **More samples** at 10 and **NA QAOA schedule**
  at 11 ([metadata](../../website/src/content/docs/cookbook/samples.mdx#L1-L5),
  [metadata](../../website/src/content/docs/cookbook/na-qaoa.mdx#L1-L5)). In
  contrast, Shor’s explicit continuation sends the reader directly to NA QAOA
  ([source](../../website/src/content/docs/cookbook/shor-kernel.mdx#L278-L280)).
  “More samples” is a broad repository catalogue, whereas NA QAOA is the
  sequence’s neutral-atom backend transition.
- **Cause:** Discovery/catalog material is given a tutorial position in the
  generated sidebar while hand-authored next links preserve a different lesson
  order.
- **Impact:** A reader following the sidebar and one following page-level
  continuations receive different sequences, so “in order” does not identify a
  canonical path.
- **Recommendation:** Declare one canonical order. Keep the tutorial chain
  continuous through NA QAOA and make “More samples” an explicitly optional
  browse branch after completion (or move its sidebar position after the
  terminal recipe).

## Prioritized remediation map

| Priority | Slice | Resolves | Acceptance evidence |
| --- | --- | --- | --- |
| P0 | **Reconcile and verify deployment.** Publish the checked-in Learning route/sidebar or remove it from source until it is publishable; compare deployed sitemap/routes with the build on every release. | C1; protects all later navigation work. | `/learn/` is 200, its six child routes resolve, and the live sidebar includes Learning track exactly once. |
| P1 | **Repair public route contracts.** Add intentional section indexes or retarget all root/category prose links; fix Cookbook’s `/language/` prerequisite. | C2, C4. | Every public category root and every internal page link returns 200 and identifies a reader’s next action. |
| P1 | **Establish the first-run journey.** Make Home’s primary action honor Install → Quickstart → second program, with clear optional Aer/NA branches. | C3. | A fresh reader can follow the primary home CTA without encountering an unprepared command; all prerequisites appear before use. |
| P1 | **Publish an audience router.** Introduce concise landing pages/route cards for first run, learning, language lookup, CLI/tooling, backend work, architecture, and contribution. | C4 and the handoff portion of C6. | Each sidebar category has audience/outcome copy and a valid landing route; Home exposes the missing high-value intents. |
| P2 | **Assign content ownership to the Bell example.** Reduce repeats to role-specific excerpts and link to the detailed owner. | C5. | Home, Quickstart, Language/Learning, and Cookbook each make a non-overlapping promise and point to the next appropriate depth. |
| P2 | **Clarify the depth taxonomy and cross-links.** Apply the guide/reference/architecture boundary and reciprocal “continue to” links; implement the existing benchmark’s language-reference work as the reference half. | C6. | A reader can answer “concept, stable lookup, or implementation?” from each landing and move in both directions without navigation search. |
| P2 | **Separate cookbook learning from discovery.** Put the catalog after the terminal recipe or make it optional. | C7. | Sequential cookbook navigation retains a continuous Bell → … → NA QAOA arc. |

## Audit notes

- Route-status observations are direct HTTP checks against the deployed domain;
  page-content observations are direct rendered/deployed reads. Source citations
  are checked-in public website files.
- Recommendations deliberately do not prescribe a framework, URL-redirect
  policy, or release platform. Those are implementation choices after ownership
  of the public route contract is decided.
- The existing [Swift documentation benchmark](./swift-documentation-benchmark.md)
  remains the source for the broader source-language reference, diagnostic
  catalog, documentation contribution, and documentation-quality-gate backlog.
