//! A problem of PEtab v2 in memory, evaluated against its analytic solution and fitted.
//!
//! The model is the conversion `A <-> B` of first order, whose solution from `A(0) = a`,
//! `B(0) = 0` is `A(t) = a (k2 + k1 exp(-(k1 + k2) t)) / (k1 + k2)`. The problem has two
//! experiments, one of a condition which reads a fixed parameter and one of a change after the
//! start which reads the state (`A + 1`), an observable with a placeholder whose value is an
//! estimated parameter, a log-normal noise, an estimated noise and a prior.

use std::collections::HashMap;

use sbmlsim_fit::Settings;
use sbmlsim_petab::{Executor, InMemory, Options, PetabError, PetabProblem, Plan};

const MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="conversion">
    <listOfCompartments>
      <compartment id="c" size="1" constant="true"/>
    </listOfCompartments>
    <listOfSpecies>
      <species id="A" compartment="c" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
      <species id="B" compartment="c" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
    </listOfSpecies>
    <listOfParameters>
      <parameter id="k1" value="0.8" constant="true"/>
      <parameter id="k2" value="0.6" constant="true"/>
    </listOfParameters>
    <listOfReactions>
      <reaction id="forward" reversible="false">
        <listOfReactants><speciesReference species="A" stoichiometry="1" constant="true"/></listOfReactants>
        <listOfProducts><speciesReference species="B" stoichiometry="1" constant="true"/></listOfProducts>
        <kineticLaw><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k1</ci><ci>A</ci></apply></math></kineticLaw>
      </reaction>
      <reaction id="backward" reversible="false">
        <listOfReactants><speciesReference species="B" stoichiometry="1" constant="true"/></listOfReactants>
        <listOfProducts><speciesReference species="A" stoichiometry="1" constant="true"/></listOfProducts>
        <kineticLaw><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k2</ci><ci>B</ci></apply></math></kineticLaw>
      </reaction>
    </listOfReactions>
  </model>
</sbml>"#;

const YAML: &str = "format_version: 2.0.0
parameter_files: [parameters.tsv]
model_files:
  conversion: {location: model.xml, language: sbml}
condition_files: [conditions.tsv]
experiment_files: [experiments.tsv]
observable_files: [observables.tsv]
measurement_files: [measurements.tsv]
";

const CONDITIONS: &str = "conditionId\ttargetId\ttargetValue
low\tA\t1
high\tA\ta_high
dose\tA\tA + 1
";

const EXPERIMENTS: &str = "experimentId\ttime\tconditionId
e1\t0\tlow
e2\t0\thigh
e2\t5\tdose
";

const OBSERVABLES: &str =
    "observableId\tobservableFormula\tnoiseFormula\tnoiseDistribution\tobservablePlaceholders
obs_a\tA\tsigma_a\tnormal\t
obs_b\tscale * B\t0.1\tlog-normal\tscale
";

/// The measurements as `(observable, experiment, time, scale)`.
const MEASURED: [(&str, &str, f64, &str); 12] = [
    ("obs_a", "e1", 0.0, ""),
    ("obs_a", "e1", 1.0, ""),
    ("obs_a", "e1", 2.0, ""),
    ("obs_a", "e1", 10.0, ""),
    ("obs_b", "e1", 1.0, "2"),
    ("obs_b", "e1", 10.0, "2"),
    ("obs_a", "e2", 0.0, ""),
    ("obs_a", "e2", 2.0, ""),
    ("obs_a", "e2", 5.0, ""),
    ("obs_a", "e2", 7.0, ""),
    ("obs_b", "e2", 5.0, "scale_b"),
    ("obs_b", "e2", 10.0, "scale_b"),
];

fn measurements(values: &[f64]) -> String {
    let mut table =
        "observableId\texperimentId\ttime\tmeasurement\tobservableParameters\n".to_string();
    for ((observable, experiment, time, scale), value) in MEASURED.iter().zip(values) {
        table.push_str(&format!(
            "{observable}\t{experiment}\t{time}\t{value}\t{scale}\n"
        ));
    }
    table
}

fn files(parameters: &str, values: &[f64]) -> InMemory {
    let mut files: InMemory = HashMap::new();
    for (name, text) in [
        ("problem.yaml", YAML.to_string()),
        ("model.xml", MODEL.to_string()),
        ("conditions.tsv", CONDITIONS.to_string()),
        ("experiments.tsv", EXPERIMENTS.to_string()),
        ("observables.tsv", OBSERVABLES.to_string()),
        ("measurements.tsv", measurements(values)),
        ("parameters.tsv", parameters.to_string()),
    ] {
        files.insert(name.to_string(), text.into_bytes());
    }
    files
}

const PARAMETERS: &str = "parameterId\tlowerBound\tupperBound\tnominalValue\testimate\tpriorDistribution\tpriorParameters
k1\t0.001\t10\t0.8\ttrue\tnormal\t1;0.5
k2\t0.001\t10\t0.6\ttrue\t\t
sigma_a\t0.01\t1\t0.05\ttrue\t\t
scale_b\t0.1\t10\t2\ttrue\t\t
a_high\t\t\t2\tfalse\t\t
";

fn executor(parameters: &str, values: &[f64]) -> Executor {
    let problem = PetabProblem::read("problem.yaml", &files(parameters, values)).unwrap();
    let plan = Plan::new(&problem, &Options::default()).unwrap();
    Executor::new(plan).unwrap()
}

/// `A(t)` of the conversion from `A(0) = a`, `B(0) = b`.
fn analytic(a: f64, b: f64, k1: f64, k2: f64, t: f64) -> f64 {
    let total = a + b;
    let equilibrium = total * k2 / (k1 + k2);
    equilibrium + (a - equilibrium) * (-(k1 + k2) * t).exp()
}

/// The values of the measurements of the analytic solution at the nominal parameters.
fn exact(k1: f64, k2: f64, scale_b: f64) -> Vec<f64> {
    MEASURED
        .iter()
        .map(|(observable, experiment, time, _)| {
            let (a, b) = match (*experiment, *time > 5.0) {
                ("e1", _) => (analytic(1.0, 0.0, k1, k2, *time), None),
                (_, false) if *time < 5.0 => (analytic(2.0, 0.0, k1, k2, *time), None),
                _ => {
                    // the dose at 5 adds 1 to A, the measurement at 5 reads the state after it
                    let a5 = analytic(2.0, 0.0, k1, k2, 5.0);
                    let b5 = 2.0 - a5;
                    let a = analytic(a5 + 1.0, b5, k1, k2, *time - 5.0);
                    (a, Some(3.0 - a))
                }
            };
            match *observable {
                "obs_a" => a,
                _ => {
                    let b = b.unwrap_or(match *experiment {
                        "e1" => 1.0 - a,
                        _ => 2.0 - a,
                    });
                    let scale = if *experiment == "e1" { 2.0 } else { scale_b };
                    scale * b
                }
            }
        })
        .collect()
}

#[test]
fn the_simulations_are_the_analytic_solution() {
    let truth = exact(0.8, 0.6, 2.0);
    let executor = executor(PARAMETERS, &truth);
    let x = executor.plan().nominal();
    let evaluation = executor.evaluate(&x);
    assert!(evaluation.failures.is_empty(), "{:?}", evaluation.failures);
    for (k, (simulated, expected)) in evaluation.simulated.iter().zip(&truth).enumerate() {
        assert!(
            (simulated - expected).abs() <= 1e-6 * expected.abs().max(1.0),
            "measurement {k}: {simulated} != {expected}"
        );
    }
    // the data is the simulation: every residual is zero, the log-likelihood is the sum of the
    // normalizations of the densities
    assert!(evaluation.chi2 < 1e-6, "{}", evaluation.chi2);
    let normalization: f64 = MEASURED
        .iter()
        .zip(&truth)
        .map(|((observable, ..), value)| match *observable {
            "obs_a" => -0.5 * (2.0 * std::f64::consts::PI * 0.05f64.powi(2)).ln(),
            _ => -0.5 * (2.0 * std::f64::consts::PI * 0.01f64 * value * value).ln(),
        })
        .sum();
    assert!(
        (evaluation.llh - normalization).abs() < 1e-3,
        "{} != {normalization}",
        evaluation.llh
    );
    // the prior of k1 and the uniform priors of PEtab of the others
    let priors: HashMap<&str, f64> = evaluation
        .log_priors
        .iter()
        .map(|(id, value)| (id.as_str(), *value))
        .collect();
    assert_eq!(priors.len(), 4);
    assert!((priors["k2"] + (10.0f64 - 0.001).ln()).abs() < 1e-15);
}

#[test]
fn the_cost_of_the_fit_is_the_negative_log_posterior() {
    let mut data = exact(0.8, 0.6, 2.0);
    // data which is not the simulation, so that the residuals are not zero
    for (k, value) in data.iter_mut().enumerate() {
        *value *= 1.0 + 0.03 * ((k % 3) as f64 - 1.0);
    }
    let executor = executor(PARAMETERS, &data);
    let x = executor.plan().nominal();
    let evaluation = executor.evaluate(&x);
    let problem = executor.problem(2).unwrap();
    let start = problem.start();
    let cost = problem.cost(&start).unwrap();
    let offset = executor.cost_offset().unwrap();
    let posterior = evaluation.unnorm_log_posterior();
    assert!(
        (cost - offset + posterior).abs() < 1e-6 * posterior.abs().max(1.0),
        "cost {cost} - offset {offset} != -log posterior {}",
        -posterior
    );
}

#[test]
fn a_fit_finds_the_parameters_of_the_data() {
    // the data of k1 = 0.5, k2 = 0.3 and scale_b = 3, fitted from the nominal values without
    // a prior and with a fixed noise
    let parameters = "parameterId\tlowerBound\tupperBound\tnominalValue\testimate
k1\t0.001\t10\t0.8\ttrue
k2\t0.001\t10\t0.6\ttrue
sigma_a\t0.01\t1\t0.05\tfalse
scale_b\t0.1\t10\t2\ttrue
a_high\t\t\t2\tfalse
";
    let executor = executor(parameters, &exact(0.5, 0.3, 3.0));
    let settings = Settings {
        max_iterations: 200,
        ..Settings::default()
    };
    let answer = executor.fit(&settings, 2).unwrap();
    for (value, expected) in answer.x.iter().zip([0.5, 0.3, 3.0]) {
        assert!((value - expected).abs() < 1e-4, "{:?}", answer.x);
    }
    let evaluation = executor.evaluate(&answer.x);
    assert!(
        (answer.objective + evaluation.llh).abs() < 1e-6,
        "{} {}",
        answer.objective,
        evaluation.llh
    );
}

#[test]
fn a_problem_whose_parts_do_not_fit_is_refused() {
    let mut files = files(PARAMETERS, &exact(0.8, 0.6, 2.0));
    files.insert(
        "observables.tsv".to_string(),
        b"observableId\tobservableFormula\tnoiseFormula\nobs_a\tA + unknown\t1\nobs_b\tB\t1\n"
            .to_vec(),
    );
    let problem = PetabProblem::read("problem.yaml", &files).unwrap();
    let error = Plan::new(&problem, &Options::default()).unwrap_err();
    assert!(matches!(error, PetabError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("'unknown'"), "{error}");
}

#[test]
fn every_measurement_is_simulated_with_its_model() {
    // two builds of the model, the second of a faster forward reaction, one experiment each
    let mut files = files(PARAMETERS, &exact(0.8, 0.6, 2.0));
    files.insert(
        "problem.yaml".to_string(),
        YAML.replace(
            "  conversion: {location: model.xml, language: sbml}",
            "  slow: {location: model.xml, language: sbml}\n  fast: {location: fast.xml, language: sbml}",
        )
        .into_bytes(),
    );
    files.insert(
        "fast.xml".to_string(),
        MODEL
            .replace(r#"<parameter id="k2" value="0.6" constant="true"/>"#, r#"<parameter id="k2" value="0.6" constant="true"/><parameter id="boost" value="3" constant="true"/>"#)
            .replace("<ci>k1</ci><ci>A</ci>", "<ci>boost</ci><ci>k1</ci><ci>A</ci>")
            .into_bytes(),
    );
    let mut table = String::from("modelId\tobservableId\texperimentId\ttime\tmeasurement\n");
    for (model, time) in [("slow", 1.0), ("slow", 2.0), ("fast", 1.0), ("fast", 2.0)] {
        table.push_str(&format!("{model}\tobs_a\te1\t{time}\t0.5\n"));
    }
    files.insert("measurements.tsv".to_string(), table.into_bytes());
    let problem = PetabProblem::read("problem.yaml", &files).unwrap();
    let plan = Plan::new(&problem, &Options::default()).unwrap();
    assert_eq!(plan.simulations.len(), 2);
    let executor = Executor::new(plan).unwrap();
    let evaluation = executor.evaluate(&executor.plan().nominal());
    for (k, (model, time)) in [(0.8, 1.0), (0.8, 2.0), (2.4, 1.0), (2.4, 2.0)]
        .iter()
        .enumerate()
    {
        let expected = analytic(1.0, 0.0, *model, 0.6, *time);
        assert!(
            (evaluation.simulated[k] - expected).abs() < 1e-6,
            "{k}: {}",
            evaluation.simulated[k]
        );
    }
    assert!(
        executor
            .plan()
            .simulation_table(&evaluation)
            .starts_with("modelId\t")
    );
    // the fit reads both models
    let problem = executor.problem(2).unwrap();
    let cost = problem.cost(&problem.start()).unwrap();
    let posterior = evaluation.unnorm_log_posterior();
    assert!((cost - executor.cost_offset().unwrap() + posterior).abs() < 1e-6);
}

/// The log-likelihood of the problem with the extension `sbmlsim` of a role, in which `obs_b`
/// is validation data.
fn llh_of_role(role: Option<&str>) -> f64 {
    // data off the solution, so every measurement has a residual of its own
    let values: Vec<f64> = exact(0.8, 0.6, 2.0).iter().map(|v| 1.1 * v).collect();
    let mut files = files(PARAMETERS, &values);
    if let Some(role) = role {
        let extension = format!(
            "extensions:\n  sbmlsim:\n    version: 0.5.0\n    required: false\n    role: {role}\n    observables:\n      obs_a: {{kind: training}}\n      obs_b: {{kind: validation}}\n"
        );
        files.insert(
            "problem.yaml".to_string(),
            format!("{YAML}{extension}").into_bytes(),
        );
    }
    let problem = PetabProblem::read("problem.yaml", &files).unwrap();
    let executor = Executor::new(Plan::new(&problem, &Options::default()).unwrap()).unwrap();
    let evaluation = executor.evaluate(&executor.plan().nominal());
    assert!(evaluation.failures.is_empty(), "{:?}", evaluation.failures);
    evaluation.llh
}

#[test]
fn the_validation_data_of_a_validation_problem_is_its_data() {
    // a fit (and its training problem) leaves its validation data out of the likelihood, the
    // validation problem of the fit has it as its data, as sbmlsim reads the roles
    let all = llh_of_role(None);
    let fit = llh_of_role(Some("fit"));
    assert!((fit - all).abs() > 1e-3, "{fit} == {all}");
    assert_eq!(llh_of_role(Some("training")), fit);
    assert_eq!(llh_of_role(Some("validation")), all);
}
