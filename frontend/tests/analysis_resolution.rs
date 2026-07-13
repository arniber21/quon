use frontend::analysis::{ResolvedTarget, SymbolKind, analyze_program, format_hover, resolve_at};

#[test]
fn gate_resolves_to_gate_target() {
    let src = "fn g(): Circuit<1, 1, 1, Clifford> = circuit { H @ 0 }\n";
    let analysis = analyze_program(src);
    assert!(
        analysis
            .resolutions
            .entries()
            .any(|(_, t)| { matches!(t, ResolvedTarget::Gate(name) if name == "H") }),
        "H should resolve as gate"
    );
}

#[test]
fn local_binding_resolves_to_symbol() {
    let src = "fn f(): Int = let x = 1 in x\n";
    let analysis = analyze_program(src);
    assert!(analysis.resolutions.entries().any(|(_, t)| {
        matches!(t, ResolvedTarget::Symbol(id) if analysis
                .symbols
                .get(*id)
                .is_some_and(|s| s.name == "x" && s.kind == SymbolKind::LocalBinding))
    }));
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

#[test]
fn local_use_site_hover_shows_inferred_type() {
    let src = "fn f(): Int = let x = 1 in x\n";
    let analysis = analyze_program(src);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let x_use = src.find("in x").expect("x use") + 3;
    let query = resolve_at(&analysis, x_use).expect("resolve x");
    let md = format_hover(&query, &analysis);
    assert!(md.contains("Int"), "hover should show Int: {md}");
}

#[test]
fn hover_on_fn_use_shows_leading_docs() {
    let src = r#"
-- Prepare a Bell pair on two qubits
fn bell_state(): Int = 1

fn use_bell(): Int = bell_state()
"#;
    let analysis = analyze_program(src);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let use_offset = src.rfind("bell_state").expect("use site");
    let query = resolve_at(&analysis, use_offset).expect("resolve bell_state use");
    let md = format_hover(&query, &analysis);
    assert!(
        md.contains("Prepare a Bell pair on two qubits"),
        "hover should include docs: {md}"
    );
    assert!(md.contains("Int"), "hover should include type: {md}");
}

#[test]
fn hover_on_fn_decl_name_shows_leading_docs() {
    let src = "-- Docs for f\nfn f(): Int = 1\n";
    let analysis = analyze_program(src);
    let name_offset = src.find("fn f").expect("fn") + 3;
    let query = resolve_at(&analysis, name_offset).expect("resolve f decl");
    let md = format_hover(&query, &analysis);
    assert!(
        md.contains("Docs for f"),
        "hover on decl should include docs: {md}"
    );
}

#[test]
fn completion_scope_excludes_outer_fn_binding() {
    let src = "fn f(): Int = let x = 1 in x\nfn g(): Int = 0\n";
    let analysis = analyze_program(src);
    let g_body = src.find("fn g").expect("g") + "fn g(): Int = ".len();
    assert!(
        analysis.symbols.resolve_name_at("x", g_body).is_none(),
        "x should not resolve inside g"
    );
}

#[test]
fn type_alias_use_records_resolution() {
    let src = "type T = Int\nfn f(): T = 1\n";
    let analysis = analyze_program(src);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .resolutions
            .entries()
            .any(|(_, t)| matches!(t, ResolvedTarget::TypeAlias(_))),
        "expected type alias resolution entry"
    );
}
