// Negative: duplicate atom occupancy via CLI --verify-na (#256).
//
// RUN: not %quonc %s --verify-na 2>&1 | FileCheck %s
// CHECK: quantum.na verification failed
// CHECK: multiple occupancy claims

module {
  "quantum.na.schedule"() ({
    "quantum.na.layer"() ({
      "quantum.na.move"() {duration_us = 20 : i64, moves = "[{\"atom\":0,\"from_site\":0,\"to_site\":10,\"aod_id\":0,\"row\":0,\"col\":0,\"from_x_um\":0.0,\"from_y_um\":0.0,\"to_x_um\":0.0,\"to_y_um\":2.0},{\"atom\":0,\"from_site\":1,\"to_site\":11,\"aod_id\":0,\"row\":1,\"col\":1,\"from_x_um\":10.0,\"from_y_um\":10.0,\"to_x_um\":10.0,\"to_y_um\":12.0}]"} : () -> ()
    }) {cycle = 0 : i64} : () -> ()
  }) {aod_min_separation_um = 2.0 : f64, min_rydberg_spacing_um = 18.75 : f64, rydberg_range_um = 7.5 : f64, target_id = "generic_reconfigurable_neutral_atom_v0"} : () -> ()
}
