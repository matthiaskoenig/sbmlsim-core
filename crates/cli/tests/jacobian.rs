//! The analytic Jacobians of the compiled models of the SBML Test Suite against central
//! differences of their right hand sides along the trajectories, the conservative patterns, and
//! the linear solvers with both Jacobians.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use sbmlsim_abi::{
    EntityKind, IntegratorSettings, JacobianChoice, JacobianKind, LinearSolver, ModelDescription,
    Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult, Tolerance,
};
use sbmlsim_cli::testsuite::{Baseline, CaseSettings, case_request};
use sbmlsim_compiler::{CompileOptions, CompiledModel, compile};
use sbmlsim_host::{ModelFunctions, Runtime};

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

/// The directories of the cases which pass (not expected to fail by the baseline), `None`
/// without prepared cases.
fn passing_cases() -> Option<Vec<PathBuf>> {
    let dir = testdata().join("cases");
    if !dir.join("00001/case.json").exists() {
        eprintln!(
            "note: {} is absent, run `uv run scripts/prepare_testsuite.py`; skipped",
            dir.display()
        );
        return None;
    }
    let baseline: Baseline =
        serde_json::from_slice(&std::fs::read(testdata().join("testsuite_baseline.json")).unwrap())
            .unwrap();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("case.json").is_file())
        .filter(|p| {
            let cid = p.file_name().unwrap().to_string_lossy();
            !baseline.expected_failures.contains_key(cid.as_ref())
        })
        .collect();
    dirs.sort();
    assert_eq!(dirs.len(), baseline.n_passed, "the passing cases");
    Some(dirs)
}

/// The selection of the value an entity stores: `[S]` for a species which stores its
/// concentration, else its id.
fn stored_selection(e: &sbmlsim_abi::Entity) -> String {
    match &e.kind {
        EntityKind::Species {
            stores_concentration: true,
            ..
        } => format!("[{}]", e.id),
        _ => e.id.clone(),
    }
}

/// `(t, y, p)` at `n` rows of the trajectory of a case, evenly spread from the first to the last.
fn trajectory(
    compiled: &CompiledModel,
    settings: &CaseSettings,
    n: usize,
) -> Vec<(f64, Vec<f64>, Vec<f64>)> {
    let desc = &compiled.description;
    let mut req = case_request(settings);
    req.selections = std::iter::once("time".to_string())
        .chain(desc.states.iter().map(stored_selection))
        .chain(desc.parameters.iter().map(stored_selection))
        .collect();
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let result = instance.simulate(&req).unwrap();
    let k = result.columns.len();
    let (ns, np) = (desc.states.len(), desc.parameters.len());
    (0..n)
        .map(|r| {
            (r * (result.n_rows - 1))
                .div_ceil(n.max(2) - 1)
                .min(result.n_rows - 1)
        })
        .map(|row| {
            let values = &result.values[row * k..(row + 1) * k];
            (
                values[0],
                values[1..1 + ns].to_vec(),
                values[1 + ns..1 + ns + np].to_vec(),
            )
        })
        .collect()
}

/// The differences of the right hand side in the direction of one state with one step: central,
/// forward and backward, and Richardson's extrapolation of central differences of a step `1e-2`
/// of the scale (fourth order, for a rate whose evaluation cancels: `(1 + x) - 1` of a tiny `x`
/// is a staircase at the scale of a small step, case 01395).
struct Differences {
    central: Vec<f64>,
    forward: Vec<f64>,
    backward: Vec<f64>,
    richardson: Vec<f64>,
}

/// The differences of every column (state) with the steps `cbrt(eps) * max(|y_j|, s)` for the
/// scales `s` of `SCALES`, and the right hand sides at the two points of the first step (to tell
/// an entry which no difference can see).
struct Columns {
    differences: Vec<Vec<Differences>>,
    plus: Vec<Vec<f64>>,
    minus: Vec<Vec<f64>>,
}

/// The scales of the steps of the differences: a state near 0 has no scale of its own.
const SCALES: [f64; 3] = [1e-6, 1e-3, 1.0];

fn differences(model: &mut ModelFunctions, t: f64, y: &[f64], p: &[f64]) -> Columns {
    let n = y.len();
    let f0 = model.rhs(t, y, p).unwrap();
    let (mut plus, mut minus, mut differences) = (Vec::new(), Vec::new(), Vec::new());
    for j in 0..n {
        let mut column = Vec::new();
        for (k, s) in SCALES.iter().enumerate() {
            let h = f64::EPSILON.cbrt() * y[j].abs().max(*s);
            let mut z = y.to_vec();
            z[j] = y[j] + h;
            let fp = model.rhs(t, &z, p).unwrap();
            z[j] = y[j] - h;
            let fm = model.rhs(t, &z, p).unwrap();
            let r = 1e-2 * y[j].abs().max(*s);
            let mut at = |dx: f64| {
                z[j] = y[j] + dx;
                model.rhs(t, &z, p).unwrap()
            };
            let (r1, r_1, r2, r_2) = (at(r), at(-r), at(2.0 * r), at(-2.0 * r));
            column.push(Differences {
                central: (0..n).map(|i| (fp[i] - fm[i]) / (2.0 * h)).collect(),
                forward: (0..n).map(|i| (fp[i] - f0[i]) / h).collect(),
                backward: (0..n).map(|i| (f0[i] - fm[i]) / h).collect(),
                richardson: (0..n)
                    .map(|i| (8.0 * (r1[i] - r_1[i]) - (r2[i] - r_2[i])) / (12.0 * r))
                    .collect(),
            });
            if k == 0 {
                plus.push(fp);
                minus.push(fm);
            }
        }
        differences.push(column);
    }
    Columns {
        differences,
        plus,
        minus,
    }
}

/// What the comparison of one case found: the entries compared, the failures and the entries
/// outside the pattern which a difference sees.
#[derive(Default)]
struct Found {
    compared: usize,
    failures: Vec<String>,
    outside: Vec<String>,
}

fn check_case(dir: &Path, rows: usize, compare_values: bool) -> Found {
    let cid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let settings: CaseSettings =
        serde_json::from_slice(&std::fs::read(dir.join("case.json")).unwrap()).unwrap();
    let sbml = std::fs::read_to_string(dir.join("model.xml")).unwrap();
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    let desc: &ModelDescription = &compiled.description;
    let mut found = Found::default();
    let n = desc.states.len();
    if n == 0 {
        return found;
    }
    let analytic = matches!(desc.jacobian, JacobianKind::Analytic(_));
    let pattern = desc.jacobian.pattern();
    let mut model = runtime()
        .model_functions(&compiled.wasm, &compiled.description)
        .unwrap();
    for (t, y, p) in trajectory(&compiled, &settings, rows) {
        let c = differences(&mut model, t, &y, &p);
        // every entry a difference sees is in the pattern: the right hand sides of a row at the
        // two points of a column differ only where the row depends on the state of the column
        for j in 0..n {
            let rows = pattern.rows(j);
            for i in 0..n {
                let (a, b) = (c.plus[j][i], c.minus[j][i]);
                let same = a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
                if !same && !rows.contains(&(i as u32)) {
                    found.outside.push(format!(
                        "{cid}: d {} / d {} at t = {t} is not in the pattern ({a:e}, {b:e})",
                        desc.states[i].id, desc.states[j].id
                    ));
                }
            }
        }
        if !(compare_values && analytic) {
            continue;
        }
        let values = model.jac(t, &y, &p).unwrap();
        for j in 0..n {
            let start = pattern.col_ptr[j] as usize;
            for (k, &i) in pattern.rows(j).iter().enumerate() {
                let i = i as usize;
                let a = values[start + k];
                found.compared += 1;
                let close = |d: f64| {
                    (a - d).abs() <= 1e-5 * a.abs().max(d.abs())
                        || (a - d).abs() <= 1e-10
                        || (a.is_nan() && d.is_nan())
                        || a == d
                };
                // the central difference or the extrapolation of one of the steps agrees; where
                // the rate has a kink
                // or a jump between the two points (a switch of a piecewise, abs at 0, ceiling at
                // an integer) the derivative of a side, which a one-sided difference gives
                let agree = c.differences[j].iter().any(|d| {
                    close(d.central[i])
                        || close(d.forward[i])
                        || close(d.backward[i])
                        || close(d.richardson[i])
                });
                let d = c.differences[j][0].central[i];
                if !agree {
                    found.failures.push(format!(
                        "{cid}: d {} / d {} at t = {t}, y = {y:?}: analytic {a:e}, central \
                         difference {d:e}",
                        desc.states[i].id, desc.states[j].id
                    ));
                }
            }
        }
    }
    found
}

/// Runs `check_case` on every passing case and gathers what it found.
fn check_all(compare_values: bool) -> Option<Found> {
    let dirs = passing_cases()?;
    // the runtime is compiled before the threads of rayon wait for it: wasmtime compiles on
    // the threads of rayon itself
    runtime();
    let all: Vec<Found> = dirs
        .par_iter()
        .map(|dir| check_case(dir, 5, compare_values))
        .collect();
    let mut found = Found::default();
    for f in all {
        found.compared += f.compared;
        found.failures.extend(f.failures);
        found.outside.extend(f.outside);
    }
    Some(found)
}

#[test]
fn jacobian_matches_difference_on_all_cases() {
    // the analytic values against central differences at 5 states along the trajectory of every
    // passing case, within 1e-5 relative or 1e-10 absolute
    let Some(found) = check_all(true) else { return };
    eprintln!("{} entries compared", found.compared);
    assert!(
        found.compared > 10_000,
        "{} entries compared",
        found.compared
    );
    assert!(
        found.failures.is_empty(),
        "{} of {} entries differ:\n{}",
        found.failures.len(),
        found.compared,
        found.failures[..found.failures.len().min(30)].join("\n")
    );
}

#[test]
fn pattern_is_conservative() {
    // every nonzero of a dense central difference Jacobian lies in the pattern, on every
    // passing case at 5 states along its trajectory
    let Some(found) = check_all(false) else {
        return;
    };
    assert!(
        found.outside.is_empty(),
        "{} entries outside the pattern:\n{}",
        found.outside.len(),
        found.outside[..found.outside.len().min(30)].join("\n")
    );
}

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// Robertson's problem (`cvRoberts_dns`) as SBML: three species of amounts and three reactions,
/// the rate of the third read through an assignment rule chain.
fn robertson_sbml() -> String {
    let ci = |x: &str| format!("<ci>{x}</ci>");
    let times = |args: &[String]| format!("<apply><times/>{}</apply>", args.concat());
    let reaction = |id: &str, from: &str, to: &str, law: String| {
        format!(
            r#"<reaction id="{id}" reversible="false"><listOfReactants><speciesReference species="{from}" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="{to}" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math {MATH}>{law}</math></kineticLaw></reaction>"#
        )
    };
    let species = |id: &str, x: f64| {
        format!(
            r#"<species id="{id}" compartment="C" initialAmount="{x}" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/>"#
        )
    };
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="robertson">
<listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments>
<listOfSpecies>{}{}{}</listOfSpecies>
<listOfParameters><parameter id="k1" value="0.04" constant="true"/><parameter id="k2" value="3e7" constant="true"/><parameter id="k3" value="1e4" constant="true"/><parameter id="a" constant="false"/><parameter id="b" constant="false"/></listOfParameters>
<listOfRules><assignmentRule variable="b"><math {MATH}>{}</math></assignmentRule><assignmentRule variable="a"><math {MATH}>{}</math></assignmentRule></listOfRules>
<listOfReactions>{}{}{}</listOfReactions>
</model></sbml>"#,
        species("y1", 1.0),
        species("y2", 0.0),
        species("y3", 0.0),
        times(&[ci("y2"), ci("y3")]),
        times(&[ci("k3"), ci("b")]),
        reaction("R1", "y1", "y2", times(&[ci("k1"), ci("y1")])),
        reaction("R2", "y2", "y3", times(&[ci("k2"), ci("y2"), ci("y2")])),
        reaction("R3", "y2", "y1", ci("a")),
    )
}

/// The integrator steps of Robertson to `4e10` with a linear solver.
fn robertson(choice: JacobianChoice, solver: LinearSolver) -> SimulationResult {
    let compiled = compile(
        &robertson_sbml(),
        &CompileOptions {
            jacobian: choice,
            ..CompileOptions::default()
        },
    )
    .unwrap();
    let analytic = matches!(compiled.description.jacobian, JacobianKind::Analytic(_));
    assert_eq!(analytic, choice != JacobianChoice::DifferenceQuotient);
    let req = SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 4e10,
        output: Output::IntegratorSteps,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: ["time", "y1", "y2", "y3"].map(String::from).to_vec(),
        settings: IntegratorSettings {
            relative_tolerance: 1e-8,
            absolute_tolerance: Tolerance::Scalar(1e-14),
            linear_solver: solver,
            ..IntegratorSettings::default()
        },
        seed: 0,
    };
    runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
        .simulate(&req)
        .unwrap()
}

/// The rows of two runs of the integrator steps up to the first row of another time (the step
/// sequences part there), each value within `1e-10` relative; the number of rows compared.
fn same_steps(a: &SimulationResult, b: &SimulationResult, what: &str) -> usize {
    let k = a.columns.len();
    let mut rows = 0;
    for r in 0..a.n_rows.min(b.n_rows) {
        let (ra, rb) = (&a.values[r * k..(r + 1) * k], &b.values[r * k..(r + 1) * k]);
        if ra[0] != rb[0] {
            break;
        }
        for (x, y) in ra.iter().zip(rb) {
            assert!(
                x == y || (x - y).abs() <= 1e-10 * x.abs().max(y.abs()),
                "{what}: row {r} differs: {ra:?} {rb:?}"
            );
        }
        rows += 1;
    }
    rows
}

#[test]
fn klu_and_dense_agree_on_an_analytic_model() {
    // within 1e-10 relative while the step counts agree (the integrator
    // steps have the same times), with the analytic Jacobian and with the colored difference
    // quotient (the two Jacobians differ in the last bits, so their step sequences part after a
    // few steps; each run ends within 1e-4 of the reference y1(4e10) of `cvRoberts_dns`)
    for choice in [JacobianChoice::Analytic, JacobianChoice::DifferenceQuotient] {
        let dense = robertson(choice, LinearSolver::Dense);
        let klu = robertson(choice, LinearSolver::Klu);
        let rows = same_steps(&dense, &klu, &format!("{choice:?}"));
        eprintln!(
            "{choice:?}: {rows} of {} and {} steps agree",
            dense.n_rows, klu.n_rows
        );
        assert!(
            rows > 50,
            "{choice:?}: {rows} of {} and {} steps agree",
            dense.n_rows,
            klu.n_rows
        );
        for r in [&dense, &klu] {
            let last = &r.values[(r.n_rows - 1) * 4..];
            assert_eq!(last[0], 4e10);
            let y1 = last[1];
            assert!(
                (y1 - 5.2083e-08).abs() <= 1e-4 * 5.2083e-08,
                "{choice:?}: y1(4e10) = {y1:e}"
            );
        }
    }
}

/// A model of species of amounts (`id`, initial amount) and reactions which consume one species
/// each at a rate (MathML), without products.
fn consumed(species: &[(&str, f64)], reactions: &[(&str, &str)], parameters: &str) -> String {
    let species: String = species
        .iter()
        .map(|(id, x)| {
            format!(
                r#"<species id="{id}" compartment="C" initialAmount="{x}" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/>"#
            )
        })
        .collect();
    let reactions: String = reactions
        .iter()
        .enumerate()
        .map(|(k, (s, law))| {
            format!(
                r#"<reaction id="R{k}" reversible="false"><listOfReactants><speciesReference species="{s}" stoichiometry="1" constant="true"/></listOfReactants><kineticLaw><math {MATH}>{law}</math></kineticLaw></reaction>"#
            )
        })
        .collect();
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m"><listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments><listOfSpecies>{species}</listOfSpecies><listOfParameters>{parameters}</listOfParameters><listOfReactions>{reactions}</listOfReactions></model></sbml>"#
    )
}

/// The analytic Jacobian of a model has an entry which is not finite at its initial state, and
/// the simulation with it (`Auto`) integrates and agrees with the colored difference quotient
/// within the tolerances of the integration.
fn falls_back(sbml: &str, selections: &[&str]) {
    let analytic = compile(sbml, &CompileOptions::default()).unwrap();
    assert!(matches!(
        analytic.description.jacobian,
        JacobianKind::Analytic(_)
    ));
    let desc = &analytic.description;
    let y: Vec<f64> = desc.states.iter().map(|e| e.initial).collect();
    let p: Vec<f64> = desc.parameters.iter().map(|e| e.initial).collect();
    let entries = runtime()
        .model_functions(&analytic.wasm, desc)
        .unwrap()
        .jac(0.0, &y, &p)
        .unwrap();
    assert!(entries.iter().any(|v| !v.is_finite()), "{entries:?}");
    let difference = compile(
        sbml,
        &CompileOptions {
            jacobian: JacobianChoice::DifferenceQuotient,
            ..CompileOptions::default()
        },
    )
    .unwrap();
    let req = SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 10.0,
        output: Output::Uniform { steps: 10 },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: selections.iter().map(|s| s.to_string()).collect(),
        settings: IntegratorSettings {
            relative_tolerance: 1e-8,
            absolute_tolerance: Tolerance::Scalar(1e-12),
            ..IntegratorSettings::default()
        },
        seed: 0,
    };
    let simulate = |c: &CompiledModel, solver: LinearSolver| {
        let mut req = req.clone();
        req.settings.linear_solver = solver;
        runtime()
            .instantiate(&c.wasm, &c.description)
            .unwrap()
            .simulate(&req)
            .unwrap()
    };
    for solver in [LinearSolver::Dense, LinearSolver::Klu] {
        let (a, d) = (simulate(&analytic, solver), simulate(&difference, solver));
        assert_eq!(a.n_rows, 11);
        for (x, y) in a.values.iter().zip(&d.values) {
            assert!(
                x.is_finite() && (x - y).abs() <= 1e-6 * y.abs() + 1e-10,
                "{solver:?}: {:?} {:?}",
                a.values,
                d.values
            );
        }
    }
}

#[test]
fn an_infinite_derivative_at_a_finite_state_falls_back_to_the_difference_quotient() {
    // S(0) = 0 consumed at k * sqrt(S), whose analytic derivative
    // 0.5 * pow(S, -0.5) is infinite at S = 0, and an unrelated decay of Q; and P(0) = 0
    // consumed at pow(P, Q), whose derivative pow(P, Q) * ln(P) by Q is NaN at P = 0
    let sbml = consumed(
        &[("S", 0.0), ("Q", 1.0), ("P", 0.0)],
        &[
            (
                "S",
                "<apply><times/><ci>k</ci><apply><root/><ci>S</ci></apply></apply>",
            ),
            ("Q", "<apply><times/><ci>k</ci><ci>Q</ci></apply>"),
            ("P", "<apply><power/><ci>P</ci><ci>Q</ci></apply>"),
        ],
        r#"<parameter id="k" value="0.5" constant="true"/>"#,
    );
    falls_back(&sbml, &["time", "S", "Q", "P"]);
}

#[test]
fn a_power_0_at_0_falls_back_to_the_difference_quotient() {
    // x^0 at x = 0: the derivative 0 * pow(0, -1) is NaN; S(0) = 0 is consumed at k * S^0 * S,
    // so it stays at 0 and every Jacobian has the NaN
    let sbml = consumed(
        &[("S", 0.0), ("Q", 1.0)],
        &[
            (
                "S",
                "<apply><times/><ci>k</ci><apply><power/><ci>S</ci><cn>0</cn></apply><ci>S</ci></apply>",
            ),
            ("Q", "<apply><times/><ci>k</ci><ci>Q</ci></apply>"),
        ],
        r#"<parameter id="k" value="0.5" constant="true"/>"#,
    );
    falls_back(&sbml, &["time", "S", "Q"]);
}
