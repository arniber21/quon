module {
  "quantum.na.schedule"() ({
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 0 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 1 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 2 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 3 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 4 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 5 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 6 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 7 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 8 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 9 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 10 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 11 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.transfer"() {aod_id = 0 : i64, atom = 0 : i64, col = 0 : i64, direction = "slm_to_aod", duration_us = 15 : i64, row = 0 : i64, site = 0 : i64} : () -> ()
      "quantum.na.transfer"() {aod_id = 0 : i64, atom = 1 : i64, col = 1 : i64, direction = "slm_to_aod", duration_us = 15 : i64, row = 0 : i64, site = 1 : i64} : () -> ()
    }) {cycle = 12 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.move"() {duration_us = 336 : i64, moves = "[{\22atom\22:0,\22from_site\22:0,\22to_site\22:7373,\22aod_id\22:0,\22row\22:0,\22col\22:0,\22from_x_um\22:0.0,\22from_y_um\22:0.0,\22to_x_um\22:0.0,\22to_y_um\22:310.0},{\22atom\22:1,\22from_site\22:1,\22to_site\22:7374,\22aod_id\22:0,\22row\22:0,\22col\22:1,\22from_x_um\22:4.0,\22from_y_um\22:0.0,\22to_x_um\22:2.0,\22to_y_um\22:310.0}]"} : () -> ()
    }) {cycle = 13 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.transfer"() {aod_id = 0 : i64, atom = 0 : i64, col = 0 : i64, direction = "aod_to_slm", duration_us = 15 : i64, row = 0 : i64, site = 7373 : i64} : () -> ()
      "quantum.na.transfer"() {aod_id = 0 : i64, atom = 1 : i64, col = 1 : i64, direction = "aod_to_slm", duration_us = 15 : i64, row = 0 : i64, site = 7374 : i64} : () -> ()
    }) {cycle = 14 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 15 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.78539816339744828 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 16 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 17 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 18 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 19 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 20 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 21 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 22 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 23 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 24 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 25 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 26 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 27 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 28 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 29 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 30 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 31 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 32 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 33 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 34 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 35 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 36 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 37 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 38 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 39 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 40 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.78539816339744828 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 41 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 42 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 43 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 44 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 45 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 46 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 47 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 48 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 49 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 50 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 51 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 52 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 53 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 54 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 55 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 56 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 57 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 58 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 59 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 60 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 61 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 62 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 63 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 64 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 65 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.78539816339744828 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 66 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 67 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 68 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 69 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 70 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 71 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 72 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 73 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 74 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 75 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 76 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 77 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 78 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.39269908169872414 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 79 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 80 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 81 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 82 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 83 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 84 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 85 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 86 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = 3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 87 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.global_ry"() {duration_us = 1 : i64, theta = 0.78539816339744828 : f64} : () -> ()
    }) {cycle = 88 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 0 : i64, duration_us = 1 : i64, gate = "rz", theta = -3.1415926535897931 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 89 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 90 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.local_gate"(%0) {atom = 1 : i64, duration_us = 1 : i64, gate = "rz", theta = 0.78539816339744828 : f64} : (!quantum.na.atom) -> !quantum.na.atom
    }) {cycle = 91 : i64} : () -> ()
    "quantum.na.layer"() ({
      "quantum.na.entangle"() {duration_us = 1 : i64, pairs = "[{\22lhs\22:{\22atom\22:0,\22x_um\22:0.0,\22y_um\22:310.0},\22rhs\22:{\22atom\22:1,\22x_um\22:2.0,\22y_um\22:310.0}}]"} : () -> ()
    }) {cycle = 92 : i64} : () -> ()
    "quantum.na.layer"() ({
      %0 = "quantum.na.alloc_atom"() {atom = 0 : i64} : () -> !quantum.na.atom
      %1 = "quantum.na.measure"(%0) {atom = 0 : i64, basis = "z", duration_us = 1500 : i64} : (!quantum.na.atom) -> !quantum.bit
      %2 = "quantum.na.alloc_atom"() {atom = 1 : i64} : () -> !quantum.na.atom
      %3 = "quantum.na.measure"(%2) {atom = 1 : i64, basis = "z", duration_us = 1500 : i64} : (!quantum.na.atom) -> !quantum.bit
    }) {cycle = 93 : i64} : () -> ()
  }) {aod_min_separation_um = 2.000000e+00 : f64, min_rydberg_spacing_um = 1.875000e+01 : f64, rydberg_range_um = 7.500000e+00 : f64, target_id = "generic_reconfigurable_neutral_atom_v0"} : () -> ()
}
