//! The test data which the tests of the C target (`c_target.rs`, `qfunction.rs`), of the stepper
//! of the engine (`crates/engine/tests/stepper.rs`) and of the batched kernel
//! (`crates/codegen/tests/kernel.rs`) share, the latter two including this file by its path: the
//! cell model of `testdata/models` with its 50 seeded parameter sets, the small models of the
//! integrator (`linear`, `blow_up`, `with_event`), and the cases of the SBML Test Suite with
//! events and with an algebraic rule.
//!
//! The tests over `testdata/cases` need `uv run scripts/prepare_testsuite.py` and skip with a
//! note when the directory is absent, unless `SBMLSIM_CORE_REQUIRE_CASES` is set (the jobs of
//! the suite in CI), where they fail.

// every test crate which includes this file uses a part of it
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

/// The cases with events: a delay (00071), priorities (00934), an event which assigns a
/// compartment of a species of a concentration (01506).
pub const EVENT_CASES: [&str; 3] = ["00071", "00934", "01506"];

/// The cases with an algebraic rule, which IDA integrates (00039).
pub const ALGEBRAIC_CASES: [&str; 1] = ["00039"];

/// The namespace declaration of MathML.
pub const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// The directory `testdata` of the workspace.
pub fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

/// The SBML of the cell model of `testdata/models`.
pub fn cell_model() -> String {
    fs::read_to_string(testdata().join("models/spt_liver_cell.xml")).unwrap()
}

/// SplitMix64, a seeded generator of uniform numbers in `[0, 1)`.
pub struct Random(pub u64);

impl Random {
    pub fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.uniform()
    }
}

/// The 50 parameter sets of the cell model, seeded: its pattern, the protein and the position
/// of the cell, and the concentration of the external substrate, as pre-initialization values.
pub fn cell_parameter_sets() -> Vec<Vec<(String, f64)>> {
    let mut random = Random(7);
    (0..50)
        .map(|_| {
            vec![
                ("pattern".to_string(), 7.0),
                ("protein_ext".to_string(), random.uniform()),
                ("position".to_string(), random.uniform()),
                ("[S_ext]".to_string(), random.between(0.0, 5.0)),
            ]
        })
        .collect()
}

/// The directory of the prepared cases, `None` (with a note) where it is absent.
pub fn cases() -> Option<PathBuf> {
    let dir = testdata().join("cases");
    if dir.join("00001/case.json").is_file() {
        return Some(dir);
    }
    assert!(
        std::env::var_os("SBMLSIM_CORE_REQUIRE_CASES").is_none(),
        "{} is absent and SBMLSIM_CORE_REQUIRE_CASES is set",
        dir.display()
    );
    eprintln!(
        "note: {} is absent, run `uv run scripts/prepare_testsuite.py`; skipped",
        dir.display()
    );
    None
}

/// A prepared case of the suite: its model and the settings of its run.
pub struct Case {
    pub sbml: String,
    pub start: f64,
    pub duration: f64,
    pub steps: u32,
    pub selections: Vec<String>,
    pub absolute_tolerance: f64,
    pub relative_tolerance: f64,
}

impl Case {
    /// Reads the case of the directory `dir`.
    pub fn read(dir: &Path) -> Case {
        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("case.json")).unwrap()).unwrap();
        let number = |key: &str| settings[key].as_f64().unwrap();
        Case {
            sbml: fs::read_to_string(dir.join("model.xml")).unwrap(),
            start: number("start"),
            duration: number("duration"),
            steps: settings["steps"].as_u64().unwrap() as u32,
            selections: settings["selections"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect(),
            absolute_tolerance: number("absolute_tolerance"),
            relative_tolerance: number("relative_tolerance"),
        }
    }
}

/// `x' = -x` and `y' = k (cos t - y)` with `k = 1`, `x(0) = 1`, `y(0) = 2`: `x = exp(-t)` and
/// `y = (cos t + sin t) / 2 + 3/2 exp(-t)`, a rate which depends on the time. (Not the
/// Prothero-Robinson problem `y' = -k (y - g(t)) + g'(t)` of a large `k`, on which RODAS4, as
/// every Rosenbrock method of its kind, loses order.)
pub fn linear() -> String {
    let time = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;
    let rate_y = format!(
        "<apply><times/><ci>k</ci><apply><minus/><apply><cos/>{time}</apply><ci>y</ci></apply></apply>"
    );
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="linear"><listOfParameters><parameter id="k" value="1" constant="true"/><parameter id="x" value="1" constant="false"/><parameter id="y" value="2" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><minus/><ci>x</ci></apply></math></rateRule><rateRule variable="y"><math {MATH}>{rate_y}</math></rateRule></listOfRules></model></sbml>"#
    )
}

/// A model of `x` with an event which halves it at `x < 0.5`.
pub fn with_event() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="event"><listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><minus/><ci>x</ci></apply></math></rateRule></listOfRules><listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="true" persistent="true"><math {MATH}><apply><lt/><ci>x</ci><cn>0.5</cn></apply></math></trigger><listOfEventAssignments><eventAssignment variable="x"><math {MATH}><apply><divide/><ci>x</ci><cn>2</cn></apply></math></eventAssignment></listOfEventAssignments></event></listOfEvents></model></sbml>"#
    )
}

/// `y' = a y^2` with `y(0) = 1`: `y = 1 / (1 - a t)`, which grows without bound at `t = 1 / a`.
pub fn blow_up() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="blow_up"><listOfParameters><parameter id="a" value="1" constant="true"/><parameter id="y" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="y"><math {MATH}><apply><times/><ci>a</ci><ci>y</ci><ci>y</ci></apply></math></rateRule></listOfRules></model></sbml>"#
    )
}
