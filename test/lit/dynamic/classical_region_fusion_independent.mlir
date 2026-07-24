// Classical region fusion — independent-condition case (issue #97, PRD story 28).
//
// Distinct from the same-condition fixture in `classical_region_fusion.mlir`
// (#23): there the two `if`s share one predicate and merge into a single `if`
// with parallel regions. Here the `if`s branch on *different*, independent
// condition bits (`%b1` from measuring `%m0`, `%b2` from measuring `%m1`) and
// act on disjoint target qubits, so a same-condition merge would lose the
// independent branching. The pass fuses them with the **nested-if** shape: one
// outer `if` on `%b1` wraps the X body, and an inner `if` on `%b2` (materialized
// in both outer branches, to keep all four condition combinations reachable)
// wraps the Z body. Only the outer `if` crosses the classical/quantum boundary,
// so the crossing count drops from two to one; the two inner `if`s live inside
// the quantum region.
//
// This is a separate lit file (not appended to `classical_region_fusion.mlir`)
// because each lit RUN parses a single module: combining the two fixtures in
// one module would leave the same-condition merged `if` adjacent to the
// independent `if` and let the pass re-fuse across the cases.
//
// RUN: %classical-region-fuse < %s | FileCheck %s

module {
  // The walk below pins the nested-if shape exactly. A single outer `if`
  // threads *both* target qubits, so its result binding carries the multi-result
  // `:2` suffix; the two original sibling `if`s (one qubit each) are gone.
  //   outer `if` (:2) -> X body -> inner `if` -> Z body -> inner `if` -> Z body
  // The inner `if`s sit inside the outer region, so only the outer `if` is a
  // boundary crossing: one outer + two inner, then no more `if`s — i.e. the
  // crossing count dropped from two to one.
  // CHECK: :2 = "quantum.dynamic.if"
  // CHECK: gate_name = "X"
  // CHECK: "quantum.dynamic.if"
  // CHECK: gate_name = "Z"
  // CHECK: "quantum.dynamic.if"
  // CHECK: gate_name = "Z"
  // CHECK-NOT: "quantum.dynamic.if"

  %m0 = "test.qubit"() : () -> !quantum.qubit
  %m1 = "test.qubit"() : () -> !quantum.qubit
  %q0 = "test.qubit"() : () -> !quantum.qubit
  %q1 = "test.qubit"() : () -> !quantum.qubit

  %b1 = "quantum.dynamic.measure"(%m0) : (!quantum.qubit) -> !quantum.bit
  %b2 = "quantum.dynamic.measure"(%m1) : (!quantum.qubit) -> !quantum.bit

  %a = "quantum.dynamic.if"(%b1, %q0) ({
  ^bb0(%arg0: !quantum.qubit):
    %x = "quantum.circ.gate"(%arg0) {clifford = true, depth_contribution = 1 : i64, gate_name = "X"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.dynamic.yield"(%x) : (!quantum.qubit) -> ()
  }, {
  ^bb0(%arg1: !quantum.qubit):
    "quantum.dynamic.yield"(%arg1) : (!quantum.qubit) -> ()
  }) : (!quantum.bit, !quantum.qubit) -> !quantum.qubit

  "quantum.dynamic.if"(%b2, %q1) ({
  ^bb0(%arg2: !quantum.qubit):
    %z = "quantum.circ.gate"(%arg2) {clifford = true, depth_contribution = 1 : i64, gate_name = "Z"} : (!quantum.qubit) -> !quantum.qubit
    "quantum.dynamic.yield"(%z) : (!quantum.qubit) -> ()
  }, {
  ^bb0(%arg3: !quantum.qubit):
    "quantum.dynamic.yield"(%arg3) : (!quantum.qubit) -> ()
  }) : (!quantum.bit, !quantum.qubit) -> !quantum.qubit
}
