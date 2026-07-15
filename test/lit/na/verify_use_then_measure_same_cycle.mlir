// Negative: Use then Measure same cycle is order-independent (#256).
//
// RUN: not %quonc %s --verify-na 2>&1 | FileCheck %s
// CHECK: quantum.na verification failed
// CHECK: measured and entangled or moved in the same cycle
// CHECK: measurement ordering

module {
  "quantum.na.schedule"() ({
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\"lhs\":{\"atom\":0,\"x_um\":0.0,\"y_um\":0.0},\"rhs\":{\"atom\":1,\"x_um\":6.0,\"y_um\":0.0}}]"} : () -> ()
      %a = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %b = "quantum.na.measure"(%a) {atom = 0 : i64, basis = "z", duration_us = 10 : i64} : (!quantum.na.atom) -> !quantum.bit
    }) {cycle = 0 : i64} : () -> ()
  }) {aod_min_separation_um = 2.0 : f64, min_rydberg_spacing_um = 18.75 : f64, rydberg_range_um = 7.5 : f64, target_id = "generic_reconfigurable_neutral_atom_v0"} : () -> ()
}
