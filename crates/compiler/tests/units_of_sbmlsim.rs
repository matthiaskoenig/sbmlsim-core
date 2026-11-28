//! The units of the description `model/2` are the ones sbmlsim gives the entities of the model:
//! every model of `testdata/model_units` (written by sbmlsim's `scripts/scan_fixtures.py` from
//! `RoadrunnerSBMLModel(...).uinfo`, the SBML of the model the one sbmlsim hands to the engine)
//! compiled, its `units` the strings of the file byte for byte and its `time_unit` the one of
//! the key `time`; every unit of a description is read by a registry with the unit definitions
//! of the description.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sbmlsim_compiler::{CompileOptions, compile};

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/model_units")
}

/// The names of the models, sorted.
fn models() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(testdata())
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            (path.extension()? == "json").then(|| path.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    names.sort();
    names
}

#[test]
fn the_units_are_the_ones_of_sbmlsim() {
    let names = models();
    // the models sbmlsim's `scripts/scan_fixtures.py` writes, a model missing from it fails here
    assert_eq!(
        names.len(),
        24,
        "the model units are not the 24 models: {names:?}"
    );
    let mut failures = Vec::new();
    let mut entities = 0;
    for name in &names {
        let text = std::fs::read_to_string(testdata().join(format!("{name}.json"))).unwrap();
        let expected: BTreeMap<String, String> = serde_json::from_str(&text).unwrap();
        let sbml = std::fs::read_to_string(testdata().join(format!("{name}.xml"))).unwrap();
        let description = compile(&sbml, &CompileOptions::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .description;
        entities += expected.len();
        for (id, unit) in &expected {
            match description.units.get(id) {
                Some(ours) if ours == unit => {}
                ours => failures.push(format!("{name}: {id}: {ours:?} != {unit:?}")),
            }
        }
        for id in description.units.keys() {
            if !expected.contains_key(id) {
                failures.push(format!("{name}: {id} is no entity of sbmlsim's units"));
            }
        }
        if description.time_unit.as_ref() != expected.get("time") {
            failures.push(format!(
                "{name}: the time unit {:?} != {:?}",
                description.time_unit,
                expected.get("time")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {entities} units differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn a_registry_reads_every_unit_with_the_unit_definitions() {
    let mut read = 0;
    for name in &models() {
        let sbml = std::fs::read_to_string(testdata().join(format!("{name}.xml"))).unwrap();
        let description = compile(&sbml, &CompileOptions::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .description;
        let registry = sbmlsim_units::Registry::new()
            .with_model_units(&BTreeMap::new(), &description.unit_definitions);
        for (defined, expression) in &description.unit_definitions {
            // a name of a definition is no unit of pint
            assert!(
                sbmlsim_units::Registry::new().parse(defined).is_err(),
                "{name}: {defined}"
            );
            assert_eq!(
                registry.factor(defined, expression).unwrap(),
                1.0,
                "{name}: {defined}"
            );
        }
        for (id, unit) in &description.units {
            if description.unit_definitions.contains_key(unit) {
                registry
                    .parse(unit)
                    .unwrap_or_else(|e| panic!("{name}: the unit {unit:?} of {id}: {e}"));
                read += 1;
            }
        }
    }
    assert!(read > 0, "no unit of the models is a unit definition");
}
