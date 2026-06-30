#!/usr/bin/env python3
"""E2E fixture smoke test for teleport.qn."""

from pathlib import Path
import sys

qn = Path(__file__).with_suffix(".qn")
if not qn.is_file():
    sys.exit(f"missing {qn}")
text = qn.read_text()
assert "circuit" in text or "fn" in text
print("ok", qn.name)
