// Negative: schedule cycles must be non-decreasing (#256).
//
// RUN: not %quonc %s --verify-na 2>&1 | FileCheck %s
// CHECK: quantum.na verification failed
// CHECK: schedule cycles must be non-decreasing

module {
  "quantum.na.schedule"() ({
    "quantum.na.layer"() ({
      "quantum.na.wait"() {duration_us = 1 : i64} : () -> ()
    }) {cycle = 2 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.wait"() {duration_us = 1 : i64} : () -> ()
    }) {cycle = 1 : i64} : () -> ()
  }) {aod_min_separation_um = 2.0 : f64, min_rydberg_spacing_um = 18.75 : f64, rydberg_range_um = 7.5 : f64, target_id = "generic_reconfigurable_neutral_atom_v0"} : () -> ()
}
