"""
LLVM lit configuration — see issue #28, SPEC.md §10.1

Run:
    lit test/lit/
    lit test/lit/emit/        # emission tests only
    lit test/lit/circ/        # quantum.circ pass tests only
"""

import lit.formats
import os

config.name = "quon"
config.test_format = lit.formats.ShTest(True)
config.suffixes = [".qn", ".mlir"]
config.test_source_root = os.path.dirname(__file__)
config.test_exec_root = os.path.join(os.environ.get("QUON_BUILD_DIR", "."), "test")

# Require quonc and FileCheck on PATH
config.substitutions.append(("%quonc", "quonc"))
config.substitutions.append(("%FileCheck", "FileCheck"))
