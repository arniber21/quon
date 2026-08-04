#!/usr/bin/env node
// Check the built Starlight site for broken internal links.
//
// Run after `astro build` (which writes website/dist). Every internal `href`
// (a path beginning with "/" but not "//") must resolve to a file that Astro
// emitted under dist. External URLs, mailto/tel, fragments, and Astro's
// hashed asset URLs are ignored. A single broken link fails the check so the
// public route/link contracts in the sidebar and prose stay protected in CI.
//
// Usage: node scripts/check-links.mjs [dist-dir]

import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join, extname } from 'node:path';

const dist = process.argv[2] || new URL('../dist/', import.meta.url).pathname;

function walkHtml(dir, out = []) {
	for (const name of readdirSync(dir)) {
		const p = join(dir, name);
		const st = statSync(p);
		if (st.isDirectory()) walkHtml(p, out);
		else if (extname(name) === '.html') out.push(p);
	}
	return out;
}

// Resolve an internal URL path to a file under dist, or null if not found.
// Astro emits directory-style URLs: /foo/ -> dist/foo/index.html.
function resolveHref(url) {
	let path = url;
	// Strip query string and fragment.
	path = path.split('#')[0].split('?')[0];
	if (path === '' || path === '/') return join(dist, 'index.html');
	// Astro static assets (fonts, images, icons) live at the dist root.
	const direct = join(dist, path);
	if (existsSync(direct) && statSync(direct).isFile()) return direct;
	// Directory-style page: /foo/ or /foo -> dist/foo/index.html
	const trimmed = path.replace(/\/+$/, '');
	const indexHtml = join(dist, trimmed, 'index.html');
	if (existsSync(indexHtml)) return indexHtml;
	// Some links reference /foo/bar.html directly.
	if (!path.endsWith('/')) {
		const htmlExt = join(dist, `${trimmed}.html`);
		if (existsSync(htmlExt)) return htmlExt;
	}
	return null;
}

if (!existsSync(dist)) {
	console.error(`check-links: dist directory not found: ${dist}`);
	console.error('Run `pnpm build` first.');
	process.exit(2);
}

const htmlFiles = walkHtml(dist);
const HREF_RE = /href="([^"]+)"/g;
const broken = [];
const seen = new Set();

for (const file of htmlFiles) {
	const html = readFileSync(file, 'utf8');
	let m;
	while ((m = HREF_RE.exec(html)) !== null) {
		const href = m[1];
		// Only internal absolute paths; skip protocol-relative, external,
		// mailto, tel, and anchor-only /Astro client routes.
		if (!href.startsWith('/') || href.startsWith('//')) continue;
		if (href.startsWith('/mailto:') || href.startsWith('mailto:')) continue;
		if (href.startsWith('tel:')) continue;
		if (href === '') continue;
		const key = `${file} -> ${href}`;
		if (seen.has(key)) continue;
		seen.add(key);
		const resolved = resolveHref(href);
		if (!resolved) {
			broken.push({ from: file, href });
		}
	}
}

if (broken.length > 0) {
	console.error(`check-links: ${broken.length} broken internal link(s) found:\n`);
	for (const b of broken) {
		const rel = b.from.replace(dist, '');
		console.error(`  ${rel}\n    -> ${b.href}`);
	}
	process.exit(1);
}

console.log(`check-links: OK (${htmlFiles.length} pages scanned, all internal links resolve)`);
