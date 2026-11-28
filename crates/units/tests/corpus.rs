//! The units of the corpus of sbmlsim (`testdata/units_corpus.json`, written by
//! `scripts/scan_fixtures.py` of sbmlsim from `sbmlsim.units.ureg`): every expression parses to
//! the factor pint gives it to its root units, within 1e-15 relative, and to its dimensions.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sbmlsim_units::{DIMENSIONS, Registry};
use serde_json::Value;

/// The entries of the corpus: the expression, the factor and the dimensions of pint.
fn corpus() -> Vec<(String, f64, BTreeMap<String, f64>)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/units_corpus.json");
    let text = std::fs::read_to_string(&path).expect("the corpus is read");
    let entries: Vec<Value> = serde_json::from_str(&text).expect("the corpus is JSON");
    entries
        .iter()
        .map(|entry| {
            let dims = entry["dimensions"]
                .as_object()
                .expect("dimensions")
                .iter()
                .map(|(k, v)| (k.clone(), v.as_f64().expect("an exponent")))
                .collect();
            (
                entry["expression"]
                    .as_str()
                    .expect("an expression")
                    .to_owned(),
                entry["factor_to_root"].as_f64().expect("a factor"),
                dims,
            )
        })
        .collect()
}

#[test]
fn every_expression_matches_pint() {
    let registry = Registry::new();
    let entries = corpus();
    assert_eq!(entries.len(), 177);
    let mut failures = Vec::new();
    for (expression, factor, dims) in &entries {
        match registry.parse(expression) {
            Ok(unit) => {
                let relative = ((unit.factor - factor) / factor).abs();
                let ours: BTreeMap<String, f64> = DIMENSIONS
                    .iter()
                    .zip(unit.dims)
                    .filter(|(_, e)| *e != 0)
                    .map(|(name, e)| (format!("[{name}]"), f64::from(e)))
                    .collect();
                if relative > 1e-15 || &ours != dims {
                    failures.push(format!(
                        "{expression}: {} {ours:?}, pint {factor} {dims:?}",
                        unit.factor
                    ));
                }
            }
            Err(err) => failures.push(format!("{expression}: {err}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Stronger than the tolerance of the parity: the factors are pint's to the last bit, since
/// the registry multiplies the scales of the definitions in pint's order.
#[test]
fn every_factor_is_the_bits_of_pint() {
    let registry = Registry::new();
    let differ: Vec<String> = corpus()
        .iter()
        .filter_map(|(expression, factor, _)| {
            let ours = registry.parse(expression).ok()?.factor;
            (ours.to_bits() != factor.to_bits()).then(|| format!("{expression}: {ours} {factor}"))
        })
        .collect();
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
