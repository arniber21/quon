module {
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %6 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    %7 = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%6, %arg1, %7) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "prep_101"} : () -> ()
  %0:3 = "quantum.dynamic.alloc"() : () -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
  %1:3 = "quantum.dynamic.unitary_region"(%0#0, %0#1, %0#2) ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %6 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    %7 = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%6, %arg1, %7) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = true, depth = "2"} : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
  "quantum.circ.func"() ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %6 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %7 = "quantum.circ.gate"(%arg1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %8:2 = "quantum.circ.gate"(%6, %7) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %9 = "quantum.circ.gate"(%8#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %10:2 = "quantum.circ.gate"(%8#0, %9) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %11 = "quantum.circ.gate"(%arg2) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %12:2 = "quantum.circ.gate"(%10#0, %11) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %13 = "quantum.circ.gate"(%12#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %14:2 = "quantum.circ.gate"(%12#0, %13) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %15 = "quantum.circ.gate"(%10#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %16 = "quantum.circ.gate"(%14#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %17:2 = "quantum.circ.gate"(%15, %16) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %18 = "quantum.circ.gate"(%17#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %19:2 = "quantum.circ.gate"(%17#0, %18) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %20 = "quantum.circ.gate"(%19#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %21:2 = "quantum.circ.gate"(%19#0, %20) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %22:2 = "quantum.circ.gate"(%14#0, %21#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %23:2 = "quantum.circ.gate"(%22#0, %22#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %24:2 = "quantum.circ.gate"(%21#0, %23#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %25 = "quantum.circ.gate"(%24#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %26:2 = "quantum.circ.gate"(%24#0, %25) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %27 = "quantum.circ.gate"(%26#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %28:2 = "quantum.circ.gate"(%26#0, %27) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %29 = "quantum.circ.gate"(%28#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %30 = "quantum.circ.gate"(%28#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %31:2 = "quantum.circ.gate"(%23#0, %29) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %32 = "quantum.circ.gate"(%31#1) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %33:2 = "quantum.circ.gate"(%31#0, %32) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %34 = "quantum.circ.gate"(%33#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %35:2 = "quantum.circ.gate"(%33#0, %30) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %36 = "quantum.circ.gate"(%35#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %37:2 = "quantum.circ.gate"(%35#0, %36) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %38 = "quantum.circ.gate"(%37#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %39 = "quantum.circ.gate"(%37#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%39, %38, %34) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "(* (* 4 3) 3)", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "qft_roundtrip__elab0"} : () -> ()
  %2:3 = "quantum.dynamic.unitary_region"(%1#0, %1#1, %1#2) ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %6 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %7 = "quantum.circ.gate"(%arg1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %8:2 = "quantum.circ.gate"(%6, %7) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %9 = "quantum.circ.gate"(%8#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %10:2 = "quantum.circ.gate"(%8#0, %9) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %11 = "quantum.circ.gate"(%arg2) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %12:2 = "quantum.circ.gate"(%10#0, %11) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %13 = "quantum.circ.gate"(%12#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %14:2 = "quantum.circ.gate"(%12#0, %13) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %15 = "quantum.circ.gate"(%10#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %16 = "quantum.circ.gate"(%14#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %17:2 = "quantum.circ.gate"(%15, %16) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %18 = "quantum.circ.gate"(%17#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %19:2 = "quantum.circ.gate"(%17#0, %18) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %20 = "quantum.circ.gate"(%19#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %21:2 = "quantum.circ.gate"(%19#0, %20) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %22:2 = "quantum.circ.gate"(%14#0, %21#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %23:2 = "quantum.circ.gate"(%22#0, %22#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %24:2 = "quantum.circ.gate"(%21#0, %23#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %25 = "quantum.circ.gate"(%24#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %26:2 = "quantum.circ.gate"(%24#0, %25) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %27 = "quantum.circ.gate"(%26#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %28:2 = "quantum.circ.gate"(%26#0, %27) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %29 = "quantum.circ.gate"(%28#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %30 = "quantum.circ.gate"(%28#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %31:2 = "quantum.circ.gate"(%23#0, %29) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %32 = "quantum.circ.gate"(%31#1) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %33:2 = "quantum.circ.gate"(%31#0, %32) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %34 = "quantum.circ.gate"(%33#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %35:2 = "quantum.circ.gate"(%33#0, %30) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %36 = "quantum.circ.gate"(%35#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %37:2 = "quantum.circ.gate"(%35#0, %36) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %38 = "quantum.circ.gate"(%37#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %39 = "quantum.circ.gate"(%37#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%39, %38, %34) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "(* (* 4 3) 3)"} : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
  %3 = "quantum.dynamic.measure"(%2#0) : (!quantum.qubit) -> !quantum.bit
  %4 = "quantum.dynamic.measure"(%2#1) : (!quantum.qubit) -> !quantum.bit
  %5 = "quantum.dynamic.measure"(%2#2) : (!quantum.qubit) -> !quantum.bit
}

