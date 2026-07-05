// Classical region fusion test (issue #23).
//
// RUN: %classical-region-fuse < %s | FileCheck %s

module {
  // Two `if`s on the same classical bit fuse into one, sharing a single
  // condition check for both the X and Z corrections.
  // CHECK-COUNT-1: "quantum.dynamic.if"
  // CHECK: gate_name = "X"
  // CHECK: gate_name = "Z"

  %q0 = "test.qubit"() : () -> !quantum.qubit
  %q1 = "test.qubit"() : () -> !quantum.qubit
  %q2 = "test.qubit"() : () -> !quantum.qubit
  %b = "quantum.dynamic.measure"(%q0) : (!quantum.qubit) -> !quantum.bit

  %a = "quantum.dynamic.if"(%b, %q1) ({
  ^bb0(%arg0: !quantum.qubit):
    %x = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.dynamic.yield"(%x) : (!quantum.qubit) -> ()
  }, {
  ^bb0(%arg1: !quantum.qubit):
    "quantum.dynamic.yield"(%arg1) : (!quantum.qubit) -> ()
  }) : (!quantum.bit, !quantum.qubit) -> !quantum.qubit

  "quantum.dynamic.if"(%b, %q2) ({
  ^bb0(%arg2: !quantum.qubit):
    %z = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "Z"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.dynamic.yield"(%z) : (!quantum.qubit) -> ()
  }, {
  ^bb0(%arg3: !quantum.qubit):
    "quantum.dynamic.yield"(%arg3) : (!quantum.qubit) -> ()
  }) : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
}
