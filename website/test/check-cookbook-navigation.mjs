// Navigation test for the Cookbook curriculum (#388).
//
// Asserts that the Cookbook declares ONE canonical sequential curriculum
// (Bell -> ... -> NA QAOA schedule capstone) and that the sidebar ordering,
// per-page "Next" continuation links, and frontmatter `sidebar.order` all
// agree with it. Sample discovery (the `samples/` catalog and the sample-based
// recipes) must remain optional: it sits after the capstone in the sidebar and
// never interrupts the curriculum.
//
// Run: node test/check-cookbook-navigation.mjs  (from the website/ directory)
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const websiteDir = join(here, '..');
const docsDir = join(websiteDir, 'src/content/docs/cookbook');
const configPath = join(websiteDir, 'astro.config.mjs');

let failures = 0;
function check(cond, msg) {
	if (!cond) {
		console.error(`  ✗ ${msg}`);
		failures++;
	} else {
		console.log(`  ✓ ${msg}`);
	}
}

// Canonical curriculum sequence (slug order), then the optional sample-discovery
// group. The sidebar, prev/next, and page-level continuation links must agree.
const curriculum = [
	['cookbook', 'index.mdx', 1],
	['cookbook/bell', 'bell.mdx', 2],
	['cookbook/teleportation', 'teleportation.mdx', 3],
	['cookbook/bernstein-vazirani', 'bernstein-vazirani.mdx', 4],
	['cookbook/grover', 'grover.mdx', 5],
	['cookbook/qft', 'qft.mdx', 6],
	['cookbook/ising', 'ising.mdx', 7],
	['cookbook/qaoa', 'qaoa.mdx', 8],
	['cookbook/shor-kernel', 'shor-kernel.mdx', 9],
	['cookbook/na-qaoa', 'na-qaoa.mdx', 10],
];
const optional = [
	['cookbook/samples', 'samples.mdx', 11],
	['cookbook/deutsch-jozsa', 'deutsch-jozsa.mdx', 12],
	['cookbook/simon', 'simon.mdx', 13],
	['cookbook/phase-estimation', 'phase-estimation.mdx', 14],
];
const allPages = [...curriculum, ...optional];

// --- Parse the sidebar order from astro.config.mjs (document order of slugs). ---
const configText = readFileSync(configPath, 'utf8');
const sidebarSlugRe = /slug:\s*'([^']+)'/g;
const sidebarSlugs = [];
let m;
while ((m = sidebarSlugRe.exec(configText)) !== null) {
	if (m[1] === 'cookbook' || m[1].startsWith('cookbook/')) {
		sidebarSlugs.push(m[1]);
	}
}

console.log('\n# Sidebar ordering');
check(JSON.stringify(sidebarSlugs) === JSON.stringify(allPages.map((p) => p[0])),
	'sidebar lists cookbook pages in canonical order: curriculum then optional group');

const curriculumSlugs = curriculum.map((p) => p[0]);
check(JSON.stringify(sidebarSlugs.slice(0, curriculumSlugs.length)) === JSON.stringify(curriculumSlugs),
	'curriculum pages form a contiguous prefix of the sidebar (no sample page interrupts)');

check(sidebarSlugs[curriculumSlugs.length - 1] === 'cookbook/na-qaoa',
	'NA QAOA schedule is the last curriculum page in the sidebar (capstone)');

const optionalSlugs = optional.map((p) => p[0]);
check(JSON.stringify(sidebarSlugs.slice(curriculumSlugs.length)) === JSON.stringify(optionalSlugs),
	'sample-discovery pages follow the capstone, in order');

// --- Parse each cookbook page's frontmatter and continuation link. ---
function readPage(file) {
	const text = readFileSync(join(docsDir, file), 'utf8');
	const fmMatch = text.match(/^---\n([\s\S]*?)\n---\n/);
	const fm = fmMatch ? fmMatch[1] : '';
	const orderMatch = fm.match(/sidebar:\s*\n\s*order:\s*(\d+)/);
	const order = orderMatch ? Number(orderMatch[1]) : null;
	// Continuation link: "→ Next: [Label](./slug/)" or "→ Start with: [Label](./slug/)".
	//   group[1]=kind, group[2]=label, group[3]=slug.
	const linkMatch = text.match(
		/→\s*(Next|Start with):\s*\[([^\]]+)\]\(\.\/([^)]+)\//,
	);
	// Detect a curriculum-style "→ Next:" continuation specifically.
	const nextMatch = text.match(/→\s*Next:\s*\[([^\]]+)\]\(\.\/([^)]+)\//);
	return { text, fm, order, link: linkMatch, hasNext: !!nextMatch };
}

console.log('\n# Frontmatter sidebar.order agrees with sidebar sequence');
for (const [slug, file, expectedOrder] of allPages) {
	const page = readPage(file);
	check(page.order === expectedOrder,
		`${file} frontmatter sidebar.order === ${expectedOrder} (got ${page.order})`);
}

console.log('\n# Page-level continuation links follow the curriculum');
{
	const idx = readPage('index.mdx');
	check(idx.link && idx.link[3] === 'bell',
		'index page "→ Start with" points to the Bell state (./bell/)');
	check(!idx.hasNext,
		'index page has no "→ Next:" continuation (it is the landing page)');
}

for (let i = 1; i < curriculum.length - 1; i++) {
	const [, file] = curriculum[i];
	const nextSlug = curriculum[i + 1][0].replace('cookbook/', '');
	const page = readPage(file);
	check(page.hasNext && page.link[3] === nextSlug,
		`${file} "→ Next:" points to ./${nextSlug}/ (next curriculum page)`);
}

{
	const cap = readPage('na-qaoa.mdx');
	check(!cap.hasNext,
		'na-qaoa (capstone) has no "→ Next:" curriculum continuation (curriculum ends here)');
	check(/Optional/i.test(cap.text) && /samples\//.test(cap.text),
		'na-qaoa marks the samples pointer as optional, not a continuation');
}

console.log('\n# Sample discovery is visibly optional and resumable');
{
	const samples = readPage('samples.mdx');
	check(/optional/i.test(samples.text),
		'samples page declares itself optional');
	check(/resume the curriculum/i.test(samples.text),
		'samples page explains how to resume the curriculum');
	check(/na-qaoa\//.test(samples.text),
		'samples page links back to the capstone (na-qaoa) to resume');
	check(samples.order > readPage('na-qaoa.mdx').order,
		'samples frontmatter order is after the capstone');
}

console.log('\n# Sample-based recipes do not interrupt the curriculum');
for (const file of ['deutsch-jozsa.mdx', 'simon.mdx', 'phase-estimation.mdx']) {
	const sidebarIdx = sidebarSlugs.indexOf(
		'cookbook/' + file.replace('.mdx', ''),
	);
	const capIdx = sidebarSlugs.indexOf('cookbook/na-qaoa');
	check(sidebarIdx > capIdx,
		`${file} appears after the capstone in the sidebar (does not interrupt)`);
}

console.log('\n# Sample-recipe chain is independent of the curriculum');
{
	const dj = readPage('deutsch-jozsa.mdx');
	const simon = readPage('simon.mdx');
	check(dj.hasNext && dj.link[3] === 'simon',
		'deutsch-jozsa "→ Next:" points to Simon (./simon/)');
	check(simon.hasNext && simon.link[3] === 'phase-estimation',
		'simon "→ Next:" points to phase estimation (./phase-estimation/)');
	check(!readPage('phase-estimation.mdx').hasNext,
		'phase estimation is the end of the sample-recipe chain (no "→ Next:")');
}

console.log('');
if (failures > 0) {
	console.error(`\nFAIL: ${failures} navigation assertion(s) failed.`);
	process.exit(1);
} else {
	console.log('PASS: cookbook curriculum navigation is consistent.');
}
