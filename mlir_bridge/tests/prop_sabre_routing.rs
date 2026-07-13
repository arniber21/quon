//! Property tests: SABRE routing preserves statevectors up to layout (issue #118).
//!
//! Post-pass IR (including inserted SWAPs) is simulated in wire-slot order.
//! Equivalence holds if some qubit permutation makes the post statevector match
//! the pre one on **all** probed inputs (n≤4 → at most 24 permutations).
//!
//! Choosing the perm from `|0…0⟩` alone is unsafe: circuits like `H⊗n` make
//! that state permutation-symmetric, so a wrong identity perm can "match".

mod equiv_harness;

use equiv_harness::circuit_spec::sabre_circuit_strategy;
use equiv_harness::extract::extract_func_circuit;
use equiv_harness::lower::{context, lower_func_module, prop_config, read_func_op};
use equiv_harness::sabre_layout::permute_statevector;
use equiv_harness::statevector::{State, TOL, apply_circuit, phase_invariant_distance};
use equiv_harness::{CircuitSpec, GateInst, GateKind};

use mlir_bridge::passes::sabre_routing::{self, SabreCost};
use proptest::prelude::*;

fn linear_topology(n: usize) -> backend::BackendTarget {
    let edges: Vec<(usize, usize)> = (0..n.saturating_sub(1)).map(|i| (i, i + 1)).collect();
    backend::BackendTarget::fixed(
        "linear",
        backend::FixedTarget {
            num_qubits: n,
            topology: backend::ConnectivityGraph::try_from_edges(n, edges).expect("topology"),
            native_gates: vec![
                backend::NativeGate::passthrough("cx", 2),
                backend::NativeGate::passthrough("swap", 2),
            ],
            noise: backend::NoiseModel::default(),
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    )
}

/// Heap's algorithm — all permutations of `0..n`.
fn all_perms(n: u8) -> Vec<Vec<u8>> {
    let mut elems: Vec<u8> = (0..n).collect();
    let mut out = Vec::new();
    fn generate(k: usize, elems: &mut [u8], out: &mut Vec<Vec<u8>>) {
        if k == 1 {
            out.push(elems.to_vec());
            return;
        }
        generate(k - 1, elems, out);
        for i in 0..k - 1 {
            if k.is_multiple_of(2) {
                elems.swap(i, k - 1);
            } else {
                elems.swap(0, k - 1);
            }
            generate(k - 1, elems, out);
        }
    }
    generate(n as usize, &mut elems, &mut out);
    out
}

fn probe_inputs(n: u8) -> Vec<usize> {
    let dim = 1usize << n;
    let mut inputs = vec![0usize];
    let step = (dim / 8).max(1);
    let mut bits = step;
    while bits < dim && inputs.len() < 8 {
        inputs.push(bits);
        bits += step;
    }
    // Always include a few low-weight non-zero basis states so H⊗n symmetry
    // cannot make every permutation look equivalent.
    for extra in [1usize, 2, 4] {
        if extra < dim && !inputs.contains(&extra) {
            inputs.push(extra);
        }
    }
    inputs
}

fn sabre_equiv(pre: &CircuitSpec, post: &CircuitSpec) -> Result<(), String> {
    if pre.width != post.width {
        return Err(format!("width mismatch: {} vs {}", pre.width, post.width));
    }
    let n = pre.width;
    let inputs = probe_inputs(n);
    let mut pairs = Vec::with_capacity(inputs.len());
    for bits in inputs {
        let mut a = State::computational_basis(n, bits);
        let mut b = State::computational_basis(n, bits);
        apply_circuit(&mut a, pre)?;
        apply_circuit(&mut b, post)?;
        pairs.push((bits, a, b));
    }

    for perm in all_perms(n) {
        let ok = pairs.iter().all(|(_, a, b)| {
            let b_p = permute_statevector(b, &perm);
            phase_invariant_distance(a, &b_p) <= TOL
        });
        if ok {
            return Ok(());
        }
    }

    let detail = pairs
        .iter()
        .map(|(bits, a, b)| {
            let best = all_perms(n)
                .iter()
                .map(|p| phase_invariant_distance(a, &permute_statevector(b, p)))
                .fold(f64::INFINITY, f64::min);
            format!("|{bits:0width$b}⟩ best_dist={best}", width = n as usize)
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!(
        "no qubit permutation matches all inputs ({detail})\npre={pre:?}\npost={post:?}"
    ))
}

fn assert_sabre_preserves(spec: &CircuitSpec) -> Result<(), TestCaseError> {
    prop_assert!(spec.width >= 2);
    let ctx = context();
    let module = lower_func_module(&ctx, spec);
    let target = linear_topology(spec.width.max(3) as usize);
    sabre_routing::run_on_module(&ctx, &target, SabreCost::default(), &module);
    let post = extract_func_circuit(read_func_op(&module));
    sabre_equiv(spec, &post).map_err(TestCaseError::fail)
}

proptest! {
    #![proptest_config(prop_config())]

    #[test]
    fn sabre_preserves_statevector_up_to_swaps(spec in sabre_circuit_strategy()) {
        assert_sabre_preserves(&spec)?;
    }

    /// Routing with non-default β / lookahead must still emit connectivity-
    /// respecting SWAPs (statevector equivalence up to layout).
    #[test]
    fn sabre_preserves_with_beta_lookahead(
        spec in sabre_circuit_strategy(),
        beta in prop_oneof![Just(0.0), Just(0.5), Just(10.0)],
        lookahead in prop_oneof![Just(0usize), Just(1), Just(20)],
    ) {
        prop_assert!(spec.width >= 2);
        let ctx = context();
        let module = lower_func_module(&ctx, &spec);
        let target = linear_topology(spec.width.max(3) as usize);
        let cost = SabreCost {
            alpha: 1.0,
            beta,
            gamma: 0.0,
            lookahead,
        };
        sabre_routing::run_on_module(&ctx, &target, cost, &module);
        let post = extract_func_circuit(read_func_op(&module));
        sabre_equiv(&spec, &post).map_err(TestCaseError::fail)?;
    }
}

#[test]
fn seeded_non_adjacent_cnot() {
    let spec = CircuitSpec::new(
        3,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::H, vec![1]),
            GateInst::new(GateKind::H, vec![2]),
            GateInst::new(GateKind::CNOT, vec![0, 2]),
        ],
    );
    assert_sabre_preserves(&spec).expect("non-adjacent CNOT");
}

#[test]
fn seeded_bell_on_linear() {
    let spec = CircuitSpec::new(
        2,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::CNOT, vec![0, 1]),
        ],
    );
    assert_sabre_preserves(&spec).expect("bell");
}
