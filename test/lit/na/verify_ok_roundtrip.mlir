// Positive standalone --verify-na on a legal quantum.na schedule (#256).
//
// RUN: %quonc %s --verify-na --quiet

module {
  "quantum.na.schedule"() ({
    "quantum.na.layer"() ({
      "quantum.na.move"() {duration_us = 20 : i64, moves = "[{\"atom\":0,\"from_site\":0,\"to_site\":10,\"aod_id\":0,\"row\":0,\"col\":0,\"from_x_um\":0.0,\"from_y_um\":0.0,\"to_x_um\":0.0,\"to_y_um\":2.0},{\"atom\":1,\"from_site\":1,\"to_site\":11,\"aod_id\":0,\"row\":1,\"col\":1,\"from_x_um\":10.0,\"from_y_um\":10.0,\"to_x_um\":10.0,\"to_y_um\":12.0}]"} : () -> ()
    }) {cycle = 0 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\"lhs\":{\"atom\":0,\"x_um\":0.0,\"y_um\":0.0},\"rhs\":{\"atom\":1,\"x_um\":6.0,\"y_um\":0.0}}]"} : () -> ()
      "quantum.na.wait"() {duration_us = 3 : i64} : () -> ()
    }) {cycle = 1 : i64} : () -> ()
  }) {aod_min_separation_um = 2.0 : f64, min_rydberg_spacing_um = 18.75 : f64, rydberg_range_um = 7.5 : f64, target_id = "generic_reconfigurable_neutral_atom_v0"} : () -> ()
}
