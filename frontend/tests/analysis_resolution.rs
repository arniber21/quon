use frontend::analysis::{ResolvedTarget, SymbolKind, analyze_program};

#[test]
fn gate_resolves_to_gate_target() {
    let src = "fn g(): Circuit<1, 1, 1, Clifford> = circuit { H @ 0 }\n";
    let analysis = analyze_program(src);
    assert!(
        analysis.resolutions.entries().any(|(_, t)| {
            matches!(t, ResolvedTarget::Gate(name) if name == "H")
        }),
        "H should resolve as gate"
    );
}

#[test]
fn local_binding_resolves_to_symbol() {
    let src = "fn f(): Int = let x = 1 in x\n";
    let analysis = analyze_program(src);
    assert!(
        analysis.resolutions.entries().any(|(_, t)| {
            matches!(t, ResolvedTarget::Symbol(id) if analysis
                .symbols
                .get(*id)
                .is_some_and(|s| s.name == "x" && s.kind == SymbolKind::LocalBinding))
        })
    );
}

#[test]
fn unbound_ident_has_no_resolution() {
    let src = "fn f(): Int = unknown_var\n";
    let analysis = analyze_program(src);
    assert!(!analysis.diagnostics.is_empty());
    assert!(
        !analysis
            .resolutions
            .entries()
            .any(|(_, t)| matches!(t, ResolvedTarget::Symbol(_)))
    );
}
