// Round-trip test for the quantum.circ dialect (issue #4).
//
// Parses a quantum.circ module, reprints it, and checks the text is stable:
// quantum.circ module -> MLIR text -> re-parsed -> identical. The input below is
// already in MLIR's canonical generic form (sorted attributes, %argN / %N value
// names), so a faithful round-trip reproduces it verbatim.
//
// The %circ-roundtrip tool is mlir_bridge's `circ_roundtrip` example; the lit
// harness (issue #28) builds it and puts it on PATH.
//
// RUN: %circ-roundtrip < %s | FileCheck %s

module {
  // CHECK: "quantum.circ.func"() ({
  "quantum.circ.func"() ({
  // CHECK-NEXT: ^bb0(%arg0: !quantum.qubit):
  ^bb0(%arg0: !quantum.qubit):
    // CHECK-NEXT: %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    // CHECK-NEXT: "quantum.circ.return"(%0) : (!quantum.qubit) -> ()
    "quantum.circ.return"(%0) : (!quantum.qubit) -> ()
  // CHECK-NEXT: }) {clifford = true, depth = "1", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "main"} : () -> ()
  }) {clifford = true, depth = "1", in_qubits = 1 : i64, out_qubits = 1 : i64, sym_name = "main"} : () -> ()
}
