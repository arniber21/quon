// Gate cancellation test (issue #18).
//
// RUN: %gate-cancel < %s | FileCheck %s

module {
  // CHECK-NOT: gate_name = "H"
  // CHECK: depth = "0"

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.circ.gate"(%0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%1) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "main"} : () -> ()
}
