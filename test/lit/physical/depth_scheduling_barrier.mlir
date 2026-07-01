// Depth scheduling barrier-offset regression (issue #26).
//
// RUN: %depth-schedule %S/../../../backend/tests/fixtures/device_5q.json < %s | FileCheck %s

module {
  // CHECK: gate_name = "X"
  // CHECK-SAME: schedule_time = 0
  // CHECK: gate_name = "Z"
  // CHECK-SAME: schedule_time = 1

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X", phys_qubit = 0 : i32} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.dynamic.barrier"(%0) : (!quantum.qubit) -> !quantum.qubit
    %2 = "quantum.circ.gate"(%1) {clifford = true, depth_contribution = 1 : i64, gate_name = "Z", phys_qubit = 0 : i32} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%2) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "barrier_offsets"} : () -> ()
}
