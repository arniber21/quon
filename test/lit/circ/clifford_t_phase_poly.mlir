// Clifford+T optimization: phase polynomial T-count reduction (issue #96).
//
// T(0), CNOT(0,1), T(0) → both T on parity {0}, merge to S — T-count 2→0.
// Non-adjacent reduction not possible with peephole gate_cancellation.
//
// RUN: %clifford-t-opt < %s | FileCheck %s

module {
  // CHECK-NOT: gate_name = "T"
  // CHECK: gate_name = "S"
  // CHECK: gate_name = "CNOT"

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = false, depth_contribution = 1 : i64, gate_name = "T"} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.circ.gate"(%0, %arg1) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %2 = "quantum.circ.gate"(%1#0) {clifford = false, depth_contribution = 1 : i64, gate_name = "T"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%2, %1#1) : (!quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "3", in_qubits = 2 : i64, out_qubits = 2 : i64, sym_name = "main"} : () -> ()
}
