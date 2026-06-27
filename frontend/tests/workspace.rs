use frontend::types::Ty;
use quon_core::DepthExpr;

#[test]
fn linear_resources_are_classified_correctly() {
    // The quantum values that must be consumed exactly once.
    assert!(Ty::Qubit.is_linear_resource());
    assert!(Ty::QReg(4).is_linear_resource());
    assert!(
        Ty::Circuit {
            n: 1,
            m: 1,
            d: DepthExpr::Nat(1),
            c: frontend::ast::CliffordClass::Clifford,
        }
        .is_linear_resource()
    );

    // Classical values are unrestricted.
    assert!(!Ty::Int.is_linear_resource());
    assert!(!Ty::Bool.is_linear_resource());
    assert!(!Ty::Bit.is_linear_resource());

    // A `-o` function and a `Q<τ>` computation are reusable, not linear resources: the
    // linearity of `Qubit -o Q<Bit>` is a promise about the *argument*, not the function.
    assert!(
        !Ty::Linear(Box::new(Ty::Qubit), Box::new(Ty::Q(Box::new(Ty::Bit)))).is_linear_resource()
    );
    assert!(!Ty::Q(Box::new(Ty::Bit)).is_linear_resource());

    // Aggregates are linear exactly when they carry a linear resource.
    assert!(Ty::Tuple(vec![Ty::Int, Ty::Qubit]).is_linear_resource());
    assert!(!Ty::Tuple(vec![Ty::Int, Ty::Bool]).is_linear_resource());
    assert!(Ty::list(Ty::Qubit).is_linear_resource());
    assert!(!Ty::list(Ty::Int).is_linear_resource());
}
