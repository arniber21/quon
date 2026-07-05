// Measurement deferral test (issue #22).
//
// RUN: %monadic-lower < %s | %measurement-defer | FileCheck %s

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

  "quantum.circ.run"(%meas, %tgt) ({
  ^bb0(%q0: !quantum.qubit, %q1: !quantum.qubit):
    %b = "quantum.circ.measure"(%q0) : (!quantum.qubit) -> !quantum.bit
    %out = "quantum.circ.cond_apply"(%b, %q1) {else_callee = "identity_1", then_callee = "X_1"} : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
    "quantum.circ.yield"(%out) : (!quantum.qubit) -> ()
  }) : (!quantum.qubit, !quantum.qubit) -> ()
}
