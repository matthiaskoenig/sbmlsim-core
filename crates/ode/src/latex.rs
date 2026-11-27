//! LaTeX projection of the shared document math printer.
use super::document_math::Dialect;
#[cfg(test)]
use sbmlsim_document::Ast;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
fn print(a: &Ast, s: &HashMap<String, String>, c: bool) -> Result<String, String> {
    Dialect::Latex.print(a, s, c)
}
#[cfg(test)]
fn lines(a: &Ast, s: &HashMap<String, String>) -> Result<Vec<String>, String> {
    Dialect::Latex.lines(a, s)
}
pub(super) fn symbol(s: &str) -> Result<String, String> {
    Dialect::Latex.symbol(s)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equation_latex_matches_sbmlode_corpus() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../testdata/oracles/equation_maths.json"
        ))
        .unwrap();
        let mut failures = Vec::new();
        for (i, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
            let math = sbmlsim_reader::read_math(case["mathml"].as_str().unwrap())
                .unwrap()
                .unwrap();
            let symbols: HashMap<String, String> =
                serde_json::from_value(case["symbols"].clone()).unwrap();
            for mode in ["math", "condition", "lines"] {
                let actual = if mode == "lines" {
                    lines(&math, &symbols).map(|lines| serde_json::json!(lines))
                } else {
                    print(&math, &symbols, mode == "condition").map(|text| serde_json::json!(text))
                };
                let actual = match actual {
                    Ok(value) => serde_json::json!({"ok":value}),
                    Err(error) => serde_json::json!({"error":error}),
                };
                if actual != case[mode] {
                    failures.push(format!(
                        "case {i} {mode}: ours {}, expected {}",
                        actual.to_string().chars().take(350).collect::<String>(),
                        case[mode].to_string().chars().take(350).collect::<String>()
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} differences:\n{}",
            failures.len(),
            failures
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
