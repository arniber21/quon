use quonfmt::format_str;

#[test]
fn lambda_as_compose_operand_keeps_parens() {
    let src = "fn f(): Int = (fn() -> a) |> b\n";
    let f1 = format_str(src).unwrap();
    let f2 = format_str(&f1).unwrap();
    assert_eq!(f1, f2);
    assert!(
        f1.contains("(fn()"),
        "expected parens around lambda compose operand:\n{f1}"
    );
}

#[test]
fn return_as_compose_operand_keeps_parens() {
    let src = "fn f(): Int = (return a) |> b\n";
    let f1 = format_str(src).unwrap();
    let f2 = format_str(&f1).unwrap();
    assert_eq!(f1, f2);
    assert!(
        f1.contains("(return "),
        "expected parens around return compose operand:\n{f1}"
    );
}

#[test]
fn mul_of_lambda_compose_keeps_parens_across_break() {
    let src = "fn f(): Int = a * ((fn() -> b) |> c)\n";
    let f1 = format_str(src).unwrap();
    let f2 = format_str(&f1).unwrap();
    assert_eq!(f1, f2, "f1:\n{f1}\nf2:\n{f2}");
}

#[test]
fn overwidth_pipe_before_match_breaks() {
    let src = "fn c(): List<QReg<o>> = fn((_, f)) -> (if x then true else 47.25) |> controlled(true)(let _ = match return 16.75 {\n(q, _) => if () then 22.25 else 59977,\n(26, _) => (ox, 32461)\n} in ox)\n";
    let f1 = format_str(src).unwrap();
    let f2 = format_str(&f1).unwrap();
    assert_eq!(f1, f2);
    let line0 = f1.lines().next().unwrap();
    assert!(line0.len() <= 100, "line0 len {}: {line0}", line0.len());
}
