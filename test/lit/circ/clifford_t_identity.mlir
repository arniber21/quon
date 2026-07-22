// Clifford+T optimization: stabilizer tableau identity detection (issue #96).
//
// S · S · S · S = I — non-adjacent identity not caught by gate_cancellation
// (S is not self-inverse, so adjacent S·S pairs are not cancelled).
//
// RUN: %clifford-t-opt < %s | FileCheck %s

module {
  // CHECK-NOT: gate_name = "S"
  // CHECK: depth = "0"

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "S"} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.circ.gate"(%0) {clifford = true, depth_contribution = 1 : i64, gate_name = "S"} : (!quantum.qubit) -> !quantum.qubit
    %2 = "quantum.circ.gate"(%1) {clifford = true, depth_contribution = 1 : i64, gate_name = "S"} : (!quantum.qubit) -> !quantum.qubit
    %3 = "quantum.circ.gate"(%2) {clifford = true, depth_contribution = 1 : i64, gate_name = "S"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%3) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "4", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "main"} : () -> ()
}
