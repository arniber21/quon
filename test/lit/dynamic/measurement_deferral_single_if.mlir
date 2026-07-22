// Measurement deferral test (issue #22).
//
// The staging dialect was collapsed (#213 / ADR-0037): `lower` now emits
// `quantum.dynamic` IR directly, so this fixture is already in the dynamic
// form the old `quantum.circ.run` staging + monadic-lowering pass produced.
//
// RUN: %measurement-defer < %s | FileCheck %s

module {
  // CHECK-NOT: "quantum.dynamic.if"
  // CHECK: "quantum.dynamic.unitary_region"
  // CHECK: gate_name = "CNOT"
  // CHECK: "quantum.dynamic.measure"

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%0) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "1", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "X_1"} : () -> ()

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    "quantum.circ.return"(%arg0) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "0", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "identity_1"} : () -> ()

  %meas = "test.qubit"() : () -> !quantum.qubit
  %tgt = "test.qubit"() : () -> !quantum.qubit

  %b = "quantum.dynamic.measure"(%meas) : (!quantum.qubit) -> !quantum.bit
  %out = "quantum.dynamic.if"(%b, %tgt) ({
  ^bb0(%q0: !quantum.qubit):
    %x = "quantum.circ.gate"(%q0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.dynamic.yield"(%x) : (!quantum.qubit) -> ()
  }, {
  ^bb0(%q1: !quantum.qubit):
    "quantum.dynamic.yield"(%q1) : (!quantum.qubit) -> ()
  }) : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
}
