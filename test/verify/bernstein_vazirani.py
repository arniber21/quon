#!/usr/bin/env python3
from pathlib import Path
import sys

qn = Path(__file__).with_suffix(".qn")
assert qn.is_file(), qn
assert len(qn.read_text().strip()) > 0
print("ok", qn.name)
