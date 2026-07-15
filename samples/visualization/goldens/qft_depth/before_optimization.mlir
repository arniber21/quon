module {
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%0, %arg1, %1) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "prep_101"} : () -> ()
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %0 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %1 = "quantum.circ.gate"(%arg1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %2:2 = "quantum.circ.gate"(%0, %1) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %3 = "quantum.circ.gate"(%2#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %4:2 = "quantum.circ.gate"(%2#0, %3) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %5 = "quantum.circ.gate"(%arg2) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %6:2 = "quantum.circ.gate"(%4#0, %5) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %7 = "quantum.circ.gate"(%6#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %8:2 = "quantum.circ.gate"(%6#0, %7) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %9 = "quantum.circ.gate"(%4#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %10 = "quantum.circ.gate"(%8#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %11:2 = "quantum.circ.gate"(%9, %10) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %12 = "quantum.circ.gate"(%11#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %13:2 = "quantum.circ.gate"(%11#0, %12) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %14 = "quantum.circ.gate"(%13#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %15:2 = "quantum.circ.gate"(%13#0, %14) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %16:2 = "quantum.circ.gate"(%8#0, %15#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %17:2 = "quantum.circ.gate"(%16#0, %16#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %18:2 = "quantum.circ.gate"(%15#0, %17#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %19 = "quantum.circ.gate"(%18#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %20:2 = "quantum.circ.gate"(%18#0, %19) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %21 = "quantum.circ.gate"(%20#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %22:2 = "quantum.circ.gate"(%20#0, %21) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %23 = "quantum.circ.gate"(%22#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %24 = "quantum.circ.gate"(%22#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %25:2 = "quantum.circ.gate"(%17#0, %23) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %26 = "quantum.circ.gate"(%25#1) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %27:2 = "quantum.circ.gate"(%25#0, %26) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %28 = "quantum.circ.gate"(%27#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %29:2 = "quantum.circ.gate"(%27#0, %24) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %30 = "quantum.circ.gate"(%29#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %31:2 = "quantum.circ.gate"(%29#0, %30) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %32 = "quantum.circ.gate"(%31#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %33 = "quantum.circ.gate"(%31#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%33, %32, %28) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
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

