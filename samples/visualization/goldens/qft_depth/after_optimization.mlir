module {
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %8 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    %9 = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%8, %arg1, %9) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "prep_101"} : () -> ()
  %0 = "test.qubit"() : () -> !quantum.qubit
  %1 = "test.qubit"() : () -> !quantum.qubit
  %2 = "test.qubit"() : () -> !quantum.qubit
  %3:3 = "quantum.dynamic.unitary_region"(%0, %1, %2) ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %8 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    %9 = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%8, %arg1, %9) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2"} : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    "quantum.circ.return"(%arg0, %arg1, %arg2) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "(* (* 4 3) 3)", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "qft_roundtrip__elab0"} : () -> ()
  %4:3 = "quantum.dynamic.unitary_region"(%3#0, %3#1, %3#2) ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    "quantum.circ.return"(%arg0, %arg1, %arg2) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "(* (* 4 3) 3)"} : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
  %5 = "quantum.dynamic.measure"(%4#0) : (!quantum.qubit) -> !quantum.bit
  %6 = "quantum.dynamic.measure"(%4#1) : (!quantum.qubit) -> !quantum.bit
  %7 = "quantum.dynamic.measure"(%4#2) : (!quantum.qubit) -> !quantum.bit
}

