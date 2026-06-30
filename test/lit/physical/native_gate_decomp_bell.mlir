// Native gate decomposition test (issue #24).
//
// RUN: %native-gate-decomp %S/../../../backend/tests/fixtures/device_5q.json < %s | FileCheck %s

module {
  // CHECK-NOT: gate_name = "H"
  // CHECK-NOT: gate_name = "CNOT"
  // CHECK-DAG: gate_name = "cx"
  // CHECK-DAG: gate_name = "rz"
  // CHECK-DAG: gate_name = "sx"
  // CHECK: native_gate = true

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H", native_gate = false} : (!quantum.qubit) -> !quantum.qubit
    %1:2 = "quantum.circ.gate"(%0, %arg1) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT", native_gate = false} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    "quantum.circ.return"(%1#0, %1#1) : (!quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 2 : i64, out_qubits = 2 : i64, sym_name = "bell_state"} : () -> ()
}
