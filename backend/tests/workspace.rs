use backend::target::NoiseModel;

#[test]
fn noise_model_defaults_to_empty_maps() {
    let noise = NoiseModel::default();
    assert!(noise.single_qubit_fidelity.is_empty());
    assert!(noise.two_qubit_fidelity.is_empty());
    assert!(noise.t1_us.is_empty());
    assert!(noise.t2_us.is_empty());
    assert!(noise.readout_error.is_empty());
}
