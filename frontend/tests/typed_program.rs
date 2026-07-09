use frontend::{TypedProgram, analyze_program};

#[test]
fn typed_program_from_clean_analysis() {
    let src = "fn f(): Int = 1\n";
    let analysis = analyze_program(src);
    let typed = TypedProgram::from_analysis(&analysis).expect("typed program");
    assert!(typed.fn_types.contains_key("f"));
    assert!(analysis.diagnostics.is_empty());
}

#[test]
fn typed_program_none_on_parse_error() {
    let analysis = analyze_program("fn broken\n");
    assert!(TypedProgram::from_analysis(&analysis).is_none());
}
