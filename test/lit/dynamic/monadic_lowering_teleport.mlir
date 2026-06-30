// Monadic lowering test for teleport (issue #17).
//
// RUN: %monadic-lower < %s | FileCheck %s

module {
  // CHECK-NOT: quantum.circ.run
  // CHECK: "quantum.dynamic.measure"
  // CHECK: "quantum.dynamic.measure"
  // CHECK: "quantum.dynamic.if"
  // CHECK: "quantum.dynamic.if"
  // CHECK: "quantum.dynamic.unitary_region"

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %1:2 = "quantum.circ.gate"(%0, %arg1) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    "quantum.circ.return"(%1#0, %1#1) : (!quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 2 : i64, out_qubits = 2 : i64, sym_name = "bell_state"} : () -> ()

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit):
    %0:2 = "quantum.circ.gate"(%arg0, %arg1) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %1 = "quantum.circ.gate"(%0#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%1, %0#1) : (!quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 2 : i64, out_qubits = 2 : i64, sym_name = "adjoint_bell"} : () -> ()

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%0) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "1", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "X_1"} : () -> ()

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "Z"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%0) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "1", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "Z_1"} : () -> ()

  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit):
    "quantum.circ.return"(%arg0) : (!quantum.qubit) -> ()
  }) {clifford = true, depth = "0", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "identity_1"} : () -> ()

  %msg = "test.qubit"() : () -> !quantum.qubit
  %alice = "test.qubit"() : () -> !quantum.qubit
  %bob = "test.qubit"() : () -> !quantum.qubit

  "quantum.circ.run"(%msg, %alice, %bob) ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %ent:2 = "quantum.circ.apply"(%arg1, %arg2) {callee = "bell_state"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %unent:2 = "quantum.circ.apply"(%arg0, %ent#0) {callee = "adjoint_bell"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %x = "quantum.circ.measure"(%unent#0) : (!quantum.qubit) -> !quantum.bit
    %z = "quantum.circ.measure"(%unent#1) : (!quantum.qubit) -> !quantum.bit
    %b2 = "quantum.circ.cond_apply"(%x, %ent#1) {else_callee = "identity_1", then_callee = "X_1"} : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
    %b3 = "quantum.circ.cond_apply"(%z, %b2) {else_callee = "identity_1", then_callee = "Z_1"} : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
    "quantum.circ.yield"(%b3) : (!quantum.qubit) -> ()
  }) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
}
