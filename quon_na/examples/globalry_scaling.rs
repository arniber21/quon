//! Benchmark harness for GlobalRy echo-refocus scaling (issue #322).
//!
//! Measures the action count, layer count, and serialized schedule JSON size
//! for N = 2..16 independent single-qubit `ry(theta)` rotations, one per atom,
//! to quantify the O(N²) scaling ceiling imposed by the Hahn-echo refocus
//! sequence (`push_global_ry_with_refocus`, issue #298).
//!
//! Each `ry(theta)` decomposes to one `GlobalRy` raster. With N atoms trapped,
//! the refocus sequence for a single rotation emits `2 + 2*(N-1)` actions
//! (two `GlobalRy` half-pulses + `N-1` `Rz(pi)` + `N-1` `Rz(-pi)` echo pulses).
//! N independent rotations → `N * (2 + 2*(N-1)) = 2*N²` total actions.
//! Since `interleave_local_gates` gives each action its own `ScheduleLayer`,
//! that is also `2*N²` layers/cycles.
//!
//! Run: `cargo run -p quon_na --example globalry_scaling --features mlir`
//!
//! This is a deterministic structural benchmark — no wall-clock timing (which
//! would be flaky in CI). The measured quantities (action count, layer count,
//! JSON byte size) are exact and reproducible.

use serde::Serialize;

use quon_na::{AtomId, LocalGateKind, NeutralAtomAction, ScheduleLayer};

/// Replicate `pipeline::push_global_ry_with_refocus` exactly — it is a pure
/// function, but private in the pipeline module. The logic and derivation are
/// identical; see that function's doc comment for the Hahn-echo proof.
fn push_global_ry_with_refocus(
    bucket: &mut Vec<NeutralAtomAction>,
    atom: AtomId,
    theta: f64,
    all_atoms: &[AtomId],
    duration_us: u64,
) {
    let bystanders: Vec<AtomId> = all_atoms.iter().copied().filter(|&a| a != atom).collect();
    if bystanders.is_empty() {
        bucket.push(NeutralAtomAction::GlobalRy {
            theta_rad: theta,
            duration_us,
        });
        return;
    }

    let half = theta / 2.0;
    bucket.push(NeutralAtomAction::GlobalRy {
        theta_rad: half,
        duration_us,
    });
    for &bystander in &bystanders {
        bucket.push(NeutralAtomAction::LocalGate {
            atom: bystander,
            gate: LocalGateKind::Rz(std::f64::consts::PI),
            duration_us,
        });
    }
    bucket.push(NeutralAtomAction::GlobalRy {
        theta_rad: half,
        duration_us,
    });
    for &bystander in &bystanders {
        bucket.push(NeutralAtomAction::LocalGate {
            atom: bystander,
            gate: LocalGateKind::Rz(-std::f64::consts::PI),
            duration_us,
        });
    }
}

/// One row of the benchmark output table.
#[derive(Debug, Clone, Serialize)]
struct BenchmarkRow {
    n_atoms: u32,
    n_rotations: u32,
    total_actions: u64,
    total_layers: u64,
    global_ry_count: u64,
    local_rz_count: u64,
    json_size_bytes: usize,
    /// Expected actions: N * (2 + 2*(N-1)) = 2*N²
    expected_actions: u64,
    /// Expected layers: same as actions (one action per layer)
    expected_layers: u64,
}

fn build_schedule_for_n(n: u32) -> Vec<ScheduleLayer> {
    let atoms: Vec<AtomId> = (0..n).map(AtomId).collect();
    let duration_us = 1; // structural benchmark — duration irrelevant to counts

    // N independent ry rotations, one per atom, each decomposed to a GlobalRy
    // with refocus. Each rotation gets its own set of layers (matching
    // interleave_local_gates' one-action-per-layer policy).
    let mut all_actions = Vec::new();
    for &atom in &atoms {
        // Distinct angle per atom to prevent any accidental deduplication
        let theta = 0.1 + 0.01 * f64::from(atom.0);
        push_global_ry_with_refocus(&mut all_actions, atom, theta, &atoms, duration_us);
    }

    // One action per layer (matches interleave_local_gates)
    all_actions
        .into_iter()
        .enumerate()
        .map(|(i, action)| ScheduleLayer {
            cycle: i as u32,
            actions: vec![action],
        })
        .collect()
}

fn count_actions(layers: &[ScheduleLayer]) -> (u64, u64, u64) {
    let mut total = 0u64;
    let mut global_ry = 0u64;
    let mut local_rz = 0u64;
    for layer in layers {
        for action in &layer.actions {
            total += 1;
            match action {
                NeutralAtomAction::GlobalRy { .. } => global_ry += 1,
                NeutralAtomAction::LocalGate {
                    gate: LocalGateKind::Rz(_),
                    ..
                } => local_rz += 1,
                _ => {}
            }
        }
    }
    (total, global_ry, local_rz)
}

fn main() {
    println!("# GlobalRy echo-refocus scaling benchmark (issue #322)");
    println!();
    println!("Measures action count, layer/cycle count, and serialized schedule JSON size");
    println!("for N=2..16 independent single-qubit ry rotations, one per trapped atom.");
    println!();
    println!("Each ry(theta) decomposes to one GlobalRy raster. With N atoms, the Hahn-echo");
    println!("refocus sequence emits 2 + 2*(N-1) actions per rotation. N independent rotations");
    println!("→ N*(2 + 2*(N-1)) = 2*N² total actions and layers (one action per layer).");
    println!();
    println!(
        "| N atoms | N rotations | Total actions | Total layers | GlobalRy count | LocalRz count | JSON size (bytes) | Expected actions | Expected layers |"
    );
    println!(
        "|--------:|------------:|--------------:|-------------:|---------------:|--------------:|------------------:|-----------------:|----------------:|"
    );

    let mut rows = Vec::new();

    for n in 2u32..=16 {
        let layers = build_schedule_for_n(n);
        let (total_actions, global_ry_count, local_rz_count) = count_actions(&layers);
        let total_layers = layers.len() as u64;

        // Serialize to JSON to measure size
        let schedule_json = serde_json::to_string(&layers).unwrap_or_default();
        let json_size_bytes = schedule_json.len();

        let expected_actions = u64::from(n) * (2 + 2 * (u64::from(n) - 1));
        let expected_layers = expected_actions;

        let row = BenchmarkRow {
            n_atoms: n,
            n_rotations: n,
            total_actions,
            total_layers,
            global_ry_count,
            local_rz_count,
            json_size_bytes,
            expected_actions,
            expected_layers,
        };
        rows.push(row);

        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            n,
            n,
            total_actions,
            total_layers,
            global_ry_count,
            local_rz_count,
            json_size_bytes,
            expected_actions,
            expected_layers,
        );
    }

    println!();
    println!("## Verification: actual vs expected");
    println!();
    let all_match = rows
        .iter()
        .all(|r| r.total_actions == r.expected_actions && r.total_layers == r.expected_layers);
    if all_match {
        println!("✓ All measured counts match the 2*N² formula exactly.");
    } else {
        println!("✗ MISMATCH: some counts deviate from the expected formula.");
        for r in &rows {
            if r.total_actions != r.expected_actions || r.total_layers != r.expected_layers {
                println!(
                    "  N={}: actions {} vs expected {}, layers {} vs expected {}",
                    r.n_atoms,
                    r.total_actions,
                    r.expected_actions,
                    r.total_layers,
                    r.expected_layers,
                );
            }
        }
    }

    // Extrapolation table
    println!();
    println!("## Extrapolation to larger N");
    println!();
    println!(
        "| N atoms | Predicted actions (2*N²) | Predicted layers | Approx JSON size (KB, linear fit) |"
    );
    println!(
        "|--------:|------------------------:|-----------------:|--------------------------------:|"
    );

    // Linear fit: JSON size ≈ a * N² + b (since layers = 2*N², JSON grows with it)
    // Use the last data point to estimate per-action overhead
    let last = match rows.last() {
        Some(r) => r,
        None => {
            eprintln!("no benchmark rows produced");
            return;
        }
    };
    let bytes_per_action = last.json_size_bytes as f64 / last.total_actions as f64;

    for &n in &[20u32, 32, 50, 64, 100, 128, 256] {
        let predicted_actions = 2u64 * u64::from(n) * u64::from(n);
        let predicted_layers = predicted_actions;
        let predicted_json_kb = (predicted_actions as f64 * bytes_per_action) / 1024.0;
        println!(
            "| {} | {} | {} | {:.1} |",
            n, predicted_actions, predicted_layers, predicted_json_kb,
        );
    }

    // Emit CSV for downstream tooling
    println!();
    println!("## CSV output");
    println!();
    println!(
        "n_atoms,n_rotations,total_actions,total_layers,global_ry_count,local_rz_count,json_size_bytes,expected_actions,expected_layers"
    );
    for r in &rows {
        println!(
            "{},{},{},{},{},{},{},{},{}",
            r.n_atoms,
            r.n_rotations,
            r.total_actions,
            r.total_layers,
            r.global_ry_count,
            r.local_rz_count,
            r.json_size_bytes,
            r.expected_actions,
            r.expected_layers,
        );
    }
}
