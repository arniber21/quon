//! Call-site signature information for LSP signature help (#174).

use crate::types::Ty;

use super::cursor::partial_ident;
use super::prelude_names::{gate_type, is_quantum_builtin};
use super::{DocumentAnalysis, SymbolKind};

/// A resolved call / gate-application site under the cursor.
#[derive(Debug, Clone)]
pub struct SignatureSite {
    pub label: String,
    pub parameters: Vec<SignatureParam>,
    pub active_parameter: u32,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SignatureParam {
    pub label: String,
    pub documentation: Option<String>,
}

/// Find signature help at `offset`, using lexical call scanning + type lookup.
pub fn signature_site_at(analysis: &DocumentAnalysis, offset: usize) -> Option<SignatureSite> {
    let offset = offset.min(analysis.src.len());

    // Prefer parenthesized call sites: `name(…)` / `name(a, |)`.
    if let Some(site) = call_site_from_parens(&analysis.src, offset) {
        return signature_for_callee(analysis, &site.name, site.active_parameter, CallKind::Paren);
    }

    // Gate / circuit application: `H @ |` or `Rz(theta) @ |`.
    if let Some(site) = gate_app_site(&analysis.src, offset) {
        return signature_for_callee(analysis, &site.name, site.active_parameter, CallKind::At);
    }

    None
}

#[derive(Clone, Copy)]
enum CallKind {
    Paren,
    At,
}

struct LexCall {
    name: String,
    active_parameter: u32,
}

fn call_site_from_parens(src: &str, offset: usize) -> Option<LexCall> {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut i = offset;
    let mut comma_count = 0u32;
    let mut found_open = false;

    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    found_open = true;
                    break;
                }
                depth -= 1;
            }
            b',' if depth == 0 => comma_count += 1,
            b';' | b']' if depth == 0 => return None,
            b'{' | b'[' if depth == 0 => return None,
            _ => {}
        }
    }
    if !found_open {
        return None;
    }

    // Skip whitespace before `(`.
    let mut name_end = i;
    while name_end > 0 && bytes[name_end - 1].is_ascii_whitespace() {
        name_end -= 1;
    }
    // Identifier must end exactly at name_end.
    let mut start = name_end;
    while start > 0 && is_ident_part(bytes[start - 1]) {
        start -= 1;
    }
    if start == name_end {
        return None;
    }
    let name = src.get(start..name_end)?.to_string();
    Some(LexCall {
        name,
        active_parameter: comma_count,
    })
}

fn gate_app_site(src: &str, offset: usize) -> Option<LexCall> {
    let (ident_start, _, _) = partial_ident(src, offset);
    let mut i = ident_start;
    let bytes = src.as_bytes();
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'@' {
        return None;
    }
    // Walk left past `@` and whitespace to the gate / callee expression end.
    let mut end = i - 1;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    // If callee ends with `)`, find matching `(` then the name before it.
    if end > 0 && bytes[end - 1] == b')' {
        let mut depth = 0i32;
        let mut j = end;
        while j > 0 {
            j -= 1;
            match bytes[j] {
                b')' => depth += 1,
                b'(' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let mut name_end = j;
        while name_end > 0 && bytes[name_end - 1].is_ascii_whitespace() {
            name_end -= 1;
        }
        let mut start = name_end;
        while start > 0 && is_ident_part(bytes[start - 1]) {
            start -= 1;
        }
        if start == name_end {
            return None;
        }
        return Some(LexCall {
            name: src.get(start..name_end)?.to_string(),
            active_parameter: 0,
        });
    }
    let mut start = end;
    while start > 0 && is_ident_part(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    Some(LexCall {
        name: src.get(start..end)?.to_string(),
        active_parameter: 0,
    })
}

fn signature_for_callee(
    analysis: &DocumentAnalysis,
    name: &str,
    active_parameter: u32,
    kind: CallKind,
) -> Option<SignatureSite> {
    let ty = lookup_callee_ty(analysis, name)?;
    match kind {
        CallKind::Paren => paren_signature(name, &ty, active_parameter),
        CallKind::At => at_signature(name, &ty, active_parameter),
    }
}

fn lookup_callee_ty(analysis: &DocumentAnalysis, name: &str) -> Option<Ty> {
    if let Some(ty) = gate_type(name) {
        return Some(ty);
    }
    if let Some(scheme) = crate::typecheck::builtins::lookup(name) {
        return Some(scheme.body);
    }
    // In-scope / top-level symbols.
    for sym in &analysis.symbols.symbols {
        if sym.name != name {
            continue;
        }
        if matches!(
            sym.kind,
            SymbolKind::Function | SymbolKind::Parameter | SymbolKind::LocalBinding
        ) && let Some(ref ty) = sym.ty
        {
            return Some(ty.clone());
        }
    }
    if is_quantum_builtin(name) {
        // Fallback label without a precise scheme.
        return Some(Ty::Unit);
    }
    None
}

fn paren_signature(name: &str, ty: &Ty, active_parameter: u32) -> Option<SignatureSite> {
    let (params, ret) = uncurry(ty);
    if params.is_empty() {
        // Nullary / constants: still show `name()` when invoked.
        if matches!(ty, Ty::Fn(_, _) | Ty::Linear(_, _)) {
            // unreachable if uncurry works
        } else if gate_type(name).is_some() {
            // Non-parametric gate typed as Circuit — angle-less; no paren params.
            return None;
        }
    }

    // Parametric gates: `Rz : Float -> Circuit<…>` → one paren param (angle).
    let parameters: Vec<SignatureParam> = if params.is_empty() {
        // `f()` unit application
        vec![SignatureParam {
            label: "()".into(),
            documentation: None,
        }]
    } else {
        params
            .iter()
            .enumerate()
            .map(|(i, p)| SignatureParam {
                label: format!("arg{i}: {p}"),
                documentation: Some(format!("```quon\n{p}\n```")),
            })
            .collect()
    };

    let label = format_paren_label(name, &parameters, &ret);
    let active = active_parameter.min(parameters.len().saturating_sub(1) as u32);
    Some(SignatureSite {
        label,
        parameters,
        active_parameter: active,
        documentation: Some(format!("```quon\n{ty}\n```")),
    })
}

fn at_signature(name: &str, ty: &Ty, _active_parameter: u32) -> Option<SignatureSite> {
    let qubit_ty = qubit_arg_ty(ty);
    let param = SignatureParam {
        label: format!("qubits: {qubit_ty}"),
        documentation: Some(format!(
            "Target qubit index or register for `{name}` (`{qubit_ty}`)."
        )),
    };
    let label = format!("{name} @ {}", param.label);
    Some(SignatureSite {
        label,
        parameters: vec![param],
        active_parameter: 0,
        documentation: Some(format!("```quon\n{ty}\n```")),
    })
}

fn qubit_arg_ty(ty: &Ty) -> String {
    let circuit = match ty {
        Ty::Fn(_, ret) | Ty::Linear(_, ret) => ret.as_ref(),
        other => other,
    };
    match circuit {
        Ty::Circuit { n, .. } => {
            if matches!(n, quon_core::DepthExpr::Nat(1)) {
                "Qubit | index".to_string()
            } else {
                format!("QReg<{n}> | indices")
            }
        }
        _ => "qubits".to_string(),
    }
}

fn uncurry(ty: &Ty) -> (Vec<Ty>, Ty) {
    let mut params = Vec::new();
    let mut cur = ty.clone();
    loop {
        match cur {
            Ty::Fn(a, b) | Ty::Linear(a, b) => {
                params.push(*a);
                cur = *b;
            }
            other => return (params, other),
        }
    }
}

fn format_paren_label(name: &str, params: &[SignatureParam], ret: &Ty) -> String {
    if params.len() == 1 && params[0].label == "()" {
        return format!("{name}() -> {ret}");
    }
    let inner = params
        .iter()
        .map(|p| p.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({inner}) -> {ret}")
}

fn is_ident_part(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{analyze_program, cursor_at};

    fn site(src: &str) -> Option<SignatureSite> {
        let offset = cursor_at(src, "/*cursor*/");
        let clean = src.replace("/*cursor*/", "");
        let a = analyze_program(&clean);
        signature_site_at(&a, offset)
    }

    #[test]
    fn map_call_active_second_arg() {
        let src = "fn f(xs: List<Int>): List<Int> = map(fn(x) -> x, /*cursor*/xs)\n";
        let Some(s) = site(src) else {
            panic!("expected signature");
        };
        assert!(s.label.starts_with("map("), "{}", s.label);
        assert_eq!(s.active_parameter, 1);
        assert!(s.parameters.len() >= 2);
    }

    #[test]
    fn gate_at_qubit() {
        let src = "fn c(): Circuit<1,1,1,Clifford> = circuit { H @/*cursor*/0 }\n";
        let Some(s) = site(src) else {
            panic!("expected signature");
        };
        assert!(s.label.contains('@'), "{}", s.label);
        assert!(s.label.contains('H'), "{}", s.label);
    }
}
