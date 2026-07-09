"""
LLVM lit configuration — see issue #28, SPEC.md §10.1

Run:
    lit test/lit/
    lit test/lit/emit/        # emission tests only
    lit test/lit/circ/        # quantum.circ pass tests only
    lit test/lit/dynamic/     # quantum.dynamic pass tests only
"""

import lit.formats
import os

config.name = "quon"
config.test_format = lit.formats.ShTest(True)
config.suffixes = [".qn", ".mlir"]
config.test_source_root = os.path.dirname(__file__)
config.test_exec_root = os.path.join(os.environ.get("QUON_BUILD_DIR", "."), "test")

# Require quonc, the round-trip oracles, and FileCheck on PATH.
# %circ-roundtrip is mlir_bridge's `circ_roundtrip` example (issue #4).
# %dynamic-roundtrip is mlir_bridge's `dynamic_roundtrip` example (issue #6).
# %na-roundtrip is quon_na's `na_roundtrip` example (issue #102).
config.substitutions.append(("%quonc", "quonc"))
config.substitutions.append(("%circ-roundtrip", "circ_roundtrip"))
config.substitutions.append(("%circ-lower", "circ_lower"))
config.substitutions.append(("%dynamic-roundtrip", "dynamic_roundtrip"))
config.substitutions.append(("%na-roundtrip", "na_roundtrip"))
config.substitutions.append(("%monadic-lower", "monadic_lower"))
config.substitutions.append(("%gate-cancel", "gate_cancel"))
config.substitutions.append(("%rotation-merge", "rotation_merge"))
config.substitutions.append(("%measurement-defer", "measurement_defer"))
config.substitutions.append(("%classical-region-fuse", "classical_region_fuse"))
config.substitutions.append(("%native-gate-decomp", "native_gate_decomp"))
config.substitutions.append(("%sabre-route", "sabre_route"))
config.substitutions.append(("%depth-schedule", "depth_schedule"))
config.substitutions.append(("%FileCheck", "FileCheck"))
