#!/usr/bin/env python3
# Documentation quality-gate corpus validator (issue #377).
#
# Reads docs/doc-manifest.yaml and mechanically validates the declared
# documentation corpus: executable examples that must mirror a tested source
# fixture, command fences whose recipes/paths must resolve, generated excerpts
# that must declare a regenerator, internal links that must resolve, and
# explicitly stale material. See docs/agents/doc-quality.md for the policy.
#
# Failures report the originating page and line so a regression points back to
# the exact snippet. Pure stdlib — no PyYAML required (the docs CI job has no
# Rust toolchain and may not have PyYAML). A minimal YAML parser handles the
# manifest's flat schema; PyYAML is used opportunistically when available for
# richer error messages. No compiler is needed, so executable examples are
# drift-checked against canonical fixtures that ci-rust already compiles — the
# source-compatibility discipline cited in #377.
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
MANIFEST = os.path.join(ROOT, "docs", "doc-manifest.yaml")


def _parse_flow_map(text):
    """Parse a flow-style YAML mapping like {lang: qn, ordinal: 1, mode: executable}.

    Handles unquoted scalars, double-quoted strings, and inline lists. Returns
    a dict. Values are typed: ints stay ints, bare words are strings.
    """
    text = text.strip()
    if text.startswith("{") and text.endswith("}"):
        text = text[1:-1]
    out = {}
    # split on commas not inside quotes/brackets
    parts = []
    depth = 0
    cur = ""
    in_q = False
    qc = ""
    for ch in text:
        if in_q:
            cur += ch
            if ch == qc:
                in_q = False
            continue
        if ch in ('"', "'"):
            in_q = True
            qc = ch
            cur += ch
            continue
        if ch in "[{":
            depth += 1
            cur += ch
            continue
        if ch in "]}":
            depth -= 1
            cur += ch
            continue
        if ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
            continue
        cur += ch
    if cur.strip():
        parts.append(cur)
    for part in parts:
        if ":" not in part:
            continue
        k, _, v = part.partition(":")
        k = k.strip()
        v = v.strip()
        if v.startswith('"') and v.endswith('"'):
            out[k] = v[1:-1]
        elif v.startswith("'") and v.endswith("'"):
            out[k] = v[1:-1]
        elif v.startswith("[") and v.endswith("]"):
            out[k] = [x.strip().strip("'\"") for x in v[1:-1].split(",") if x.strip()]
        else:
            try:
                out[k] = int(v)
            except ValueError:
                out[k] = v
    return out


def parse_manifest(text):
    """Parse the manifest YAML into {version, pages: [{page, checks, examples}]}.

    Hand-rolled for the manifest's flat schema; PyYAML is used when available.
    """
    try:
        import yaml
        return yaml.safe_load(text)
    except ImportError:
        pass
    lines = text.split("\n")
    result = {"version": 1, "pages": []}
    i = 0
    n = len(lines)
    while i < n:
        s = lines[i].strip()
        if s.startswith("#") or not s:
            i += 1
            continue
        if s.startswith("version:"):
            result["version"] = int(s.split(":", 1)[1].strip())
            i += 1
            continue
        if s == "pages:":
            i += 1
            break
        i += 1
    cur = None
    while i < n:
        raw = lines[i]
        s = raw.strip()
        if s.startswith("#") or not s:
            i += 1
            continue
        indent = len(raw) - len(raw.lstrip())
        if indent == 2 and s.startswith("- page:"):
            cur = {"page": s[len("- page:"):].strip(), "checks": [], "examples": []}
            result["pages"].append(cur)
            i += 1
            continue
        if indent == 4 and s.startswith("checks:"):
            val = s[len("checks:"):].strip()
            if val.startswith("["):
                cur["checks"] = [x.strip() for x in val[1:-1].split(",") if x.strip()]
            i += 1
            continue
        if indent == 4 and s.startswith("examples:"):
            val = s[len("examples:"):].strip()
            if val == "[]":
                i += 1
                continue
            i += 1
            while i < n:
                raw2 = lines[i]
                s2 = raw2.strip()
                if s2.startswith("#") or not s2:
                    i += 1
                    continue
                indent2 = len(raw2) - len(raw2.lstrip())
                if indent2 < 6:
                    break
                if s2.startswith("- {"):
                    cur["examples"].append(_parse_flow_map(s2[s2.index("{"):]))
                    i += 1
                    continue
                i += 1
            continue
        i += 1
    return result

# File extensions whose tokens in command fences are treated as repo paths
# that must resolve (avoids flagging /tmp outputs or URLs).
REPO_PATH_EXT = {
    ".qn", ".py", ".sh", ".md", ".mdx", ".json", ".toml", ".rs",
    ".mlir", ".dot", ".txt", ".json", ".yaml", ".yml", ".ts", ".mjs",
    ".js", "Cargo.toml",
}
SKIP_PATH_PREFIXES = ("/tmp", "/var", "/dev", "~", "/usr", "/opt", "/etc")
ALLOWED_BINS = {
    "cargo", "rustup", "devbox", "just", "npx", "pnpm", "npm", "node",
    "python", "python3", "pip", "pip3", "brew", "apt", "git", "echo", "cat",
    "mkdir", "cp", "mv", "rm", "ls", "cd", "true", "false", "test", "which",
    "rustc", "cargo-flux", "llvm-cov", "make", "cmake", "pnpx",
    # project binaries (basename of ./target/{release,debug}/<bin>)
    "quonc", "quonfmt", "quonlint", "quon_lsp", "quon", "naviz",
}
KNOWN_TOP_DIRS = {
    "test", "samples", "scripts", "docs", "website", "frontend", "mlir_bridge",
    "backend", "quonc", "quon_core", "quon_lsp", "quonfmt", "quonlint",
    "quon_qec", "quon_na", "flux_verify", "examples", "targets", "python",
    ".github", ".taskless", "bench", "extensions", "nix", "packaging",
}


def fail(page, line, msg):
    print(f"assert-docs-corpus: FAIL {page}:{line}: {msg}", file=sys.stderr)


def read_lines(rel):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as fh:
        return fh.read().split("\n")


def fence_inventory(lines):
    """Return list of (start_line, lang, end_line) for every fenced block."""
    out = []
    i = 0
    n = len(lines)
    while i < n:
        m = re.match(r"^```([\w-]*)", lines[i])
        if m:
            lang = m.group(1) or "text"
            start = i + 1  # 1-based open fence line
            j = i + 1
            while j < n and not lines[j].startswith("```"):
                j += 1
            out.append((start, lang, j + 1))  # end_line = 1-based close fence (or EOF+1)
            i = j + 1
        else:
            i += 1
    return out


def fence_content(lines, start, end):
    # start is the open-fence line (1-based); content is lines start+1 .. end-1
    return "\n".join(lines[start:end - 1]) if end > start else ""


def nth_fence(fences, lang, ordinal):
    """1-based ordinal among fences with matching lang (lang '' counts as text)."""
    matches = [f for f in fences if f[1] == lang]
    if ordinal < 1 or ordinal > len(matches):
        return None
    return matches[ordinal - 1]


def normalize(code):
    """Strip --/line comments and collapse whitespace for substring mirror."""
    out = []
    for raw in code.split("\n"):
        # strip trailing line comments (Quon uses `--`; shell/other ignored)
        line = re.sub(r"\s*--.*$", "", raw)
        out.append(line)
    text = "\n".join(out)
    text = re.sub(r"\s+", " ", text).strip()
    return text


def _mirror_check(content, src_text):
    """True if the fence content mirrors the source fixture.

    First tries a whole-block substring match (the common case: the fence is a
    contiguous excerpt). If that fails, splits the fence on blank lines into
    chunks and requires each chunk to be a substring of the source — this
    tolerates excerpts that elide intermediate definitions (a blank line in the
    fence stands in for omitted code) while still catching drift in any
    individual fragment. Comments (`-- ...`) are stripped before comparing.
    """
    nc = normalize(content)
    ns = normalize(src_text)
    if nc in ns:
        return True
    chunks = [normalize(c) for c in re.split(r"\n\s*\n", content)]
    chunks = [c for c in chunks if c]
    if len(chunks) > 1 and all(c in ns for c in chunks):
        return True
    return False

def just_recipes():
    """Parse public recipe names from the Justfile (skip [private]).

    Just recipe headers sit at column 0 as `name:` or `name args:`. Attribute
    lines like `[private]` or `[group("x")]` attach to the next recipe; we
    collect pending attributes and apply them when a header appears.
    """
    rec = set()
    pending_attrs = []
    with open(os.path.join(ROOT, "Justfile"), encoding="utf-8") as fh:
        for line in fh:
            if not line.strip() or line.lstrip().startswith("#"):
                pending_attrs = []
                continue
            stripped = line.strip()
            if stripped.startswith("[") and stripped.endswith("]"):
                pending_attrs.append(stripped)
                continue
            m = re.match(r"^([A-Za-z0-9_.-]+)(?:\s|\:)", line)
            # Skip `set`/`export` directives and `NAME := value` assignments
            # (recipes use `:`/` args:`, never `:=`).
            if m and (re.match(r"^[A-Za-z0-9_.-]+\s*:=", line) or m.group(1) in ("set", "export")):
                pending_attrs = []
                continue
            if m:
                name = m.group(1)
                if not any("private" in a for a in pending_attrs):
                    rec.add(name)
                pending_attrs = []
            else:
                pending_attrs = []
    return rec


def repo_path_exists(rel):
    p = os.path.join(ROOT, rel)
    return os.path.exists(p)


def check_command_fence(page, lines, start, end, recipes, errors):
    content = fence_content(lines, start, end)
    for raw in content.split("\n"):
        s = raw.strip()
        if not s or s.startswith("#"):
            continue
        # drop leading env assignments: FOO=bar BAZ=qux <cmd>
        s = re.sub(r"^(?:[A-Z_][A-Z0-9_]*=\S+\s+)+", "", s)
        # split on shell separators into individual commands
        cmds = re.split(r"(?:&&|\|\||\||;)", s)
        for cmd in cmds:
            toks = cmd.split()
            if not toks:
                continue
            head = toks[0]
            # strip path qualifiers on the binary
            bare = os.path.basename(head)
            if head.startswith("./scripts/"):
                if not repo_path_exists(head[2:]):
                    fail(page, start, f"command references missing script {head}")
                    errors[0] += 1
                continue
            if bare in ALLOWED_BINS:
                # validate path-like arguments
                for tok in toks[1:]:
                    _check_path_token(page, start, tok, errors)
                # just recipe?
                if bare == "just":
                    rest = [t for t in toks[1:] if not t.startswith("-") and "=" not in t]
                    if rest:
                        recipe = rest[0]
                        if recipe not in recipes:
                            fail(page, start, f"command references unknown just recipe 'just {recipe}'")
                            errors[0] += 1
                continue
            if head.startswith("http") or head.startswith("/"):
                continue
            # otherwise: maybe a path token itself
            _check_path_token(page, start, head, errors)


def _check_path_token(page, line, tok, errors):
    """Validate a command-fence token that references a repo path.

    Only tokens whose first path component is a known repo top-level dir are
    checked, so illustrative placeholders like `src/main.qn` or
    `path/to/config.toml` are not flagged. Build outputs under `target/` and
    absolute/temp paths are skipped.
    """
    tok = tok.strip("'\"")
    tok = re.sub(r"[<>|&;].*$", "", tok).strip("'\"")
    if not tok or "/" not in tok:
        return
    if tok.startswith(SKIP_PATH_PREFIXES) or tok.startswith("http") or tok.startswith("file:"):
        return
    if tok.startswith("./") or tok.startswith("../"):
        candidate = tok.lstrip("./")
    else:
        candidate = tok
    first = candidate.split("/")[0]
    if first not in KNOWN_TOP_DIRS:
        return  # illustrative placeholder, not a real repo path
    if not repo_path_exists(candidate):
        fail(page, line, f"command references missing repo path '{candidate}'")
        errors[0] += 1


def _resolve_link_target(page, tgt):
    """Resolve a markdown link target to candidate docs file paths.

    Starlight resolves relative links against the source file's directory in
    the content collection (file-path semantics, not URL semantics), and
    absolute `/section/` links against the docs root. Returns a list of
    candidate file paths to check.
    """
    docs_root = "website/src/content/docs/"
    clean = tgt.split("#", 1)[0].split("?", 1)[0]
    if not clean:
        return []
    if clean.startswith("/"):
        rel = clean.lstrip("/")
    else:
        rel = os.path.normpath(os.path.join(os.path.dirname(page), clean))
        if rel.startswith(".."):
            return []
        rel = rel[len(docs_root):] if rel.startswith(docs_root) else rel
    base = rel.strip("/")
    return [
        docs_root + base + ".md",
        docs_root + base + ".mdx",
        docs_root + base + "/index.md",
        docs_root + base + "/index.mdx",
    ]


def _dir_has_doc_page(dir_rel):
    """True if a docs directory exists and contains at least one .md/.mdx."""
    p = os.path.join(ROOT, dir_rel)
    if not os.path.isdir(p):
        return False
    for name in os.listdir(p):
        if name.endswith((".md", ".mdx")) and not name.startswith("_"):
            return True
    return False


def _looks_internal(tgt):
    if tgt.startswith(("#", "mailto:", "http")):
        return tgt.startswith("http") and "github.com/arniber21/quon/blob" in tgt
    if "/" not in tgt and not tgt.startswith((".", "/")):
        return False
    return True


def check_links(page, lines, errors):
    text = "\n".join(lines)
    targets = []
    for m in re.finditer(r"\[[^\]]*\]\(([^)]+)\)", text):
        targets.append(m.group(1).split()[0])
    for m in re.finditer(r"^\s*\[[^\]]+\]:\s*(\S+)", text, re.M):
        targets.append(m.group(1))
    seen = set()
    for tgt in targets:
        if tgt in seen:
            continue
        seen.add(tgt)
        if not _looks_internal(tgt):
            continue
        if tgt.startswith("http"):
            mb = re.match(r"https?://github\.com/arniber21/quon/blob/[^/]+/(.+)", tgt)
            if mb:
                path = mb.group(1).split("#")[0]
                if path and not repo_path_exists(path):
                    fail(page, 1, f"GitHub blob link resolves to missing repo path '{path}'")
                    errors[0] += 1
            continue
        cands = _resolve_link_target(page, tgt)
        if not cands:
            continue
        if any(repo_path_exists(c) for c in cands):
            continue
        # Lenient fallback: a section-landing link like `/language/` or
        # `../cookbook/` may target a directory with child pages but no index.
        # Treat the directory as resolvable so the gate does not false-positive
        # on Starlight sections that rely on sidebar routing to a first child.
        docs_root = "website/src/content/docs/"
        clean = tgt.split("#", 1)[0].split("?", 1)[0]
        if clean.startswith("/"):
            dir_rel = docs_root + clean.strip("/")
        else:
            dir_rel = os.path.normpath(os.path.join(os.path.dirname(page), clean))
        if _dir_has_doc_page(dir_rel):
            continue
        fail(page, 1, f"internal link '{tgt}' does not resolve to a page or section")
        errors[0] += 1


def check_example(page, lines, fences, ex, recipes, errors):
    lang = ex.get("lang", "text")
    ordinal = ex.get("ordinal", 1)
    mode = ex.get("mode", "illustrative")
    f = nth_fence(fences, lang, ordinal)
    if f is None:
        fail(page, 0, f"example {lang}#{ordinal} ({mode}) not found in page")
        errors[0] += 1
        return
    start, _, end = f
    content = fence_content(lines, start, end)
    if mode == "executable":
        verify = ex.get("verify", "mirror")
        if verify == "mirror":
            src = ex.get("source")
            if not src:
                fail(page, start, f"executable {lang}#{ordinal} has no source")
                errors[0] += 1
                return
            if not repo_path_exists(src):
                fail(page, start, f"executable {lang}#{ordinal} source '{src}' missing")
                errors[0] += 1
                return
            with open(os.path.join(ROOT, src), encoding="utf-8") as fh:
                src_text = fh.read()
            if not _mirror_check(content, src_text):
                fail(page, start,
                     f"executable {lang}#{ordinal} drifted from source {src}")
                errors[0] += 1
        else:
            fail(page, start, f"unknown verify mode '{verify}' for executable example")
            errors[0] += 1
    elif mode == "generated":
        regen = ex.get("regen")
        if not regen:
            fail(page, start, f"generated {lang}#{ordinal} declares no regen")
            errors[0] += 1
            return
        # regen must name an existing just recipe or ./scripts/ file
        ok = False
        mtok = regen.split()
        if mtok and mtok[0] == "just" and len(mtok) > 1:
            ok = mtok[1] in recipes
        elif regen.startswith("./scripts/"):
            ok = repo_path_exists(regen[2:])
        elif mtok and mtok[0] in ALLOWED_BINS:
            ok = True
        if not ok:
            fail(page, start, f"generated {lang}#{ordinal} regen '{regen}' is not a known recipe/script")
            errors[0] += 1
        golden = ex.get("golden")
        if golden and not repo_path_exists(golden):
            fail(page, start, f"generated {lang}#{ordinal} golden '{golden}' missing")
            errors[0] += 1
    elif mode == "stale":
        if not ex.get("reason"):
            fail(page, start, f"stale {lang}#{ordinal} requires a reason")
            errors[0] += 1
    elif mode == "illustrative":
        pass  # recorded only
    else:
        fail(page, start, f"unknown mode '{mode}'")
        errors[0] += 1


def main():
    if not repo_path_exists("docs/doc-manifest.yaml"):
        print("assert-docs-corpus: FAIL docs/doc-manifest.yaml missing", file=sys.stderr)
        return 1
    with open(MANIFEST, encoding="utf-8") as fh:
        manifest = parse_manifest(fh.read())
    recipes = just_recipes()
    errors = [0]
    pages = manifest.get("pages") or []
    for entry in pages:
        page = entry["page"]
        if not repo_path_exists(page):
            fail(page, 0, "page file missing")
            errors[0] += 1
            continue
        lines = read_lines(page)
        fences = fence_inventory(lines)
        checks = entry.get("checks") or []
        if "commands" in checks:
            for (start, lang, end) in fences:
                if lang in ("sh", "bash"):
                    check_command_fence(page, lines, start, end, recipes, errors)
        if "links" in checks:
            check_links(page, lines, errors)
        for ex in entry.get("examples") or []:
            check_example(page, lines, fences, ex, recipes, errors)
    if errors[0]:
        print(f"assert-docs-corpus: {errors[0]} failure(s)", file=sys.stderr)
        return 1
    pages_n = len(pages)
    print(f"assert-docs-corpus: OK ({pages_n} pages)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
