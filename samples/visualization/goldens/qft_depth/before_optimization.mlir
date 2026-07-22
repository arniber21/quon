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
    %8 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %9 = "quantum.circ.gate"(%arg1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %10:2 = "quantum.circ.gate"(%8, %9) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %11 = "quantum.circ.gate"(%10#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %12:2 = "quantum.circ.gate"(%10#0, %11) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %13 = "quantum.circ.gate"(%arg2) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %14:2 = "quantum.circ.gate"(%12#0, %13) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %15 = "quantum.circ.gate"(%14#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %16:2 = "quantum.circ.gate"(%14#0, %15) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %17 = "quantum.circ.gate"(%12#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %18 = "quantum.circ.gate"(%16#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %19:2 = "quantum.circ.gate"(%17, %18) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %20 = "quantum.circ.gate"(%19#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %21:2 = "quantum.circ.gate"(%19#0, %20) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %22 = "quantum.circ.gate"(%21#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %23:2 = "quantum.circ.gate"(%21#0, %22) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %24:2 = "quantum.circ.gate"(%16#0, %23#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %25:2 = "quantum.circ.gate"(%24#0, %24#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %26:2 = "quantum.circ.gate"(%23#0, %25#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %27 = "quantum.circ.gate"(%26#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %28:2 = "quantum.circ.gate"(%26#0, %27) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %29 = "quantum.circ.gate"(%28#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %30:2 = "quantum.circ.gate"(%28#0, %29) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %31 = "quantum.circ.gate"(%30#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %32 = "quantum.circ.gate"(%30#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %33:2 = "quantum.circ.gate"(%25#0, %31) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %34 = "quantum.circ.gate"(%33#1) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %35:2 = "quantum.circ.gate"(%33#0, %34) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %36 = "quantum.circ.gate"(%35#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %37:2 = "quantum.circ.gate"(%35#0, %32) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %38 = "quantum.circ.gate"(%37#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %39:2 = "quantum.circ.gate"(%37#0, %38) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %40 = "quantum.circ.gate"(%39#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %41 = "quantum.circ.gate"(%39#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%41, %40, %36) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "(* (* 4 3) 3)", in_qubits = 3 : i64, out_qubits = 3 : i64, sym_name = "qft_roundtrip__elab0"} : () -> ()
  %4:3 = "quantum.dynamic.unitary_region"(%3#0, %3#1, %3#2) ({
  ^bb0(%arg0: !quantum.qubit, %arg1: !quantum.qubit, %arg2: !quantum.qubit):
    %8 = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %9 = "quantum.circ.gate"(%arg1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %10:2 = "quantum.circ.gate"(%8, %9) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %11 = "quantum.circ.gate"(%10#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %12:2 = "quantum.circ.gate"(%10#0, %11) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %13 = "quantum.circ.gate"(%arg2) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %14:2 = "quantum.circ.gate"(%12#0, %13) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %15 = "quantum.circ.gate"(%14#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %16:2 = "quantum.circ.gate"(%14#0, %15) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %17 = "quantum.circ.gate"(%12#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %18 = "quantum.circ.gate"(%16#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %19:2 = "quantum.circ.gate"(%17, %18) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %20 = "quantum.circ.gate"(%19#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %21:2 = "quantum.circ.gate"(%19#0, %20) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %22 = "quantum.circ.gate"(%21#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %23:2 = "quantum.circ.gate"(%21#0, %22) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %24:2 = "quantum.circ.gate"(%16#0, %23#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %25:2 = "quantum.circ.gate"(%24#0, %24#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %26:2 = "quantum.circ.gate"(%23#0, %25#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "SWAP"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %27 = "quantum.circ.gate"(%26#1) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %28:2 = "quantum.circ.gate"(%26#0, %27) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %29 = "quantum.circ.gate"(%28#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %30:2 = "quantum.circ.gate"(%28#0, %29) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %31 = "quantum.circ.gate"(%30#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %32 = "quantum.circ.gate"(%30#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    %33:2 = "quantum.circ.gate"(%25#0, %31) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %34 = "quantum.circ.gate"(%33#1) {angle = 0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %35:2 = "quantum.circ.gate"(%33#0, %34) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %36 = "quantum.circ.gate"(%35#1) {angle = -0.39269908169872414 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %37:2 = "quantum.circ.gate"(%35#0, %32) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %38 = "quantum.circ.gate"(%37#1) {angle = 0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %39:2 = "quantum.circ.gate"(%37#0, %38) {clifford = true, depth_contribution = 1 : i64, gate_name = "CNOT"} : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
    %40 = "quantum.circ.gate"(%39#1) {angle = -0.78539816339744828 : f64, clifford = false, depth_contribution = 1 : i64, gate_name = "Rz"} : (!quantum.qubit) -> !quantum.qubit
    %41 = "quantum.circ.gate"(%39#0) {clifford = true, depth_contribution = 1 : i64, gate_name = "H"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.circ.return"(%41, %40, %36) : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> ()
  }) {clifford = false, depth = "(* (* 4 3) 3)"} : (!quantum.qubit, !quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit, !quantum.qubit)
  %5 = "quantum.dynamic.measure"(%4#0) : (!quantum.qubit) -> !quantum.bit
  %6 = "quantum.dynamic.measure"(%4#1) : (!quantum.qubit) -> !quantum.bit
  %7 = "quantum.dynamic.measure"(%4#2) : (!quantum.qubit) -> !quantum.bit
}

