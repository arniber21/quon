use frontend::types::Ty;

#[test]
fn linear_types_are_classified_correctly() {
    assert!(Ty::Qubit.is_linear());
    assert!(Ty::QReg(4).is_linear());
    assert!(!Ty::Int.is_linear());
    assert!(!Ty::Bool.is_linear());
}
