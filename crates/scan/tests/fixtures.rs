//! The requests of the planner against the oracle fixtures of sbmlsim (`testdata/scan`, written
//! by sbmlsim's `scripts/scan_fixtures.py`): every round a case asks is the round of requests
//! `simulation/1` sbmlsim's engine core sent for it, bitwise.
//!
//! The description of a model is compiled again from the SBML of the fixture: the fixtures
//! recorded it before the description carried the time unit of the model and before its units
//! were the ones sbmlsim names, and the test checks that the description is the recorded one
//! besides the fields added since and the version of the compiler, with every recorded unit the
//! same unit of the registry (the same factor bits and dimensions) in the spelling of sbmlsim
//! and the units added since the ones of entities without units (empty) and of the time. The
//! exceptions are read from the SBML of the model (`UnitExceptions`): a unit which only the
//! default units of the model gave (a species or a compartment which states no units of its
//! own, which sbmlsim leaves empty) may be empty now, a reaction, recorded as the extent of the
//! model per its time, has the unit libsbml derives for its kinetic law, and a unit sbmlsim names
//! by the id of a unit definition of the model as pint has that unit agrees within the relative
//! 1e-5 of `UnitsInformation._define` (`mmHg`); `crates/compiler/tests/units_of_sbmlsim.rs`
//! checks every unit of these models against sbmlsim.
//!
//! A mapping of the request of a fixture (a pre-initialization of several targets, the
//! simulations of a dimension) is in the order of the user, which decides the order of the
//! requests, so the request is read from the text of the fixture and never through a
//! `serde_json::Value`, whose map sorts its keys.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sbmlsim_abi::{ModelDescription, SimulationRequest, SimulationResult};
use sbmlsim_scan::{
    Ask, CoordValues, FailedPoint, HostObserved, PointError, Scan, ScanError, ScanErrorKind,
    ScanRequest, ScanResult,
};
use serde::Deserialize;
use serde_json::Value as Json;

/// The cases of the planner of requests: no fold, no event, no observable.
const CASES: [&str; 15] = [
    "steps_plain",
    "times_output",
    "time_unit_hr_quantities",
    "multiple_dosing",
    "preinit",
    "presimulation_steady_state",
    "steady_state_output",
    "time_shift",
    "negative_start",
    "scan_2d_values",
    "scan_at",
    "scan_simulations",
    "keep_selections",
    "time_grid",
    "points_block",
];

/// The cases of the rounds: folds of formulas at changes, models with events and a steady state
/// after the end, every round answered by the engine.
const ROUND_CASES: [&str; 5] = [
    "fold_linear_form",
    "fold_nonlinear",
    "events_steps",
    "events_times",
    "steady_state_output",
];

/// The fields of the description added since the fixtures were written.
const ADDED: [&str; 4] = [
    "time_unit",
    "initial_assignments",
    "time_dependent",
    "unit_definitions",
];

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/scan")
}

/// A fixture.
struct Fixture {
    case: String,
    request: ScanRequest,
    description: ModelDescription,
    wasm: Vec<u8>,
    rounds: Vec<Vec<SimulationRequest>>,
}

/// The fields of a fixture the test reads, the request in the order of its text.
#[derive(Deserialize)]
struct File {
    request: ScanRequest,
    engine_rounds: Vec<Vec<SimulationRequest>>,
    model: String,
    description: Json,
}

fn fixture(case: &str) -> Fixture {
    let text = std::fs::read_to_string(testdata().join(format!("{case}.json"))).unwrap();
    let file: File = serde_json::from_str(&text).unwrap();
    let sbml = std::fs::read_to_string(testdata().join("models").join(&file.model)).unwrap();
    let compiled = sbmlsim_compiler::compile(&sbml, &Default::default()).unwrap();
    let description = compiled.description;
    let mut written = serde_json::to_value(&description).unwrap();
    for key in ADDED {
        written.as_object_mut().unwrap().remove(key);
    }
    // the version of the compiler which recorded the fixture
    written["compiler"] = file.description["compiler"].clone();
    let mut recorded = file.description.clone();
    let units = written.as_object_mut().unwrap().remove("units").unwrap();
    let recorded_units = recorded.as_object_mut().unwrap().remove("units").unwrap();
    if let Err(difference) = same_units(&units, &recorded_units, &UnitExceptions::of(&sbml)) {
        panic!("{case}: the units compiled again are not the recorded ones: {difference}");
    }
    if let Err(difference) = same(&written, &recorded, "description") {
        panic!("{case}: the description compiled again is not the recorded one: {difference}");
    }
    Fixture {
        case: case.to_string(),
        request: file.request,
        description,
        wasm: compiled.wasm,
        rounds: file.engine_rounds,
    }
}

/// What of a model may differ from the recorded units, read from its SBML.
#[derive(Default)]
struct UnitExceptions {
    /// The reactions: the recorded unit was the extent of the model per its time, sbmlsim names
    /// the unit libsbml derives for its kinetic law.
    reactions: Vec<String>,
    /// The species (`S` and `[S]`) and compartments whose recorded unit only the default units
    /// of the model gave: sbmlsim gives them none, so their unit may be empty now.
    defaulted: Vec<String>,
    /// The unit definitions of the model, the ids sbmlsim may name a unit by as pint has it.
    unit_ids: Vec<String>,
}

impl UnitExceptions {
    /// The exceptions of a model.
    fn of(sbml: &str) -> UnitExceptions {
        let m = sbmlsim_sbml::read(sbml).unwrap().model;
        let stated = |units: &Option<String>| units.as_deref().is_some_and(|u| !u.is_empty());
        let mut defaulted = Vec::new();
        for c in &m.compartments {
            if !stated(&c.units) {
                defaulted.push(c.id.clone());
            }
        }
        for s in &m.species {
            let compartment = m.compartments.iter().find(|c| c.id == s.compartment);
            let volume = compartment.is_some_and(|c| stated(&c.units));
            if !stated(&s.substance_units) {
                defaulted.push(s.id.clone());
            }
            if !stated(&s.substance_units) || !volume {
                defaulted.push(format!("[{}]", s.id));
            }
        }
        UnitExceptions {
            reactions: m.reactions.iter().map(|r| r.id.clone()).collect(),
            defaulted,
            unit_ids: m.unit_definitions.iter().map(|u| u.id.clone()).collect(),
        }
    }
}

/// The units of a description compiled again against the recorded ones (see the module): every
/// recorded unit the same unit of the registry, its factor by its bits (within the relative 1e-5
/// of `UnitsInformation._define` for a unit sbmlsim names by the id of a unit definition) and
/// its dimensions, except a reaction and an entity of a default unit which is empty now, and
/// every other unit empty or the one of the time; the first difference.
fn same_units(units: &Json, recorded: &Json, exceptions: &UnitExceptions) -> Result<(), String> {
    let registry = sbmlsim_units::Registry::new();
    let (units, recorded) = (units.as_object().unwrap(), recorded.as_object().unwrap());
    for (id, old) in recorded {
        let old = old.as_str().unwrap();
        let Some(new) = units.get(id).and_then(Json::as_str) else {
            return Err(format!("{id}: no unit, the recorded one is '{old}'"));
        };
        if exceptions.reactions.contains(id) {
            continue;
        }
        if new.is_empty() && (old.is_empty() || exceptions.defaulted.contains(id)) {
            continue;
        }
        let parse = |unit: &str| {
            registry
                .parse(unit)
                .map_err(|err| format!("{id}: '{unit}' is no unit: {err}"))
        };
        let (a, b) = (parse(new)?, parse(old)?);
        let named = exceptions.unit_ids.iter().any(|u| u == new)
            && (a.factor - b.factor).abs() <= 1e-5 * b.factor.abs();
        if (a.factor.to_bits() != b.factor.to_bits() && !named) || a.dims != b.dims {
            return Err(format!("{id}: '{new}' is another unit than '{old}'"));
        }
    }
    for (id, new) in units {
        if !recorded.contains_key(id) && id != "time" && new != "" {
            return Err(format!("{id}: the unit '{new}' was not recorded"));
        }
    }
    Ok(())
}

#[test]
fn a_unit_is_the_recorded_one_unless_an_exception_names_it() {
    let json = |pairs: &[(&str, &str)]| {
        Json::Object(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), Json::String(v.to_string())))
                .collect(),
        )
    };
    let none = UnitExceptions::default();
    // the same unit in the spelling of sbmlsim
    let recorded = json(&[("S", "0.001*mole"), ("k", "(60*second)**-1")]);
    same_units(&json(&[("S", "mmol"), ("k", "1/min")]), &recorded, &none).unwrap();
    // a blanked unit, another unit and a missing unit fail
    let blanked = same_units(&json(&[("S", ""), ("k", "1/min")]), &recorded, &none);
    assert_eq!(
        blanked.unwrap_err(),
        "S: '' is another unit than '0.001*mole'"
    );
    let other = same_units(&json(&[("S", "mol"), ("k", "1/min")]), &recorded, &none);
    assert!(other.unwrap_err().contains("another unit"));
    assert!(same_units(&json(&[("k", "1/min")]), &recorded, &none).is_err());
    // an empty unit of an entity of a default unit, and of a reaction, pass
    let defaulted = UnitExceptions {
        defaulted: vec!["S".into()],
        ..Default::default()
    };
    same_units(&json(&[("S", ""), ("k", "1/min")]), &recorded, &defaulted).unwrap();
    // a unit within 1e-5 passes only when it is the id of a unit definition of the model
    let recorded = json(&[("p", "133.32239*newton*metre**-2")]);
    assert!(same_units(&json(&[("p", "mmHg")]), &recorded, &none).is_err());
    let unit_ids = UnitExceptions {
        unit_ids: vec!["mmHg".into()],
        ..Default::default()
    };
    same_units(&json(&[("p", "mmHg")]), &recorded, &unit_ids).unwrap();
    // a unit not recorded is empty or the time
    let recorded = json(&[]);
    same_units(&json(&[("time", "min"), ("q", "")]), &recorded, &none).unwrap();
    assert!(same_units(&json(&[("q", "mmol")]), &recorded, &none).is_err());
}

/// Two JSON values equal, every number by its bits; the path of the first difference.
fn same(a: &Json, b: &Json, path: &str) -> Result<(), String> {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if x.to_bits() == y.to_bits() {
                Ok(())
            } else {
                Err(format!("{path}: {x:?} != {y:?}"))
            }
        }
        (Json::Array(x), Json::Array(y)) => {
            if x.len() != y.len() {
                return Err(format!("{path}: {} != {} items", x.len(), y.len()));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .try_for_each(|(k, (x, y))| same(x, y, &format!("{path}[{k}]")))
        }
        (Json::Object(x), Json::Object(y)) => {
            let keys = |m: &serde_json::Map<String, Json>| {
                let mut k: Vec<String> = m.keys().cloned().collect();
                k.sort();
                k
            };
            if keys(x) != keys(y) {
                return Err(format!("{path}: keys {:?} != {:?}", keys(x), keys(y)));
            }
            x.iter()
                .try_for_each(|(k, v)| same(v, &y[k], &format!("{path}.{k}")))
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{path}: {a} != {b}")),
    }
}

/// A request as the engine reads it, the defaults of the contract written.
fn normal(request: &SimulationRequest) -> Json {
    serde_json::to_value(request).unwrap()
}

/// Runs a scan with answers of no rows, the rounds it asks; the scan is of `on_error: flag`.
fn rounds_of(scan: &mut Scan) -> Vec<Vec<SimulationRequest>> {
    let mut rounds = Vec::new();
    loop {
        match scan.ask() {
            Ask::Simulate { requests, .. } => {
                let answers = requests
                    .iter()
                    .map(|r| {
                        Ok(SimulationResult {
                            columns: r.selections.clone(),
                            n_rows: 0,
                            values: Vec::new(),
                            sensitivities: None,
                            local_errors: None,
                        })
                    })
                    .collect();
                rounds.push(requests);
                scan.tell(answers).unwrap();
            }
            Ask::Observe(_) => panic!("no observable is asked"),
            Ask::Done => return rounds,
        }
    }
}

/// Compares the rounds a scan asked with the ones of its fixture, bitwise.
fn compare_rounds(
    case: &str,
    rounds: &[Vec<SimulationRequest>],
    expected: &[Vec<SimulationRequest>],
    failures: &mut Vec<String>,
) {
    if rounds.len() != expected.len() {
        failures.push(format!(
            "{case}: {} rounds, the fixture has {}",
            rounds.len(),
            expected.len()
        ));
        return;
    }
    for (k, (round, expected)) in rounds.iter().zip(expected).enumerate() {
        let (round, expected): (Vec<Json>, Vec<Json>) = (
            round.iter().map(normal).collect(),
            expected.iter().map(normal).collect(),
        );
        if let Err(difference) = same(
            &Json::Array(round),
            &Json::Array(expected),
            &format!("round {k}"),
        ) {
            failures.push(format!("{case}: {difference}"));
        }
    }
}

/// Runs a scan with the answers of the engine, the rounds it asks.
fn engine_rounds(
    scan: &mut Scan,
    instance: &mut sbmlsim_host::Instance,
) -> Result<Vec<Vec<SimulationRequest>>, ScanError> {
    let mut rounds = Vec::new();
    loop {
        match scan.ask() {
            Ask::Simulate { requests, .. } => {
                let answers = requests
                    .iter()
                    .map(|r| instance.simulate(r).map_err(PointError::from))
                    .collect();
                rounds.push(requests);
                scan.tell(answers)?;
            }
            Ask::Observe(_) => panic!("no observable is asked"),
            Ask::Done => return Ok(rounds),
        }
    }
}

#[test]
fn rounds_match_fixtures() {
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    let mut failures = Vec::new();
    for case in ROUND_CASES {
        let f = fixture(case);
        let mut instance = runtime
            .load(&f.wasm, &f.description)
            .unwrap()
            .instantiate()
            .unwrap();
        let rounds = Scan::open(f.request.clone(), f.description.clone()).and_then(|mut scan| {
            let rounds = engine_rounds(&mut scan, &mut instance)?;
            let result = scan.result()?;
            assert!(result.errors.is_none(), "{case}");
            Ok(rounds)
        });
        match rounds {
            Ok(rounds) => compare_rounds(case, &rounds, &f.rounds, &mut failures),
            Err(err) => failures.push(format!("{case}: {err}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn requests_match_fixtures() {
    let mut failures = Vec::new();
    for case in CASES {
        let f = fixture(case);
        // an answer of no rows is no output of a grid the simulations share, which fails the
        // point: the points are flagged, the rounds are the ones asked
        let mut request = f.request.clone();
        request.on_error = sbmlsim_scan::OnError::Flag;
        let mut scan = match Scan::open(request, f.description.clone()) {
            Ok(scan) => scan,
            Err(err) => {
                failures.push(format!("{}: {err}", f.case));
                continue;
            }
        };
        let rounds = rounds_of(&mut scan);
        compare_rounds(&f.case, &rounds, &f.rounds, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn refuses_unknown_fields() {
    let text = std::fs::read_to_string(testdata().join("times_output.json")).unwrap();
    let json: Json = serde_json::from_str(&text).unwrap();
    let mut request = json["request"].clone();
    request["frobnicate"] = Json::Bool(true);
    let error = serde_json::from_value::<ScanRequest>(request)
        .unwrap_err()
        .to_string();
    assert!(error.contains("frobnicate"), "{error}");
    let mut request = json["request"].clone();
    request["scan"]["simulation"]["endd"] = Json::from(3);
    let error = serde_json::from_value::<ScanRequest>(request)
        .unwrap_err()
        .to_string();
    assert!(error.contains("endd"), "{error}");
    let mut request = json["request"].clone();
    request["observables"] =
        serde_json::json!([{"type": "Formula", "id": "f", "formula": "x", "units": "mM"}]);
    let error = serde_json::from_value::<ScanRequest>(request)
        .unwrap_err()
        .to_string();
    assert!(error.contains("units"), "{error}");
}

#[test]
fn unknown_target_names_it() {
    let f = fixture("times_output");
    let mut request = f.request.clone();
    request
        .scan
        .simulation
        .preinit_changes
        .0
        .push(("Vmax_nope".to_string(), sbmlsim_scan::Value::Number(1.0)));
    let err = Scan::open(request, f.description.clone()).unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Request);
    assert!(err.message.contains("'Vmax_nope'"), "{err}");

    let mut request = f.request.clone();
    request.scan.simulation.changes[0].values.0[0].0 = "nope".to_string();
    let err = Scan::open(request, f.description.clone()).unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Request);
    assert!(err.message.contains("'nope'"), "{err}");
}

/// A failure of the integration, as the engine answers it.
fn integration_failure() -> PointError {
    PointError {
        kind: sbmlsim_scan::PointErrorKind::Integration,
        message: "CVODE failed".to_string(),
        flag: Some(-3),
        time: Some(1.5),
    }
}

/// Runs the scan with the engine, the segment of the first fold of the point 1 failing.
fn failing_fold(scan: &mut Scan, instance: &mut sbmlsim_host::Instance) -> Vec<usize> {
    let mut sizes = Vec::new();
    loop {
        match scan.ask() {
            Ask::Simulate { requests, .. } => {
                let first = sizes.is_empty();
                sizes.push(requests.len());
                let answers = requests
                    .iter()
                    .enumerate()
                    .map(|(k, r)| {
                        if first && k == 1 {
                            Err(integration_failure())
                        } else {
                            instance.simulate(r).map_err(PointError::from)
                        }
                    })
                    .collect();
                if let Err(err) = scan.tell(answers) {
                    assert_eq!(err.kind, ScanErrorKind::Point);
                    assert_eq!(err.point, Some(1));
                    assert!(err.message.contains("rate=1"), "{err}");
                    assert!(err.message.contains("CVODE failed"), "{err}");
                    return sizes;
                }
            }
            Ask::Observe(_) => panic!("no observable is asked"),
            Ask::Done => return sizes,
        }
    }
}

#[test]
fn a_failed_segment_fails_its_point_alone() {
    let f = fixture("fold_nonlinear");
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    let mut instance = runtime
        .load(&f.wasm, &f.description)
        .unwrap()
        .instantiate()
        .unwrap();

    let mut request = f.request.clone();
    request.on_error = sbmlsim_scan::OnError::Flag;
    let mut scan = Scan::open(request, f.description.clone()).unwrap();
    // the point 1 folds no further and is no request of the round of the points
    assert_eq!(failing_fold(&mut scan, &mut instance), [2, 1, 1]);
    let result = scan.result().unwrap();
    let errors = result.errors.unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].index, 1);
    let text = &errors[0].message;
    assert!(text.starts_with("RuntimeError: "), "{text}");
    assert!(text.contains("CVODE failed"), "{text}");
    let status = result
        .variables
        .iter()
        .find(|v| v.name == "status")
        .unwrap();
    assert_eq!(status.values, [0.0, 1.0]);

    // with `on_error: raise` the point fails the scan once the points of the chunk ran
    let mut scan = Scan::open(f.request.clone(), f.description.clone()).unwrap();
    assert_eq!(failing_fold(&mut scan, &mut instance), [2, 1, 1]);
    assert_eq!(scan.ask(), Ask::Done);
    assert_eq!(scan.result().unwrap_err().point, Some(1));
}

/// Every case of the fixtures, all of which carry a `result` or the `error` of their run.
const RESULT_CASES: [&str; 25] = [
    "custom_observable",
    "events_steps",
    "events_times",
    "failing_point",
    "failing_point_raise",
    "fold_linear_form",
    "fold_nonlinear",
    "formula_reductions",
    "keep_selections",
    "multiple_dosing",
    "negative_start",
    "pk_oral",
    "points_block",
    "preinit",
    "presimulation_steady_state",
    "scan_2d_values",
    "scan_at",
    "scan_simulations",
    "steady_state_output",
    "steps_plain",
    "time_grid",
    "time_shift",
    "time_unit_hr_quantities",
    "times_output",
    "tutorial_scan",
];

/// The sum of numpy over a one dimensional array (`np.add.reduce`): the first value, plus the
/// pairwise sum of the others, so the host functions of the test are numpy's to the bit.
fn numpy_sum(values: &[f64]) -> f64 {
    fn pairwise(a: &[f64]) -> f64 {
        let n = a.len();
        if n < 8 {
            let mut res = -0.0;
            for x in a {
                res += x;
            }
            res
        } else if n <= 128 {
            let mut r = [0.0; 8];
            r.copy_from_slice(&a[..8]);
            let mut i = 8;
            while i < n - n % 8 {
                for j in 0..8 {
                    r[j] += a[i + j];
                }
                i += 8;
            }
            let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
            while i < n {
                res += a[i];
                i += 1;
            }
            res
        } else {
            let mut n2 = n / 2;
            n2 -= n2 % 8;
            pairwise(&a[..n2]) + pairwise(&a[n2..])
        }
    }
    match values.split_first() {
        None => 0.0,
        Some((first, rest)) => first + pairwise(rest),
    }
}

/// The functions of the host of the fixtures (`scripts/scan_fixtures.py` of sbmlsim).
fn host_function(function: &str, time: &[f64], columns: &BTreeMap<String, Vec<f64>>) -> Vec<f64> {
    let c = &columns["[C]"];
    match function {
        // np.trapezoid(values["[C]"], time)
        "scripts.scan_fixtures:auc_of_c" => {
            let pieces: Vec<f64> = (1..time.len())
                .map(|i| (time[i] - time[i - 1]) * (c[i] + c[i - 1]) / 2.0)
                .collect();
            vec![numpy_sum(&pieces)]
        }
        // 2.0 * values["[C]"]
        "scripts.scan_fixtures:doubled_c" => c.iter().map(|x| 2.0 * x).collect(),
        other => panic!("no host function {other}"),
    }
}

/// Runs a scan with the engine and the host functions, its rounds of requests.
fn run_with_host(
    scan: &mut Scan,
    instance: &mut sbmlsim_host::Instance,
    functions: &BTreeMap<String, String>,
) -> Result<Vec<Vec<SimulationRequest>>, ScanError> {
    let mut rounds = Vec::new();
    loop {
        match scan.ask() {
            Ask::Simulate { requests, .. } => {
                let answers = requests
                    .iter()
                    .map(|r| instance.simulate(r).map_err(PointError::from))
                    .collect();
                rounds.push(requests);
                scan.tell(answers)?;
            }
            Ask::Observe(asked) => {
                let values = asked
                    .observables
                    .iter()
                    .map(|id| {
                        let values = asked
                            .points
                            .iter()
                            .map(|p| host_function(&functions[id], &p.time, &p.columns))
                            .collect();
                        (id.clone(), values)
                    })
                    .collect();
                scan.tell_observed(HostObserved {
                    values,
                    errors: Vec::new(),
                })?;
            }
            Ask::Done => return Ok(rounds),
        }
    }
}

/// The entries of a JSON object in the order of its text, which a map of serde_json does not
/// keep (it sorts its keys).
struct Pairs(Vec<(String, String)>);

impl<'de> Deserialize<'de> for Pairs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Pairs, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Pairs;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object of texts")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Pairs, A::Error> {
                let mut pairs = Vec::new();
                while let Some(entry) = map.next_entry::<String, String>()? {
                    pairs.push(entry);
                }
                Ok(Pairs(pairs))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

/// What the comparison reads of a fixture in the order of its text: the units of the result
/// and the coordinates of its block.
#[derive(Deserialize)]
struct OrderedFixture {
    result: Option<OrderedResult>,
}

#[derive(Deserialize)]
struct OrderedResult {
    attrs: OrderedAttrs,
}

#[derive(Deserialize)]
struct OrderedAttrs {
    units: Pairs,
    #[serde(default)]
    block: Option<OrderedBlock>,
}

#[derive(Deserialize)]
struct OrderedBlock {
    coords: Pairs,
}

/// A number of a fixture: `null` is NaN, `"inf"` and `"-inf"` the infinities.
fn number(value: &Json) -> f64 {
    match value {
        Json::Null => f64::NAN,
        Json::String(s) if s == "inf" => f64::INFINITY,
        Json::String(s) if s == "-inf" => f64::NEG_INFINITY,
        other => other
            .as_f64()
            .unwrap_or_else(|| panic!("no number: {other}")),
    }
}

/// The distance of two numbers in units in the last place of the larger, 0 for equal bits
/// and two NaN.
fn ulps(a: f64, b: f64) -> f64 {
    if a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) {
        return 0.0;
    }
    if a.is_nan() || b.is_nan() || a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    let larger = a.abs().max(b.abs());
    let ulp = f64::from_bits(larger.to_bits() + 1) - larger;
    (a - b).abs() / ulp
}

/// The ulps the comparison of a case allows: 0 for native values, 4 for values interpolated
/// onto a grid and observables.
const ULPS: f64 = 4.0;

/// Compares the result of a scan with the `result` of its fixture; the largest distance of
/// the values in ulps.
fn compare_result(
    case: &str,
    result: &ScanResult,
    expected: &Json,
    ordered: &OrderedResult,
    derived: &dyn Fn(&str) -> bool,
    failures: &mut Vec<String>,
) -> f64 {
    let mut fail = |text: String| failures.push(format!("{case}: {text}"));
    let sizes: Vec<(String, usize)> = result
        .sizes
        .iter()
        .map(|s| (s.name.clone(), s.size))
        .collect();
    let want: Vec<(String, usize)> = expected["sizes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s[0].as_str().unwrap().to_string(),
                s[1].as_u64().unwrap() as usize,
            )
        })
        .collect();
    if sizes != want {
        fail(format!("sizes {sizes:?} != {want:?}"));
    }
    let attrs = &expected["attrs"];
    let dims: Vec<&str> = attrs["dims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    if result.dims != dims {
        fail(format!("dims {:?} != {dims:?}", result.dims));
    }
    let order = |key: &str| -> Vec<String> {
        expected["order"][key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    let coords: Vec<String> = result.coords.iter().map(|c| c.name.clone()).collect();
    if coords != order("coords") {
        fail(format!("coords {coords:?} != {:?}", order("coords")));
    }
    let variables: Vec<String> = result.variables.iter().map(|v| v.name.clone()).collect();
    if variables != order("variables") {
        fail(format!(
            "variables {variables:?} != {:?}",
            order("variables")
        ));
    }
    for coord in &result.coords {
        let want = &expected["coords"][&coord.name];
        if want.is_null() {
            continue;
        }
        let want_dims: Vec<&str> = want["dims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap())
            .collect();
        if coord.dims != want_dims {
            fail(format!(
                "coord {} dims {:?} != {want_dims:?}",
                coord.name, coord.dims
            ));
        }
        let want_unit = want["unit"].as_str().map(str::to_string);
        if coord.unit != want_unit {
            fail(format!(
                "coord {} unit {:?} != {want_unit:?}",
                coord.name, coord.unit
            ));
        }
        let values = want["values"].as_array().unwrap();
        let same = match &coord.values {
            CoordValues::Numbers(xs) => {
                xs.len() == values.len()
                    && xs
                        .iter()
                        .zip(values)
                        .all(|(x, v)| ulps(*x, number(v)) == 0.0)
            }
            CoordValues::Indices(xs) => {
                xs.len() == values.len()
                    && xs
                        .iter()
                        .zip(values)
                        .all(|(x, v)| v.as_u64() == Some(*x as u64))
            }
            CoordValues::Labels(xs) => {
                xs.len() == values.len()
                    && xs.iter().zip(values).all(|(x, v)| match (x, v) {
                        (Json::Number(a), Json::Number(b)) => a.as_f64() == b.as_f64(),
                        _ => x == v,
                    })
            }
        };
        if !same {
            fail(format!(
                "coord {} values {:?} != {values:?}",
                coord.name, coord.values
            ));
        }
    }
    let mut largest: f64 = 0.0;
    for variable in &result.variables {
        let want = &expected["variables"][&variable.name];
        if want.is_null() {
            continue;
        }
        let want_dims: Vec<&str> = want["dims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap())
            .collect();
        let want_shape: Vec<usize> = want["shape"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_u64().unwrap() as usize)
            .collect();
        if variable.dims != want_dims || variable.shape != want_shape {
            fail(format!(
                "variable {} {:?} {:?} != {want_dims:?} {want_shape:?}",
                variable.name, variable.dims, variable.shape
            ));
            continue;
        }
        if Some(variable.unit.as_str()) != want["unit"].as_str() {
            fail(format!(
                "variable {} unit {:?} != {}",
                variable.name, variable.unit, want["unit"]
            ));
        }
        let values = want["values"].as_array().unwrap();
        if variable.values.len() != values.len() {
            fail(format!(
                "variable {} has {} values",
                variable.name,
                variable.values.len()
            ));
            continue;
        }
        let allowed = if derived(&variable.name) { ULPS } else { 0.0 };
        let mut worst = (0.0, 0);
        for (k, (x, v)) in variable.values.iter().zip(values).enumerate() {
            let d = ulps(*x, number(v));
            if d > worst.0 {
                worst = (d, k);
            }
        }
        largest = largest.max(worst.0);
        if worst.0 > allowed {
            let k = worst.1;
            fail(format!(
                "variable {} [{k}]: {:?} != {:?} ({} ulps)",
                variable.name,
                variable.values[k],
                number(&values[k]),
                worst.0
            ));
        }
    }
    if result.units != ordered.attrs.units.0 {
        fail(format!(
            "units {:?} != {:?}",
            result.units, ordered.attrs.units.0
        ));
    }
    let changes: Vec<f64> = attrs["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(number)
        .collect();
    if result.changes.len() != changes.len()
        || result
            .changes
            .iter()
            .zip(&changes)
            .any(|(a, b)| ulps(*a, *b) != 0.0)
    {
        fail(format!("changes {:?} != {changes:?}", result.changes));
    }
    let errors: Option<Vec<String>> = attrs.get("errors").map(|e| {
        e.as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect()
    });
    let texts: Option<Vec<String>> = result
        .errors
        .as_ref()
        .map(|e| e.iter().map(FailedPoint::text).collect());
    if texts != errors {
        fail(format!("errors {texts:?} != {errors:?}"));
    }
    match (&result.block, attrs.get("block")) {
        (None, None) => {}
        (Some(block), Some(want)) => {
            let coords = &ordered
                .attrs
                .block
                .as_ref()
                .expect("the block of the fixture")
                .coords
                .0;
            let same = Some(block.start as u64) == want["start"].as_u64()
                && Some(block.stop as u64) == want["stop"].as_u64()
                && serde_json::to_value(&block.shape).unwrap() == want["shape"]
                && serde_json::to_value(&block.dims).unwrap() == want["dims"]
                && block.coords == *coords;
            if !same {
                fail(format!("block {block:?} != {want}"));
            }
        }
        (a, b) => fail(format!("block {a:?} != {b:?}")),
    }
    largest
}

#[test]
fn results_match_fixtures() {
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    let mut failures = Vec::new();
    let mut summary = Vec::new();
    for case in RESULT_CASES {
        let f = fixture(case);
        let text = std::fs::read_to_string(testdata().join(format!("{case}.json"))).unwrap();
        let file: Json = serde_json::from_str(&text).unwrap();
        let ordered: OrderedFixture = serde_json::from_str(&text).unwrap();
        let mut instance = runtime
            .load(&f.wasm, &f.description)
            .unwrap()
            .instantiate()
            .unwrap();
        let functions: BTreeMap<String, String> = f
            .request
            .observables
            .iter()
            .filter_map(|o| match o {
                sbmlsim_scan::Observable::Custom { id, function, .. } => {
                    Some((id.clone(), function.clone()))
                }
                _ => None,
            })
            .collect();
        // the observables and every value interpolated onto a grid of the run compare within
        // the tolerance, the native values by their bits
        let gridded = f.request.time.is_some();
        let observed: Vec<String> = f
            .request
            .observables
            .iter()
            .map(|o| o.id().to_string())
            .collect();
        let derived = |name: &str| {
            gridded
                || observed
                    .iter()
                    .any(|id| name == id || name.starts_with(&format!("{id}.")))
        };
        let outcome = Scan::open(f.request.clone(), f.description.clone()).and_then(|mut scan| {
            let rounds = run_with_host(&mut scan, &mut instance, &functions)?;
            compare_rounds(case, &rounds, &f.rounds, &mut failures);
            scan.result()
        });
        match (outcome, file["error"].is_null()) {
            (Ok(result), true) => {
                assert_eq!(result.contract, sbmlsim_scan::RESULT_CONTRACT);
                let ordered = ordered.result.as_ref().expect("the result of the fixture");
                let largest = compare_result(
                    case,
                    &result,
                    &file["result"],
                    ordered,
                    &derived,
                    &mut failures,
                );
                summary.push(format!("{case}: {largest} ulps"));
            }
            (Err(err), false) => {
                let want = &file["error"];
                if want["type"] != "ScanError"
                    || err.kind != ScanErrorKind::Point
                    || Some(err.message.as_str()) != want["message"].as_str()
                {
                    failures.push(format!("{case}: error {err:?} != {want}"));
                }
            }
            (Ok(_), false) => failures.push(format!("{case}: no error, the fixture has one")),
            (Err(err), true) => failures.push(format!("{case}: {err}")),
        }
    }
    println!("{}", summary.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_failed_host_function_fails_its_point_alone() {
    let f = fixture("custom_observable");
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    let mut instance = runtime
        .load(&f.wasm, &f.description)
        .unwrap()
        .instantiate()
        .unwrap();
    let mut request = f.request.clone();
    request.on_error = sbmlsim_scan::OnError::Flag;
    let mut scan = Scan::open(request, f.description.clone()).unwrap();
    let mut asked = 0;
    loop {
        match scan.ask() {
            Ask::Simulate { requests, .. } => {
                let answers = requests
                    .iter()
                    .map(|r| instance.simulate(r).map_err(PointError::from))
                    .collect();
                scan.tell(answers).unwrap();
            }
            Ask::Observe(question) => {
                asked += 1;
                assert_eq!(question.observables, ["auc", "twice"]);
                assert_eq!(question.points.len(), 2);
                let values = question
                    .observables
                    .iter()
                    .map(|id| {
                        let n = |p: &sbmlsim_scan::HostPoint| match id.as_str() {
                            "auc" => 1,
                            _ => p.time.len(),
                        };
                        (
                            id.clone(),
                            question.points.iter().map(|p| vec![1.0; n(p)]).collect(),
                        )
                    })
                    .collect();
                scan.tell_observed(HostObserved {
                    values,
                    errors: vec![sbmlsim_scan::HostError {
                        observable: "auc".to_string(),
                        index: 1,
                        message: "ValueError: boom".to_string(),
                    }],
                })
                .unwrap();
            }
            Ask::Done => break,
        }
    }
    assert_eq!(asked, 1);
    let result = scan.result().unwrap();
    let errors = result.errors.unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].index, 1);
    assert_eq!(
        errors[0].text(),
        "rate=1, k=0.3: The function of the observable 'auc' failed: ValueError: boom"
    );
    let variable = |name: &str| result.variables.iter().find(|v| v.name == name).unwrap();
    assert_eq!(variable("status").values, [0.0, 1.0]);
    assert_eq!(variable("auc").values[0], 1.0);
    assert!(variable("auc").values[1].is_nan());
    assert!(variable("twice").values[11..].iter().all(|x| x.is_nan()));
}

#[test]
fn an_output_of_other_rows_than_the_shared_grid_fails_its_point() {
    let f = fixture("times_output");
    let mut request = f.request.clone();
    request.on_error = sbmlsim_scan::OnError::Flag;
    let mut scan = Scan::open(request, f.description.clone()).unwrap();
    rounds_of(&mut scan);
    let result = scan.result().unwrap();
    let errors = result.errors.unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "RuntimeError: The output of the point has 0 rows, not the 7 output times the \
         simulations of the scan share."
    );
    let status = result
        .variables
        .iter()
        .find(|v| v.name == "status")
        .unwrap();
    assert_eq!(status.values, [1.0]);
}
