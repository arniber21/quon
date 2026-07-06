// Rotation merging lit test (issue #19).
//
// RUN: %rotation-merge < %s | FileCheck %s

module {
  // CHECK: angle = 8.000000e-01
  // CHECK-NOT: angle = 5.000000e-01

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {angle = 0.5 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.circ.gate"(%0) {angle = 0.3 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%1) : (!quantum.qubit) -> ()
  }) {clifford = false, depth = "2", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "main"} : () -> ()
}
