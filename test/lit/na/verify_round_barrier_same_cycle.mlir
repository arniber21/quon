// Negative: layer after Wait must have a strictly later cycle (#256 / ADR-0021).
//
// RUN: not %quonc %s --verify-na 2>&1 | FileCheck %s
// CHECK: quantum.na verification failed
// CHECK: round-barrier Wait
// CHECK: strictly later cycle

module {
  "quantum.na.schedule"() ({
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\"lhs\":{\"atom\":0,\"x_um\":0.0,\"y_um\":0.0},\"rhs\":{\"atom\":1,\"x_um\":6.0,\"y_um\":0.0}}]"} : () -> ()
    }) {cycle = 0 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.wait"() {duration_us = 1 : i64} : () -> ()
    }) {cycle = 1 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\"lhs\":{\"atom\":2,\"x_um\":30.0,\"y_um\":0.0},\"rhs\":{\"atom\":3,\"x_um\":36.0,\"y_um\":0.0}}]"} : () -> ()
    }) {cycle = 1 : i64} : () -> ()
  }) {aod_min_separation_um = 2.0 : f64, min_rydberg_spacing_um = 18.75 : f64, rydberg_range_um = 7.5 : f64, target_id = "generic_reconfigurable_neutral_atom_v0"} : () -> ()
}
