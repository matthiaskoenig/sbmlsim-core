//! The contract `fit/1`: a fit as a file, which a host writes and the answer it reads back.

use std::sync::Arc;

use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_fit::{
    FAILED_RESIDUAL, FIT_CONTRACT, FitAnswer, FitRequest, JacobianChoice, LIKELIHOOD_OFFSET, LN_2,
    LN_2PI_HALF, NoiseKind, Problem, Stop, fit, fit_reporting, problem_of,
};
use sbmlsim_host::LoadedModel;
use serde_json::{Value, json};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, data, model, next_to_each_other, request, runtime};

/// The request of the fit of `k` of the decay model, as a host writes it.
fn json(start: f64) -> Value {
    let (times, values) = data(K);
    json!({
        "contract": FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": start, "lower": 0.01, "upper": 10.0, "scale": "log10"}
        ],
        "conditions": [{"id": "decay", "request": request()}],
        "curves": [{
            "id": "y of the decay",
            "condition": "decay",
            "column": "y",
            "times": times,
            "values": values,
        }],
        "optimizer": {"algorithm": "trust_region", "max_iterations": 50},
        "threads": 4,
    })
}

/// The request of the fit of `k` and of the initial value of `y`, with the standard
/// deviation of the data, which an identifiability analysis needs two parameters for.
fn pair() -> Value {
    let (times, values) = data(K);
    let noise: Vec<f64> = vec![0.02; values.len()];
    json!({
        "contract": FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": 0.5, "lower": 0.01, "upper": 10.0, "scale": "log10"},
            {"id": "y", "start": 2.0, "lower": 0.1, "upper": 10.0, "scale": "log10"},
        ],
        "conditions": [{"id": "decay", "request": request()}],
        "curves": [{
            "condition": "decay",
            "column": "y",
            "times": times,
            "values": values,
            "noise": noise,
        }],
        "threads": 4,
    })
}

/// The fit of a request, or what it was refused with.
fn run(request: Value) -> Result<FitAnswer, String> {
    let request: FitRequest = serde_json::from_value(request).map_err(|err| err.to_string())?;
    fit(&request, model(), None).map_err(|err| err.to_string())
}

#[test]
fn a_fit_of_a_request_finds_the_parameter_of_the_data() {
    let answer = run(json(3.0)).unwrap();

    assert_eq!(answer.contract, FIT_CONTRACT);
    assert_eq!(answer.parameters.len(), 1);
    assert_eq!(answer.parameters[0].id, "k");
    assert!(
        (answer.parameters[0].value - K).abs() < 1e-6,
        "the fit of the request found k = {} instead of {K}, {}",
        answer.parameters[0].value,
        answer.status
    );
    assert!(
        answer.cost < 1e-12,
        "the cost of the optimum is {}",
        answer.cost
    );
    assert_eq!(answer.residuals.len(), 11);
    assert!(answer.evaluations > 0 && answer.jacobians > 0);
    assert_eq!(answer.runs.len(), 1);
    assert_eq!(answer.runs[0].cost, answer.cost);
}

#[test]
fn the_answer_of_a_fit_is_a_file_of_its_own() {
    let answer = run(json(3.0)).unwrap();

    let written = serde_json::to_value(&answer).unwrap();

    assert_eq!(written["contract"], FIT_CONTRACT);
    assert_eq!(written["parameters"][0]["id"], "k");
    assert!(written["parameters"][0]["value"].is_f64());
    assert!(written["parameters"][0]["x"].is_f64());
    assert!(written["seconds"].as_f64().unwrap() > 0.0);
    let back: FitAnswer = serde_json::from_value(written).unwrap();
    assert_eq!(back.cost, answer.cost);
}

#[test]
fn the_defaults_of_a_request_are_the_ones_of_a_fit() {
    // no optimizer, no scale, no noise and no weight: a trust region on the values themselves
    let (times, values) = data(K);
    let answer = run(json!({
        "contract": FIT_CONTRACT,
        "parameters": [{"id": "k", "start": 1.0, "lower": 0.01, "upper": 10.0}],
        "conditions": [{"id": "decay", "request": request()}],
        "curves": [{"condition": "decay", "column": "y", "times": times, "values": values}],
    }))
    .unwrap();

    assert!(
        (answer.parameters[0].value - K).abs() < 1e-6,
        "{} is not {K}",
        answer.parameters[0].value
    );
    assert_eq!(answer.parameters[0].value, answer.parameters[0].x);
}

#[test]
fn an_evolution_of_a_request_is_its_own_block() {
    let mut request = json(3.0);
    request["optimizer"] = json!({
        "algorithm": {"differential_evolution": {"seed": 7, "population": 12, "polish": false}},
        "max_iterations": 30,
    });

    let answer = run(request).unwrap();

    assert!(
        (answer.parameters[0].value - K).abs() < 1e-3,
        "the evolution of the request found k = {}, {}",
        answer.parameters[0].value,
        answer.status
    );
    assert_eq!(answer.jacobians, 0, "the evolution was polished after all");
}

#[test]
fn a_fit_of_many_starts_keeps_the_best_of_them() {
    let mut request = json(3.0);
    request["starts"] = json!([[0.02], [3.0], [9.0]]);

    let answer = run(request).unwrap();

    assert_eq!(answer.runs.len(), 3);
    assert_eq!(answer.runs[0].start, vec![0.02]);
    assert!(
        answer.runs.iter().all(|run| run.cost >= answer.cost),
        "the fit kept a run which is not the best: {:?}",
        answer.runs.iter().map(|run| run.cost).collect::<Vec<_>>()
    );
    assert!(
        (answer.parameters[0].value - K).abs() < 1e-6,
        "{} is not {K}",
        answer.parameters[0].value
    );
}

#[test]
fn the_starts_of_a_fit_run_on_the_threads_of_its_batch() {
    // four runs of the same start on four threads, which is the same work four times: the
    // seconds of the answer are the ones of the longest run and not the sum of the four
    let mut request = json(3.0);
    request["starts"] = json!([[3.0], [3.0], [3.0], [3.0]]);
    request["threads"] = json!(4);

    let answer = run(request).unwrap();

    assert_eq!(answer.runs.len(), 4);
    let chains: Vec<f64> = answer.runs.iter().map(|run| run.seconds).collect();
    next_to_each_other(answer.seconds, &chains, "the fit of the four starts");
    // and every one of them is the same fit, which no thread of the others disturbed
    let costs: Vec<f64> = answer.runs.iter().map(|run| run.cost).collect();
    assert!(
        costs
            .iter()
            .all(|cost| cost.to_bits() == costs[0].to_bits()),
        "{costs:?}"
    );
}

#[test]
fn a_fit_of_many_starts_answers_the_same_on_one_thread_and_on_eight() {
    // the starts run next to each other on the threads of the batch, and none of them is a
    // step of another: the runs are in the order of the starts and the best of them is the
    // same one, whatever the pool of the fit is
    let starts = json!([[0.02], [0.5], [3.0], [9.0]]);
    let mut one = json(3.0);
    one["starts"] = starts.clone();
    one["threads"] = json!(1);
    let mut eight = json(3.0);
    eight["starts"] = starts;
    eight["threads"] = json!(8);

    let (one, eight) = (run(one).unwrap(), run(eight).unwrap());

    assert_eq!(one.runs.len(), 4);
    for (one, eight) in one.runs.iter().zip(&eight.runs) {
        assert_eq!(one.start, eight.start);
        assert_eq!(one.cost.to_bits(), eight.cost.to_bits(), "{:?}", one.start);
        assert_eq!(one.status, eight.status);
        assert_eq!(one.iterations, eight.iterations);
        let values: Vec<u64> = one.values.iter().map(|v| v.to_bits()).collect();
        let others: Vec<u64> = eight.values.iter().map(|v| v.to_bits()).collect();
        assert_eq!(values, others);
    }
    assert_eq!(one.cost.to_bits(), eight.cost.to_bits());
    assert_eq!(one.simulations, eight.simulations);
}

#[test]
fn a_request_of_another_contract_is_refused() {
    let mut request = json(3.0);
    request["contract"] = json!("fit/2");

    let refusal = run(request).unwrap_err();

    assert!(refusal.contains("fit/2"), "{refusal}");
    assert!(refusal.contains(FIT_CONTRACT), "{refusal}");
}

#[test]
fn a_curve_of_a_condition_the_request_has_not_is_refused() {
    let mut request = json(3.0);
    request["curves"][0]["condition"] = json!("the other dose");

    let refusal = run(request).unwrap_err();

    assert!(refusal.contains("the other dose"), "{refusal}");
}

#[test]
fn a_start_which_is_not_the_parameters_of_the_fit_is_refused() {
    let mut request = json(3.0);
    request["starts"] = json!([[1.0, 2.0]]);

    let refusal = run(request).unwrap_err();

    assert!(refusal.contains("2 values"), "{refusal}");
}

#[test]
fn a_start_outside_the_bounds_of_a_parameter_is_refused() {
    let mut request = json(3.0);
    request["starts"] = json!([[100.0]]);

    let refusal = run(request).unwrap_err();

    assert!(refusal.contains("100"), "{refusal}");
    assert!(refusal.contains("bounds"), "{refusal}");
}

#[test]
fn two_conditions_of_one_id_are_refused() {
    let mut fit_request = json(3.0);
    fit_request["conditions"] = json!([
        {"id": "decay", "request": request()},
        {"id": "decay", "request": request()},
    ]);

    let refusal = run(fit_request).unwrap_err();

    assert!(refusal.contains("decay"), "{refusal}");
}

#[test]
fn a_fit_which_asks_for_the_identifiability_of_its_optimum_answers_with_it() {
    let mut request = pair();
    request["identifiability"] = json!({
        "walk": {"points": 4, "step": 0.075},
        "fisher": true,
    });

    let answer = run(request).unwrap();

    assert_eq!(answer.profiles.len(), 2, "one profile per parameter");
    let (k, y) = (&answer.profiles[0], &answer.profiles[1]);
    assert_eq!(k.id, "k");
    assert_eq!(y.id, "y");
    assert!((k.at - K).abs() < 1e-6, "the profile is at k = {}", k.at);
    assert!(
        !k.points.is_empty() && k.points.len() <= 8,
        "the walk of k took {} points of the four of each side",
        k.points.len()
    );
    assert!(k.identifiable && y.identifiable, "{:?}", (k.lower, k.upper));
    assert!(k.lower.unwrap() < K && K < k.upper.unwrap());
    let fisher = answer.fisher.clone().unwrap();
    assert_eq!(fisher.parameters, vec!["k".to_string(), "y".to_string()]);
    assert_eq!(fisher.information.len(), 4);
    assert!(fisher.errors.unwrap().iter().all(|error| *error > 0.0));
    // and the answer of the analysis is a file of its own as well
    let written = serde_json::to_value(&answer).unwrap();
    assert_eq!(written["profiles"][0]["id"], "k");
    assert!(written["fisher"]["correlation"].is_array());
}

#[test]
fn a_fit_which_asks_for_the_profile_of_one_parameter_walks_that_one() {
    let mut request = pair();
    request["identifiability"] = json!({
        "parameters": ["y"],
        "walk": {"points": 2, "step": 0.1},
        "fisher": false,
    });

    let answer = run(request).unwrap();

    assert_eq!(answer.profiles.len(), 1);
    assert_eq!(answer.profiles[0].id, "y");
    assert!(answer.fisher.is_none());
}

#[test]
fn a_fit_which_asks_for_the_profile_of_a_parameter_it_has_not_is_refused() {
    let mut request = pair();
    request["identifiability"] = json!({"parameters": ["clearance"]});

    let refusal = run(request).unwrap_err();

    assert!(refusal.contains("clearance"), "{refusal}");
}

#[test]
fn a_fit_without_the_block_of_the_identifiability_walks_no_profile() {
    let answer = run(pair()).unwrap();

    assert!(answer.profiles.is_empty());
    assert!(answer.fisher.is_none());
    let written = serde_json::to_value(&answer).unwrap();
    assert!(written.get("profiles").is_none(), "{written}");
    assert!(written.get("fisher").is_none(), "{written}");
}

#[test]
fn a_fit_of_no_iteration_profiles_the_point_it_was_given() {
    // a caller which has the optimum of a fit already walks the profiles around it: the fit
    // of the request takes no step, so its answer is that point, and the points of the
    // profiles fit the other parameters with an optimizer of their own
    let mut request = pair();
    request["starts"] = json!([[K, 1.0]]);
    request["optimizer"] = json!({"max_iterations": 0});
    request["identifiability"] = json!({
        "parameters": ["k"],
        "walk": {"points": 4, "step": 0.1},
        "fisher": false,
        "optimizer": {"max_iterations": 50},
    });

    let answer = run(request).unwrap();

    // the start itself, up to the round trip through the space it is searched in
    for (parameter, start) in answer.parameters.iter().zip([K, 1.0]) {
        assert!(
            (parameter.value - start).abs() < 1e-12,
            "the fit moved {} from {start} to {}",
            parameter.id,
            parameter.value
        );
    }
    assert_eq!(answer.runs[0].iterations, 0);
    let profile = &answer.profiles[0];
    assert!(
        (profile.at - K).abs() < 1e-12,
        "the profile is at {}",
        profile.at
    );
    assert_eq!(profile.cost, answer.cost);
    // the other parameter is fitted again at every point, which an optimizer of no iteration
    // would leave at the value of the point before it
    for point in &profile.points {
        assert!(
            point.status != "MaxIter",
            "the point {} took no iteration: {}",
            point.value,
            point.status
        );
    }
}

#[test]
fn a_fit_reports_every_run_of_it_as_it_finishes() {
    // the runs are reported from the threads of the pool, so a lock keeps them
    let mut request = json(3.0);
    request["starts"] = json!([[0.02], [3.0], [9.0], [5.0]]);
    let request: FitRequest = serde_json::from_value(request).unwrap();
    let reported = std::sync::Mutex::new(Vec::<(usize, f64)>::new());

    let answer = fit_reporting(
        &request,
        model(),
        None,
        &Stop::new(),
        Some(&|index, run| reported.lock().unwrap().push((index, run.cost))),
    )
    .unwrap();

    let mut reported = reported.into_inner().unwrap();
    reported.sort_by_key(|(index, _)| *index);
    assert_eq!(
        reported,
        answer
            .runs
            .iter()
            .enumerate()
            .map(|(index, run)| (index, run.cost))
            .collect::<Vec<_>>(),
        "the runs which were reported are not the runs of the answer"
    );
    // and every run says where its jacobian came from, as the fit of all of them does
    assert!(
        answer
            .runs
            .iter()
            .all(|run| run.jacobian == answer.jacobian),
        "a run of the fit took another jacobian than the fit"
    );
}

#[test]
fn a_fit_which_is_stopped_reports_the_runs_which_finished() {
    // the fit is stopped while it runs, which the report of its first run does: the runs
    // which were reported are the answer, and the ones which never began are none of it
    let mut request = json(3.0);
    request["starts"] = json!([[0.02], [3.0], [9.0], [5.0], [1.0], [0.5], [7.0], [2.0]]);
    request["threads"] = json!(1);
    let request: FitRequest = serde_json::from_value(request).unwrap();
    let stop = Stop::new();
    let reported = std::sync::Mutex::new(Vec::<usize>::new());

    let answer = fit_reporting(
        &request,
        model(),
        Some(1),
        &stop,
        Some(&|index, _| {
            reported.lock().unwrap().push(index);
            stop.stop();
        }),
    )
    .unwrap();

    let reported = reported.into_inner().unwrap();
    assert_eq!(reported.len(), answer.runs.len(), "{reported:?}");
    assert!(
        answer.runs.len() < 8,
        "the fit ran {} of its eight starts after it was stopped",
        answer.runs.len()
    );
}

// The likelihood of a curve: a curve which says which noise distribution its measurements
// carry has two residual rows per measurement, the one of the data and the one of the scale
// of its noise, so that half the sum of their squares is the negative log-likelihood of the
// curve up to half of `LIKELIHOOD_OFFSET` per measurement, which the host subtracts.

/// The decay model whose `sigma` is an assignment rule of `k`, so that a curve which reads
/// that column has a standard deviation which follows the parameters of the fit.
const SIGMA_SBML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="decay">
    <listOfParameters>
      <parameter id="y" value="1" constant="false"/>
      <parameter id="k" value="0.5" constant="true"/>
      <parameter id="sigma" value="0" constant="false"/>
    </listOfParameters>
    <listOfRules>
      <assignmentRule variable="sigma">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><times/><cn>0.1</cn><ci>k</ci></apply>
        </math>
      </assignmentRule>
      <rateRule variable="y">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><times/>
            <apply><minus/><ci>k</ci></apply>
            <ci>y</ci>
          </apply>
        </math>
      </rateRule>
    </listOfRules>
  </model>
</sbml>
"#;

/// That model, compiled and loaded in the runtime of the tests.
fn sigma_model() -> Arc<LoadedModel> {
    let compiled = compile(SIGMA_SBML, &CompileOptions::default()).unwrap();
    Arc::new(
        runtime()
            .load(&compiled.wasm, &compiled.description)
            .unwrap(),
    )
}

/// Asserts that a number is another one bitwise, which is what a host that computes the
/// residual of a point itself reads back.
fn same(value: f64, expected: f64, what: &str) {
    assert_eq!(
        value.to_bits(),
        expected.to_bits(),
        "{what} is {value} and not {expected}"
    );
}

/// The first `n` measurements of the decay at [`K`]: their times and the column of the model
/// there, which the condition of a fit which starts at `K` simulates bitwise.
fn first(n: usize) -> (Vec<f64>, Vec<f64>) {
    let (times, values) = data(K);
    (times[..n].to_vec(), values[..n].to_vec())
}

/// The measurements of the decay at [`K`] after the one at its start, which is the initial
/// value of `y` and does not follow `k`: the rows of a jacobian are measured on these.
fn after_the_start(n: usize) -> (Vec<f64>, Vec<f64>) {
    let (times, values) = data(K);
    (times[1..=n].to_vec(), values[1..=n].to_vec())
}

/// The request of the fit of `k` of one curve on the linear scale, started at [`K`], so that
/// the column of a point of the curve is the measurement of [`first`] at its time.
fn of_a_curve(curve: Value) -> Value {
    json!({
        "contract": FIT_CONTRACT,
        "parameters": [{"id": "k", "start": K, "lower": 0.01, "upper": 10.0}],
        "conditions": [{"id": "decay", "request": request()}],
        "curves": [curve],
        "threads": 1,
    })
}

/// The same request on the model of the `sigma` column, whose condition selects it and whose
/// jacobian is the one of the forward sensitivities.
fn of_the_sigma_model(curve: Value) -> Value {
    let mut condition = request();
    condition.selections = vec!["time".to_string(), "y".to_string(), "sigma".to_string()];
    json!({
        "contract": FIT_CONTRACT,
        "parameters": [{"id": "k", "start": K, "lower": 0.01, "upper": 10.0}],
        "conditions": [{"id": "decay", "request": condition}],
        "curves": [curve],
        "jacobian": "sensitivities",
        "threads": 1,
    })
}

/// The problem of a request on a model, or what it was refused with.
fn problem_on(request: Value, model: Arc<LoadedModel>) -> Result<Problem, String> {
    let request: FitRequest = serde_json::from_value(request).map_err(|err| err.to_string())?;
    problem_of(&request, model, Some(1)).map_err(|err| err.to_string())
}

/// The problem of a request on the decay model of the fixture.
fn problem(request: Value) -> Result<Problem, String> {
    problem_on(request, model())
}

/// The jacobian of the residuals of a problem by a central difference of them, which is
/// accurate to about 1e-9 of itself and is what the block of a likelihood is checked against.
fn central(problem: &Problem, x: &[f64]) -> Vec<f64> {
    let k = problem.parameters().len();
    let n = problem.n_residuals();
    let mut jacobian = vec![0.0; n * k];
    for column in 0..k {
        let step = 1e-6 * x[column].abs().max(1.0);
        let (mut below, mut above) = (x.to_vec(), x.to_vec());
        below[column] -= step;
        above[column] += step;
        let (below, above) = (
            problem.residuals(&below).unwrap(),
            problem.residuals(&above).unwrap(),
        );
        for row in 0..n {
            jacobian[row * k + column] = (above[row] - below[row]) / (2.0 * step);
        }
    }
    jacobian
}

/// The largest relative difference of two jacobians of the same shape.
fn apart(one: &[f64], other: &[f64]) -> f64 {
    assert_eq!(one.len(), other.len());
    one.iter()
        .zip(other)
        .map(|(a, b)| (a - b).abs() / a.abs().max(b.abs()).max(1e-12))
        .fold(0.0, f64::max)
}

#[test]
fn a_normal_likelihood_curve_has_the_rows_of_the_data_and_of_the_scale() {
    let (times, column) = first(2);
    let noise = [0.5, 2.0];
    let values = [column[0] - 0.25, column[1] + 0.75];
    let problem = problem(of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "noise": noise,
        "likelihood": {"distribution": "normal"},
    })))
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(problem.n_residuals(), 4, "two measurements and two scales");
    assert_eq!(residuals.len(), 4);
    for at in 0..2 {
        let data = (column[at] - values[at]) / noise[at];
        same(residuals[at], data, &format!("the data residual {at}"));
        let scale = (2.0 * (noise[at].ln() + LN_2PI_HALF) + LIKELIHOOD_OFFSET).sqrt();
        same(
            residuals[2 + at],
            scale,
            &format!("the scale residual {at}"),
        );
    }
    // and half the sum of their squares is the negative log-likelihood of the curve, up to
    // half the offset per measurement, which the host subtracts
    let negative: f64 = (0..2)
        .map(|at| {
            let r = (column[at] - values[at]) / noise[at];
            0.5 * r * r + noise[at].ln() + LN_2PI_HALF
        })
        .sum();
    let cost = problem.cost(&problem.start()).unwrap() - 0.5 * LIKELIHOOD_OFFSET * 2.0;
    assert!(
        (cost - negative).abs() < 1e-12 * negative.abs().max(1.0),
        "the cost of the curve without its offset is {cost} and its negative \
         log-likelihood {negative}"
    );
}

#[test]
fn a_laplace_likelihood_curve_transforms_the_residual_of_its_data() {
    let (times, column) = first(2);
    let noise = [0.5, 2.0];
    // the first measurement is the column of its row itself, whose residual is zero
    let values = [column[0], column[1] - 1.5];
    let problem = problem(of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "noise": noise,
        "likelihood": {"distribution": "laplace"},
    })))
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(residuals.len(), 4);
    for at in 0..2 {
        let r = (column[at] - values[at]) / noise[at];
        let data = r.signum() * (2.0 * r.abs()).sqrt();
        same(residuals[at], data, &format!("the data residual {at}"));
        let scale = (2.0 * (noise[at].ln() + LN_2) + LIKELIHOOD_OFFSET).sqrt();
        same(
            residuals[2 + at],
            scale,
            &format!("the scale residual {at}"),
        );
    }
    assert_eq!(
        residuals[0], 0.0,
        "the residual of a measurement which is the column of its row is not zero"
    );
    // and half the square of the residual of the data is the absolute deviation the negative
    // log-likelihood of a Laplace noise carries
    let negative: f64 = (0..2)
        .map(|at| ((column[at] - values[at]) / noise[at]).abs() + noise[at].ln() + LN_2)
        .sum();
    let cost = problem.cost(&problem.start()).unwrap() - 0.5 * LIKELIHOOD_OFFSET * 2.0;
    assert!(
        (cost - negative).abs() < 1e-12 * negative.abs().max(1.0),
        "the cost of the curve without its offset is {cost} and its negative \
         log-likelihood {negative}"
    );
}

#[test]
fn a_log_likelihood_curve_adds_the_measurement_to_the_term_and_to_the_offset() {
    let (times, column) = first(2);
    let noise = [0.5, 2.0];
    // the measurements of a log kind are logarithms: the negative one of the two needs the
    // offset of its own row to keep the term of its scale above zero
    let values = [0.5, -1.0];
    for (distribution, constant) in [("log-normal", LN_2PI_HALF), ("log-laplace", LN_2)] {
        let problem = problem(of_a_curve(json!({
            "condition": "decay",
            "column": "y",
            "times": times,
            "values": values,
            "noise": noise,
            "likelihood": {"distribution": distribution},
        })))
        .unwrap();

        let residuals = problem.residuals(&problem.start()).unwrap();

        assert_eq!(residuals.len(), 4);
        let mut negative = 0.0;
        let mut offsets = 0.0;
        for at in 0..2 {
            let r = (column[at] - values[at]) / noise[at];
            let data = if distribution == "log-normal" {
                r
            } else {
                r.signum() * (2.0 * r.abs()).sqrt()
            };
            same(
                residuals[at],
                data,
                &format!("the data residual {at} of a {distribution}"),
            );
            let term = noise[at].ln() + values[at] + constant;
            let offset = LIKELIHOOD_OFFSET + (-2.0 * values[at]).max(0.0);
            let scale = (2.0 * term + offset).sqrt();
            same(
                residuals[2 + at],
                scale,
                &format!("the scale residual {at} of a {distribution}"),
            );
            negative += 0.5 * data * data + term;
            offsets += 0.5 * offset;
        }
        let cost = problem.cost(&problem.start()).unwrap() - offsets;
        assert!(
            (cost - negative).abs() < 1e-12 * negative.abs().max(1.0),
            "the cost of the {distribution} curve without its offsets is {cost} and its \
             negative log-likelihood {negative}"
        );
    }
}

#[test]
fn a_noise_column_is_the_standard_deviation_of_every_point() {
    // `time` is a column of the condition and is zero in its first row: the standard
    // deviation of a point is the value of that column in its own row, and the point whose
    // standard deviation is zero fails
    let (times, column) = first(3);
    let values: Vec<f64> = column.iter().map(|value| value - 0.1).collect();
    let problem = problem(of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "likelihood": {"distribution": "normal", "noise_column": "time"},
    })))
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(residuals.len(), 6);
    same(residuals[0], FAILED_RESIDUAL, "the data residual of t = 0");
    same(residuals[3], FAILED_RESIDUAL, "the scale residual of t = 0");
    for at in 1..3 {
        let sigma = times[at];
        let data = (column[at] - values[at]) / sigma;
        same(residuals[at], data, &format!("the data residual {at}"));
        let scale = (2.0 * (sigma.ln() + LN_2PI_HALF) + LIKELIHOOD_OFFSET).sqrt();
        same(
            residuals[3 + at],
            scale,
            &format!("the scale residual {at}"),
        );
    }
}

#[test]
fn a_noise_column_together_with_a_noise_is_refused() {
    let (times, values) = first(2);

    let refusal = problem(of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "noise": [0.5, 0.5],
        "likelihood": {"distribution": "normal", "noise_column": "time"},
    })))
    .unwrap_err();

    assert!(refusal.contains("the curve 0"), "{refusal}");
    assert!(refusal.contains("noise_column"), "{refusal}");
    assert!(refusal.contains("'noise'"), "{refusal}");
}

#[test]
fn a_likelihood_without_a_noise_of_any_kind_is_refused() {
    let (times, values) = first(2);

    let refusal = problem(of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "likelihood": {"distribution": "normal"},
    })))
    .unwrap_err();

    assert!(refusal.contains("the curve 0"), "{refusal}");
    assert!(refusal.contains("noise_column"), "{refusal}");
}

#[test]
fn a_noise_column_which_the_condition_does_not_select_is_refused() {
    let (times, values) = first(2);

    let refusal = problem(of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "likelihood": {"distribution": "normal", "noise_column": "sigma"},
    })))
    .unwrap_err();

    assert!(refusal.contains("the curve 0"), "{refusal}");
    assert!(refusal.contains("'sigma'"), "{refusal}");
    assert!(
        refusal.contains("does not select"),
        "the refusal of the noise column is '{refusal}'"
    );
}

#[test]
fn the_block_of_a_likelihood_curve_is_the_chain_rule_of_its_rows() {
    // the standard deviation of the curve is the column `sigma` of the model, `0.1 * k`, so
    // both the residual of a measurement and the one of its scale follow the parameter
    for distribution in ["normal", "laplace"] {
        let problem = problem_on(
            of_the_sigma_model(json!({
                "condition": "decay",
                "column": "y",
                "times": [1.0, 2.0, 3.0],
                "values": [0.5, 0.4, 0.3],
                "likelihood": {"distribution": distribution, "noise_column": "sigma"},
            })),
            sigma_model(),
        )
        .unwrap();
        let x = problem.start();

        let block = problem.jacobian(&x).unwrap();

        assert_eq!(problem.jacobian_of(), JacobianChoice::Sensitivities);
        assert_eq!(problem.n_residuals(), 6);
        assert_eq!(block.len(), 6);
        let apart = apart(&block, &central(&problem, &x));
        assert!(
            apart < 1e-6,
            "the block of the {distribution} curve is {apart} off the central difference of \
             its residuals: {block:?} against {:?}",
            central(&problem, &x)
        );
        assert!(
            block[3..].iter().all(|value| *value != 0.0),
            "the rows of the scales of a noise which follows the fit are zero: {block:?}"
        );
    }
}

#[test]
fn a_fixed_noise_has_zero_rows_of_the_jacobian_for_its_scales() {
    let (times, column) = after_the_start(2);
    let values: Vec<f64> = column.iter().map(|value| value - 0.1).collect();
    let mut request = of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "noise": [0.5, 2.0],
        "likelihood": {"distribution": "normal"},
    }));
    request["jacobian"] = json!("sensitivities");
    let problem = problem(request).unwrap();
    let x = problem.start();

    let block = problem.jacobian(&x).unwrap();

    assert_eq!(problem.jacobian_of(), JacobianChoice::Sensitivities);
    assert_eq!(block.len(), 4);
    assert!(
        block[..2].iter().all(|value| *value != 0.0),
        "the rows of the data of the curve are zero: {block:?}"
    );
    assert_eq!(
        &block[2..],
        &[0.0, 0.0],
        "a standard deviation which is a number of the request moves with the fit"
    );
    let apart = apart(&block, &central(&problem, &x));
    assert!(apart < 1e-6, "the block of the curve is {apart} off");
}

#[test]
fn a_fit_of_a_likelihood_curve_finds_the_parameter_of_its_data() {
    // the whole fit and not only its residuals: the optimizer of a curve of a fixed noise
    // minimizes the rows of its data, whose measurements are the model at `K`, and the rows
    // of its scales are the constant cost it ends at
    let (times, values) = data(K);
    let noise: Vec<f64> = vec![0.02; values.len()];
    let mut request = json(3.0);
    request["curves"][0]["noise"] = json!(noise);
    request["curves"][0]["likelihood"] = json!({"distribution": "normal"});

    let answer = run(request).unwrap();

    assert_eq!(answer.residuals.len(), 2 * times.len());
    assert!(
        (answer.parameters[0].value - K).abs() < 1e-6,
        "the fit of the likelihood found k = {} instead of {K}, {}",
        answer.parameters[0].value,
        answer.status
    );
    let scales: f64 = noise
        .iter()
        .map(|sigma| 0.5 * (2.0 * (sigma.ln() + LN_2PI_HALF) + LIKELIHOOD_OFFSET))
        .sum();
    assert!(
        (answer.cost - scales).abs() < 1e-6,
        "the cost of the optimum is {} and the rows of its scales {scales}",
        answer.cost
    );
}

#[test]
fn the_likelihood_of_a_curve_is_written_as_it_was_read() {
    // the shape the host writes, field for field: a round trip of a curve of a likelihood,
    // the kebab-case of every distribution, and a curve without one which carries none, as
    // every request written before the block existed does
    let (times, values) = first(2);
    let given = of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "likelihood": {"distribution": "log-laplace", "noise_column": "time"},
    }));
    let request: FitRequest = serde_json::from_value(given).unwrap();

    let written = serde_json::to_value(&request).unwrap();

    assert_eq!(
        written["curves"][0]["likelihood"],
        json!({"distribution": "log-laplace", "noise_column": "time"})
    );
    for kind in ["normal", "log-normal", "laplace", "log-laplace"] {
        let read: NoiseKind = serde_json::from_value(json!(kind)).unwrap();
        assert_eq!(serde_json::to_value(read).unwrap(), json!(kind));
    }
    let plain: FitRequest = serde_json::from_value(json(3.0)).unwrap();
    let plain = serde_json::to_value(&plain).unwrap();
    assert!(
        plain["curves"][0].get("likelihood").is_none(),
        "a curve without a likelihood carries one: {plain}"
    );
    // and a likelihood without a noise column leaves it out as well
    let column = of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": [0.0],
        "values": [1.0],
        "noise": [0.5],
        "likelihood": {"distribution": "normal"},
    }));
    let column: FitRequest = serde_json::from_value(column).unwrap();
    let column = serde_json::to_value(&column).unwrap();
    assert_eq!(
        column["curves"][0]["likelihood"],
        json!({"distribution": "normal"})
    );
}
