module {
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%0, %arg1, %1) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "prep_101"} : () -> ()
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    "quantum.circ.return"(%arg0, %arg1, %arg2) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "(* (* 4 3) 3)", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "qft_roundtrip__elab0"} : () -> ()
  "quantum.circ.run"() ({
    %0:3 = "quantum.circ.qreg"() {count = 3 : i64} : () -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
    %1:3 = "quantum.circ.apply"(%0#0, %0#1, %0#2) {callee = "prep_101"} : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
    %2:3 = "quantum.circ.apply"(%1#0, %1#1, %1#2) {callee = "qft_roundtrip__elab0"} : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
    %3 = "quantum.circ.measure"(%2#0) : (!quantum.qubit) -> !quantum.bit
    %4 = "quantum.circ.measure"(%2#1) : (!quantum.qubit) -> !quantum.bit
    %5 = "quantum.circ.measure"(%2#2) : (!quantum.qubit) -> !quantum.bit
    "quantum.circ.yield"(%3, %4, %5) : (!quantum.bit, !quantum.bit, !quantum.bit) -> ()
  }) : () -> ()
}

