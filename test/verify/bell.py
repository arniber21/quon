#!/usr/bin/env python3
"""E2E fixture smoke test — validates .qn sources until `quonc` pipeline is wired."""

from __future__ import annotations

import sys
from pathlib import Path


def main() -> int:
    qn = Path(__file__).with_suffix(".qn")
    if not qn.is_file():
        print(f"FAIL: missing fixture {qn}", file=sys.stderr)
        return 1
    text = qn.read_text(encoding="utf-8")
    if "circuit" not in text and "fn" not in text:
        print(f"FAIL: {qn} does not look like Quon source", file=sys.stderr)
        return 1
    print(f"ok {qn.name} ({len(text)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
