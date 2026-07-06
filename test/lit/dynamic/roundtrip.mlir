// Round-trip test for the quantum.dynamic dialect (issue #6).
//
// Parses a quantum.dynamic module, reprints it, and checks the text is stable.
// The input is already in MLIR's canonical generic form so a faithful
// round-trip reproduces it verbatim.
//
// RUN: %dynamic-roundtrip < %s | FileCheck %s

module {
  // CHECK: "test.qubit"
  %0 = "test.qubit"() : () -> !quantum.qubit
  // CHECK: "test.qubit"
  %1 = "test.qubit"() : () -> !quantum.qubit
  // CHECK: "quantum.dynamic.measure"
  %2 = "quantum.dynamic.measure"(%0) : (!quantum.qubit) -> !quantum.bit
  // CHECK: "quantum.dynamic.reset"
  %3 = "quantum.dynamic.reset"(%1) : (!quantum.qubit) -> !quantum.qubit
  // CHECK: "quantum.dynamic.barrier"
  %4 = "quantum.dynamic.barrier"(%3) : (!quantum.qubit) -> !quantum.qubit
  // CHECK: "quantum.dynamic.unitary_region"
  %5 = "quantum.dynamic.unitary_region"(%4) ({
  // CHECK-NEXT: ^bb0(%arg0: !quantum.qubit):
  ^bb0(%arg0: !quantum.qubit):
    // CHECK-NEXT: %7 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %7 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    // CHECK-NEXT: "quantum.circ.return"(%7) : (!quantum.qubit) -> ()
    "quantum.circ.return"(%7) : (!quantum.qubit) -> ()
  // CHECK-NEXT: }) {clifford = true, depth = "1"} : (!quantum.qubit) -> !quantum.qubit
  }) {clifford = true, depth = "1"} : (!quantum.qubit) -> !quantum.qubit
  // CHECK: "quantum.dynamic.if"
  %6 = "quantum.dynamic.if"(%2, %5) ({
  // CHECK-NEXT: ^bb0(%arg0: !quantum.qubit):
  ^bb0(%arg0: !quantum.qubit):
    // CHECK-NEXT: "quantum.dynamic.yield"(%arg0) : (!quantum.qubit) -> ()
    "quantum.dynamic.yield"(%arg0) : (!quantum.qubit) -> ()
  // CHECK-NEXT: }, {
  }, {
  // CHECK-NEXT: ^bb0(%arg0: !quantum.qubit):
  ^bb0(%arg0: !quantum.qubit):
    // CHECK-NEXT: "quantum.dynamic.yield"(%arg0) : (!quantum.qubit) -> ()
    "quantum.dynamic.yield"(%arg0) : (!quantum.qubit) -> ()
  // CHECK-NEXT: }) : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
  }) : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
}
