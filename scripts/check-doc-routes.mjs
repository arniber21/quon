#!/usr/bin/env node
// Release-time route-parity check for the Quon documentation site (#387).
//
// The checked-in Starlight site declares a Learning track and its lesson
// routes, but a deployment can silently publish a different route tree — the
// deployed navigation omits the track and the intended routes return 404.
// This script makes deployment parity a release contract: it compares the
// route manifest produced by `astro build` (the reviewed artifact) against
// the deployed public route set (the live sitemap) and fails on:
//
//   - missing routes    — built but absent from the deployed sitemap
//   - unexpected routes — deployed but absent from the built manifest
//   - 404 routes        — built routes that return HTTP 404 on the live site
//
// Every mismatch reports the originating source route (the checked-in
// `website/src/content/docs/<slug>.md(x)` file) or the sidebar entry that
// declared it, so a failed release points directly at the build/publish input
// to fix.
//
// Usage:
//   node scripts/check-doc-routes.mjs [options]
//
// Options (all optional; defaults suit CI and local use):
//   --site <url>      Deployed site origin  (default: https://quon.arnabg.me
//                     or QUON_SITE_URL env var)
//   --dist <dir>      Built site directory   (default: website/dist)
//   --src <dir>       Content source dir     (default: website/src/content/docs)
//   --no-probe        Skip live 404 probing (still compares sitemaps)
//   --expected <file> Override built manifest with a sitemap XML file
//                     (useful when the Astro build is broken on main and you
//                     need to audit the deployed site against a known-good
//                     manifest)
//
// Requires Node 18+ (global fetch). Run after `pnpm build` in website/.
//
// Recovery procedure (printed on failure):
//   1. Confirm the built manifest: `cd website && pnpm build` must succeed and
//      `website/dist/sitemap-0.xml` must list every intended route, including
//      the Learning track (`/learn/`, `/learn/01-hello-quon/`, …).
//   2. If the build is missing routes, the source tree or sidebar config in
//      `website/astro.config.mjs` is the build/publish input — fix the
//      checked-in `website/src/content/docs/` files or sidebar entries, never
//      the deployment directly.
//   3. If the build is correct but the deployed site omits routes, the
//      published artifact is stale or corrupt. Redeploy the artifact produced
//      by `pnpm build` (the `website/dist/` directory) from the same commit
//      that passed this check:
//        cd website && pnpm install --frozen-lockfile && pnpm build
//        # Publish website/dist/ to the hosting provider (GitHub Pages,
//        # Cloudflare Pages, etc.) from this commit.
//   4. Re-run this check against the redeployed site until it passes.
//
// See docs/agents/validation.md → "Documentation route parity (#387)".

import { readdir, readFile, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, '..');

// ── argument parsing ────────────────────────────────────────────────────────

function parseArgs(argv) {
  const opts = {
    site: process.env.QUON_SITE_URL || 'https://quon.arnabg.me',
    dist: join(ROOT, 'website', 'dist'),
    src: join(ROOT, 'website', 'src', 'content', 'docs'),
    probe: true,
    expected: null,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--site':     opts.site = argv[++i]; break;
      case '--dist':     opts.dist = argv[++i]; break;
      case '--src':      opts.src = argv[++i]; break;
      case '--expected': opts.expected = argv[++i]; break;
      case '--no-probe': opts.probe = false; break;
      case '-h': case '--help':
        console.log(`Usage: check-doc-routes.mjs [--site URL] [--dist DIR] [--src DIR] [--no-probe] [--expected FILE]`);
        process.exit(0);
      default:
        if (a.startsWith('--')) {
          console.error(`check-doc-routes: unknown option ${a}`);
          process.exit(2);
        }
    }
  }
  return opts;
}

// ── helpers ─────────────────────────────────────────────────────────────────

/** Recursively collect files under dir, returning absolute paths. */
async function walk(dir) {
  const out = [];
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of entries) {
    const full = join(dir, e.name);
    if (e.isDirectory()) {
      out.push(...(await walk(full)));
    } else {
      out.push(full);
    }
  }
  return out;
}

/**
 * Build a map of route path → source file by walking the content directory.
 * Starlight emits `/[slug]/` for every `src/content/docs/[slug].md(x)`.
 *   index.mdx        → /
 *   learn/index.mdx  → /learn/
 *   learn/01-hello-quon.mdx → /learn/01-hello-quon/
 */
async function buildSourceMap(srcDir) {
  const files = (await walk(srcDir)).filter((f) => /\.(md|mdx)$/i.test(f));
  const map = new Map();
  for (const f of files) {
    const rel = relative(srcDir, f).replace(/\\/g, '/'); // e.g. learn/01-hello-quon.mdx
    let slug = rel.replace(/\.(md|mdx)$/i, '');
    // A trailing `index` maps to the directory route: learn/index → /learn/
    if (slug.endsWith('/index')) slug = slug.slice(0, -'/index'.length);
    const route = slug === 'index' ? '/' : '/' + slug + '/';
    map.set(route, f);
  }
  return map;
}

/** Parse a sitemap XML string and return a sorted array of route paths. */
function parseSitemap(xml, siteOrigin) {
  const routes = new Set();
  const re = /<loc>([^<]+)<\/loc>/g;
  let m;
  while ((m = re.exec(xml)) !== null) {
    let url = m[1].trim();
    // Normalise to a site-relative route path.
    try {
      const u = new URL(url);
      url = u.pathname;
    } catch {
      // Not a full URL — strip origin if prefixed.
      url = url.replace(new RegExp('^' + escapeRegex(siteOrigin)), '');
    }
    if (!url.startsWith('/')) url = '/' + url;
    // Astro/Starlight emits trailing slashes for pages.
    if (url !== '/' && !url.endsWith('/')) url += '/';
    routes.add(url);
  }
  return [...routes].sort();
}

function escapeRegex(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Fetch text, following up to 3 redirects. */
async function fetchText(url, { redirects = 3 } = {}) {
  const res = await fetch(url, { redirect: 'manual' });
  if (res.status >= 300 && res.status < 400 && res.headers.get('location') && redirects > 0) {
    const loc = new URL(res.headers.get('location'), url).href;
    return fetchText(loc, { redirects: redirects - 1 });
  }
  if (!res.ok) throw new Error(`HTTP ${res.status} fetching ${url}`);
  return res.text();
}

/** HEAD-probe a route; return the HTTP status code (or 0 on network error). */
async function probeRoute(siteOrigin, route) {
  const url = siteOrigin.replace(/\/$/, '') + route;
  try {
    const res = await fetch(url, { method: 'HEAD', redirect: 'follow' });
    return res.status;
  } catch {
    // Some hosts reject HEAD; fall back to GET.
    try {
      const res = await fetch(url, { redirect: 'follow' });
      return res.status;
    } catch {
      return 0;
  }
  }
}

/** Resolve the deployed sitemap URL list from the live site. */
async function fetchDeployedRoutes(siteOrigin) {
  // Starlight/Astro emits sitemap-index.xml → sitemap-0.xml.
  const indexUrl = siteOrigin.replace(/\/$/, '') + '/sitemap-index.xml';
  let routes = [];
  try {
    const idx = await fetchText(indexUrl);
    const subSitemaps = parseSitemap(idx, siteOrigin);
    for (const sm of subSitemaps) {
      const smUrl = /^https?:/.test(sm)
        ? sm
        : siteOrigin.replace(/\/$/, '') + (sm.startsWith('/') ? sm : '/' + sm);
      const body = await fetchText(smUrl);
      routes.push(...parseSitemap(body, siteOrigin));
    }
  } catch {
    // Fall back to sitemap-0.xml directly.
    const body = await fetchText(siteOrigin.replace(/\/$/, '') + '/sitemap-0.xml');
    routes = parseSitemap(body, siteOrigin);
  }
  return [...new Set(routes)].sort();
}

// ── main ────────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const failures = [];

  // 1. Built route manifest.
  let builtRoutes;
  let sourceMap;
  if (opts.expected) {
    const xml = await readFile(opts.expected, 'utf8');
    builtRoutes = parseSitemap(xml, opts.site);
    sourceMap = await buildSourceMap(opts.src);
  } else {
    const sitemapPath = join(opts.dist, 'sitemap-0.xml');
    if (!existsSync(sitemapPath)) {
      console.error(`check-doc-routes: built sitemap not found at ${sitemapPath}`);
      console.error('  Run `pnpm build` in website/ first, or pass --expected <sitemap.xml>.');
      process.exit(1);
    }
    const xml = await readFile(sitemapPath, 'utf8');
    builtRoutes = parseSitemap(xml, opts.site);
    sourceMap = await buildSourceMap(opts.src);
  }

  if (builtRoutes.length === 0) {
    console.error('check-doc-routes: built manifest is empty — refusing to pass vacuously.');
    process.exit(1);
  }

  console.log(`check-doc-routes: built manifest has ${builtRoutes.length} route(s).`);
  for (const r of builtRoutes) {
    const src = sourceMap.get(r) || '(no source file)';
    console.log(`  built  ${r}  ← ${relative(ROOT, src)}`);
  }

  // 2. Deployed route set.
  let deployedRoutes;
  try {
    deployedRoutes = await fetchDeployedRoutes(opts.site);
  } catch (e) {
    console.error(`check-doc-routes: cannot fetch deployed sitemap from ${opts.site}: ${e.message}`);
    process.exit(1);
  }
  console.log(`check-doc-routes: deployed sitemap has ${deployedRoutes.length} route(s).`);

  // 3. Compare.
  const builtSet = new Set(builtRoutes);
  const deployedSet = new Set(deployedRoutes);

  const missing = builtRoutes.filter((r) => !deployedSet.has(r));
  const unexpected = deployedRoutes.filter((r) => !builtSet.has(r));

  if (missing.length) {
    console.error('\ncheck-doc-routes: MISSING routes (built but not deployed):');
    for (const r of missing) {
      const src = sourceMap.get(r) || '(no source file)';
      console.error(`  MISSING  ${r}  ← ${relative(ROOT, src)}`);
    }
    failures.push(...missing.map((r) => `missing:${r}`));
  }

  if (unexpected.length) {
    console.error('\ncheck-doc-routes: UNEXPECTED routes (deployed but not built):');
    for (const r of unexpected) {
      console.error(`  UNEXPECTED  ${r}`);
    }
    failures.push(...unexpected.map((r) => `unexpected:${r}`));
  }

  // 4. 404 probe: confirm built routes are live and reachable.
  if (opts.probe) {
    console.log('\ncheck-doc-routes: probing built routes for 404s …');
    const notFound = [];
    for (const r of builtRoutes) {
      const status = await probeRoute(opts.site, r);
      if (status === 404) {
        const src = sourceMap.get(r) || '(no source file)';
        console.error(`  404  ${r}  ← ${relative(ROOT, src)}`);
        notFound.push(r);
      }
    }
    if (notFound.length) {
      failures.push(...notFound.map((r) => `404:${r}`));
    }
  }

  // 5. Verdict.
  if (failures.length) {
    console.error(`\ncheck-doc-routes: FAILED — ${failures.length} mismatch(es).`);
    console.error('\nRecovery procedure:');
    console.error('  1. Confirm the build: `cd website && pnpm build` must succeed and');
    console.error('     website/dist/sitemap-0.xml must list every intended route.');
    console.error('  2. If routes are missing from the build, fix the checked-in source');
    console.error('     files in website/src/content/docs/ or sidebar entries in');
    console.error('     website/astro.config.mjs — never patch the deployment directly.');
    console.error('  3. If the build is correct but the deployed site omits routes, the');
    console.error('     published artifact is stale. Redeploy website/dist/ from the');
    console.error('     commit that passed this check:');
    console.error('       cd website && pnpm install --frozen-lockfile && pnpm build');
    console.error('       # Publish website/dist/ to the hosting provider.');
    console.error('  4. Re-run: node scripts/check-doc-routes.mjs');
    console.error(`\nSee docs/agents/validation.md → "Documentation route parity (#387)".`);
    process.exit(1);
  }

  console.log('\ncheck-doc-routes: OK — deployed site matches built route manifest.');
}

main().catch((e) => {
  console.error(`check-doc-routes: fatal — ${e.message}`);
  process.exit(1);
});
