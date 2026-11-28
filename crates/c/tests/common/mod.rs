//! The test data which the tests of the C target (`c_target.rs`, `qfunction.rs`), of the stepper
//! of the engine (`crates/engine/tests/stepper.rs`, `driven.rs`) and of the batched kernel
//! (`crates/codegen/tests/kernel.rs`) share, the latter three including this file by its path:
//! the cell model of `testdata/models` with its 50 seeded parameter sets, the small models of the
//! integrator (`linear`, `blow_up`, `with_event`), the model of a driven species of a closed form
//! (`uptake`), and the cases of the SBML Test Suite with events and with an algebraic rule.
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

/// The rate constant `k` of [`uptake`].
pub const UPTAKE_K: f64 = 0.7;
/// The size `Vb` of the blood of [`uptake`].
pub const UPTAKE_VB: f64 = 0.15;

/// `X -> C` of the rate `(k * blood) * X`, the amount of `X` times `k`: `X` in `blood`, whose
/// size is the parameter `Vb` (an initial assignment), given by the initial amount `0.3` (so
/// the initial value of its path, with `X` driven, is an initial evaluation, `0.3 / blood`), `C`
/// in `cell` of size 1. The product `k * blood` reads parameters only, a constant of the
/// optimizer. With `X` driven along `value + slope (t - t_ref)` the exchange over an interval is
/// `-k Vb` times the integral of the path.
pub fn uptake() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="uptake">
<listOfCompartments><compartment id="blood" constant="true"/><compartment id="cell" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="X" compartment="blood" initialAmount="0.3" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/><species id="C" compartment="cell" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="k" value="{UPTAKE_K}" constant="true"/><parameter id="Vb" value="{UPTAKE_VB}" constant="true"/></listOfParameters>
<listOfInitialAssignments><initialAssignment symbol="blood"><math {MATH}><ci>Vb</ci></math></initialAssignment></listOfInitialAssignments>
<listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="X" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="C" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math {MATH}><apply><times/><apply><times/><ci>k</ci><ci>blood</ci></apply><ci>X</ci></apply></math></kineticLaw></reaction></listOfReactions>
</model></sbml>"#
    )
}

/// The integral of the path `value + slope (t - t_ref)` from `t0` to `t1`.
pub fn path_integral(value: f64, slope: f64, t_ref: f64, t0: f64, t1: f64) -> f64 {
    let primitive = |t: f64| value * t + slope * (t - t_ref).powi(2) / 2.0;
    primitive(t1) - primitive(t0)
}

/// The bound of the states of a driven model, the accumulators included: twice the one of the
/// QFunction on the undriven cell model, `DRIVEN_RELATIVE |y| + DRIVEN_ABSOLUTE`.
pub const DRIVEN_RELATIVE: f64 = 4e-6;
pub const DRIVEN_ABSOLUTE: f64 = 4e-9;

/// A driven model of the tests: its SBML, the species it drives, the changes of a point (its
/// pre-initialization values), a parameter behind a constant of the optimizer (which every
/// interval sets anew) and its range, the range of the paths, and the intervals.
pub struct Driven {
    pub sbml: String,
    pub ids: Vec<String>,
    pub changes: fn(&mut Random) -> Vec<(String, f64)>,
    pub hoisted: &'static str,
    pub hoisted_range: (f64, f64),
    pub path_range: f64,
    pub interval: f64,
    pub intervals: usize,
}

/// The driven models of the tests: [`uptake`] with `X` driven, of a closed form, and the cell
/// model with `S_ext` and `P_ext` driven.
pub fn driven_models() -> [Driven; 2] {
    let ids = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect();
    [
        Driven {
            sbml: uptake(),
            ids: ids(&["X"]),
            changes: |_| vec![],
            hoisted: "k",
            hoisted_range: (0.1, 2.0),
            path_range: 5.0,
            interval: 1.0,
            intervals: 6,
        },
        Driven {
            sbml: cell_model(),
            ids: ids(&["S_ext", "P_ext"]),
            changes: |random| {
                vec![
                    ("pattern".to_string(), 7.0),
                    ("protein_ext".to_string(), random.uniform()),
                    ("position".to_string(), random.uniform()),
                ]
            },
            hoisted: "S2PT_Vmax",
            hoisted_range: (0.01, 0.1),
            path_range: 5.0,
            interval: 10.0,
            intervals: 6,
        },
    ]
}

impl Driven {
    /// The interval `k`: its start and its end.
    pub fn bounds(&self, k: usize) -> (f64, f64) {
        (self.interval * k as f64, self.interval * (k + 1) as f64)
    }

    /// The parameters a point sets at the start `t0` of an interval, as `(index, value)`: the
    /// reference time `t0`, a random path per species (positive over the interval) and a random
    /// value of the parameter `hoisted` (an index of `desc`).
    pub fn interval_values(
        &self,
        desc: &sbmlsim_abi::ModelDescription,
        hoisted: u32,
        random: &mut Random,
        t0: f64,
    ) -> Vec<(u32, f64)> {
        let mut values = vec![(desc.t_ref.unwrap(), t0)];
        for d in &desc.driven {
            let value = random.between(0.0, self.path_range);
            let slope = random.between(-value, self.path_range) / self.interval;
            values.extend([(d.value, value), (d.slope, slope)]);
        }
        let (low, high) = self.hoisted_range;
        values.push((hoisted, random.between(low, high)));
        values
    }
}

/// The species which the reactions of the species `id` change, `id` excepted: the amounts of
/// the cell the accumulator of `id` balances, when `id` is driven.
pub fn balanced_species(sbml: &str, id: &str) -> Vec<String> {
    let doc = sbmlsim_sbml::read(sbml).unwrap();
    let mut species: Vec<String> = Vec::new();
    for reaction in &doc.model.reactions {
        let changed = || reaction.reactants.iter().chain(&reaction.products);
        if changed().any(|r| r.species == id) {
            for r in changed() {
                if r.species != id && !species.contains(&r.species) {
                    species.push(r.species.clone());
                }
            }
        }
    }
    species
}

/// How a test of a driven model bounds the accumulators `X__exchange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accumulators {
    /// Relative to themselves, as every other state: `DRIVEN_RELATIVE |y| + DRIVEN_ABSOLUTE`.
    Own,
    /// Relative to the amounts of the cell they balance, `DRIVEN_RELATIVE s + DRIVEN_ABSOLUTE`
    /// with `s` the sum of the absolute amounts of the species the reactions of the driven
    /// species change ([`balanced_species`]), the larger one of the start and the end of the
    /// interval. An accumulator is the difference of those amounts (their rates are linear
    /// invariants, which RODAS4 keeps), so the tolerances bound its error relative to them,
    /// not relative to the accumulator, which can be a hundredth of them.
    Balanced,
}

/// The largest difference of a test of a driven model relative to its bound, and the state of
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct Worst {
    /// The id of the state.
    pub state: String,
    /// Its difference over its bound, at most 1.
    pub ratio: f64,
}

impl std::fmt::Display for Worst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {:.3e} of its bound", self.state, self.ratio)
    }
}

/// 200 points of a driven model advanced by `advance` (the QFunction or the kernel, every point
/// from its control to `context.t1`) at the tolerances `rtol` and `atol`, against a stepper per
/// point of the native model of the C target ([`sbmlsim_c::generate_with`]) at 1e-12 and
/// 1e-14: each point with a random path per species and interval and a random value of the
/// parameter behind a constant ([`Driven::interval_values`]), the accumulators starting every
/// interval at 0. Every point is done after every interval and every state within its bound;
/// for [`uptake`], the exchange is also within the bound of its closed form. Gives the largest
/// difference relative to its bound and its state.
pub fn driven_against_steppers(
    model: &Driven,
    rtol: f64,
    atol: f64,
    accumulators: Accumulators,
    advance: &mut dyn FnMut(&mut sbmlsim_c::Points, &sbmlsim_c::RosenbrockContext),
) -> Worst {
    use sbmlsim_abi::{EntityKind, IntegratorSettings, Tolerance};
    use sbmlsim_c::{CModel, CONTROL, CONTROL_DONE, Library, Points, RosenbrockContext};
    use sbmlsim_engine::Stepper;
    use std::sync::Arc;

    let source = sbmlsim_c::generate_with(&model.sbml, &model.ids).unwrap();
    let desc = Arc::new(source.description);
    assert!(desc.n_constants > 0, "{}: no constant", desc.id);
    assert_eq!(desc.driven.len(), model.ids.len());
    let native = CModel::new(Library::build(&source.code).unwrap(), &desc).unwrap();
    let settings = IntegratorSettings {
        relative_tolerance: 1e-12,
        absolute_tolerance: Tolerance::Scalar(1e-14),
        ..IntegratorSettings::default()
    };
    let (n, m) = (desc.states.len(), desc.parameters.len());
    let state_of = |id: &str| desc.states.iter().position(|s| s.id == id).unwrap();
    let parameter_of = |id: &str| desc.parameters.iter().position(|p| p.id == id).unwrap();
    // the states of the amounts each accumulator balances
    let balanced: Vec<(usize, Vec<usize>)> = desc
        .driven
        .iter()
        .map(|d| {
            let states = balanced_species(&model.sbml, &d.id)
                .iter()
                .map(|id| {
                    let k = state_of(id);
                    assert!(
                        matches!(
                            desc.states[k].kind,
                            EntityKind::Species {
                                stores_concentration: false,
                                ..
                            }
                        ),
                        "{id} is an amount"
                    );
                    k
                })
                .collect::<Vec<_>>();
            assert!(!states.is_empty(), "{} balances no species", d.id);
            (d.exchange as usize, states)
        })
        .collect();
    let mut random = Random(23);
    let mut steppers: Vec<Stepper> = (0..200)
        .map(|_| {
            let changes = (model.changes)(&mut random);
            Stepper::new(
                Box::new(native.clone()),
                desc.clone(),
                settings.clone(),
                &changes,
            )
            .unwrap()
        })
        .collect();
    let mut points = Points {
        state: steppers.iter().flat_map(|s| s.state().to_vec()).collect(),
        parameters: steppers
            .iter()
            .flat_map(|s| s.parameters().to_vec())
            .collect(),
        control: vec![0.0; steppers.len() * CONTROL],
        counts: vec![0.0; steppers.len() * sbmlsim_c::COUNTS],
    };
    let hoisted = parameter_of(model.hoisted) as u32;
    let mut worst = Worst {
        state: String::new(),
        ratio: 0.0,
    };
    for k in 0..model.intervals {
        let (t0, t1) = model.bounds(k);
        let mut starts = Vec::with_capacity(steppers.len());
        for (i, stepper) in steppers.iter_mut().enumerate() {
            let values = model.interval_values(&desc, hoisted, &mut random, t0);
            stepper.set_parameters(&values).unwrap();
            points.parameters[i * m..(i + 1) * m].copy_from_slice(stepper.parameters());
            let mut y = stepper.state().to_vec();
            for d in &desc.driven {
                y[d.exchange as usize] = 0.0;
            }
            points.state[i * n..(i + 1) * n].copy_from_slice(&y);
            stepper.advance(&y, t0, t1).unwrap();
            starts.push(y);
        }
        points.start(t0);
        let context = RosenbrockContext {
            t1,
            rtol,
            atol,
            h_min: 1e-10,
            attempts: 1e6,
        };
        advance(&mut points, &context);
        for (i, stepper) in steppers.iter().enumerate() {
            let control = &points.control[i * CONTROL..(i + 1) * CONTROL];
            assert_eq!(control[CONTROL_DONE], 1.0, "{}, point {i}", desc.id);
            let state = &points.state[i * n..(i + 1) * n];
            let reference = stepper.state();
            for (s, (a, b)) in state.iter().zip(reference).enumerate() {
                let scale = match balanced.iter().find(|(e, _)| *e == s) {
                    Some((_, cell)) if accumulators == Accumulators::Balanced => {
                        let sum = |y: &[f64]| cell.iter().map(|&c| y[c].abs()).sum::<f64>();
                        sum(reference).max(sum(&starts[i]))
                    }
                    _ => b.abs(),
                };
                let bound = DRIVEN_RELATIVE * scale + DRIVEN_ABSOLUTE;
                let ratio = (a - b).abs() / bound;
                if ratio > worst.ratio {
                    worst = Worst {
                        state: desc.states[s].id.clone(),
                        ratio,
                    };
                }
                assert!(
                    (a - b).abs() <= bound,
                    "{}, point {i}, {} at {t1}: {a}, stepper {b}, bound {bound}",
                    desc.id,
                    desc.states[s].id
                );
            }
            if desc.id == "uptake" {
                let p = &points.parameters[i * m..(i + 1) * m];
                let d = &desc.driven[0];
                let exact = -p[parameter_of("k")]
                    * UPTAKE_VB
                    * path_integral(p[d.value as usize], p[d.slope as usize], t0, t0, t1);
                let exchange = state[d.exchange as usize];
                assert!(
                    (exchange - exact).abs() <= DRIVEN_RELATIVE * exact.abs() + DRIVEN_ABSOLUTE,
                    "point {i} at {t1}: exchange {exchange}, closed form {exact}"
                );
            }
        }
    }
    worst
}
