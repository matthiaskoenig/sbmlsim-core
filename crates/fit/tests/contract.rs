//! The contract `fit/1`: a fit as a file, which a host writes and the answer it reads back.

use std::sync::Arc;

use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_fit::{
    FAILED_RESIDUAL, FIT_CONTRACT, FitAnswer, FitRequest, JacobianChoice, LIKELIHOOD_OFFSET, LN_2,
    LN_2PI_HALF, NoiseKind, PriorKind, Problem, Stop, fit, fit_reporting, problem_of,
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

/// The decay model whose `sigma` is the assignment rule `factor * k`, so that a curve which
/// reads that column has a noise which follows the parameters of the fit.
fn sigma_sbml(factor: f64) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="decay">
    <listOfParameters>
      <parameter id="y" value="1" constant="false"/>
      <parameter id="k" value="0.5" constant="true"/>
      <parameter id="factor" value="{factor:e}" constant="true"/>
      <parameter id="sigma" value="0" constant="false"/>
    </listOfParameters>
    <listOfRules>
      <assignmentRule variable="sigma">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><times/><ci>factor</ci><ci>k</ci></apply>
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
"#
    )
}

/// That model, compiled and loaded in the runtime of the tests.
fn sigma_model(factor: f64) -> Arc<LoadedModel> {
    let compiled = compile(&sigma_sbml(factor), &CompileOptions::default()).unwrap();
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
    // the noise of the curve is the column `sigma` of the model, `0.1 * k`, so both the
    // residual of a measurement and the one of its scale follow the parameter
    for distribution in ["normal", "log-normal", "laplace", "log-laplace"] {
        let problem = problem_on(
            of_the_sigma_model(json!({
                "condition": "decay",
                "column": "y",
                "times": [1.0, 2.0, 3.0],
                "values": [0.5, 0.4, 0.3],
                "likelihood": {"distribution": distribution, "noise_column": "sigma"},
            })),
            sigma_model(0.1),
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

#[test]
fn a_noise_too_small_for_the_offset_of_a_scale_has_a_row_of_zero() {
    // the term of the scale of such a noise is below `-0.5 * LIKELIHOOD_OFFSET`, which no
    // root reaches: the row is zero and so is its derivative, although the noise follows the
    // fit, and the rows of the data are the ones of the noise
    let problem = problem_on(
        of_the_sigma_model(json!({
            "condition": "decay",
            "column": "y",
            "times": [1.0, 2.0],
            "values": [0.5, 0.4],
            "likelihood": {"distribution": "normal", "noise_column": "sigma"},
        })),
        sigma_model(1e-30),
    )
    .unwrap();
    let x = problem.start();

    let residuals = problem.residuals(&x).unwrap();
    let block = problem.jacobian(&x).unwrap();

    assert_eq!(
        &residuals[2..],
        &[0.0, 0.0],
        "the rows of the scales of a noise of {} are not zero",
        1e-30 * K
    );
    assert_eq!(&block[2..], &[0.0, 0.0], "their derivatives are not zero");
    assert!(
        residuals[..2].iter().all(|value| value.abs() > 1e20),
        "the rows of the data of such a noise are {:?}",
        &residuals[..2]
    );
}

#[test]
fn the_rows_of_a_likelihood_curve_do_not_reach_into_the_curve_beside_it() {
    // two curves of one condition, the first of a likelihood and the second of the weighted
    // least squares: the residual vector is [data, data, scale, scale, plain, plain] and the
    // rows of the jacobian are in the same places
    let (times, column) = after_the_start(2);
    let noise = [0.5, 2.0];
    let values: Vec<f64> = column.iter().map(|value| value - 0.25).collect();
    let plain: Vec<f64> = column.iter().map(|value| value + 0.5).collect();
    let mut request = of_a_curve(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": values,
        "noise": noise,
        "likelihood": {"distribution": "normal"},
    }));
    request["curves"].as_array_mut().unwrap().push(json!({
        "condition": "decay",
        "column": "y",
        "times": times,
        "values": plain,
        "noise": [1.0, 4.0],
    }));
    request["jacobian"] = json!("sensitivities");
    let problem = problem(request).unwrap();
    let x = problem.start();

    let residuals = problem.residuals(&x).unwrap();
    let block = problem.jacobian(&x).unwrap();

    assert_eq!(problem.n_residuals(), 6, "two measurements twice and two");
    assert_eq!(residuals.len(), 6);
    assert_eq!(block.len(), 6);
    for at in 0..2 {
        let data = (column[at] - values[at]) / noise[at];
        same(residuals[at], data, &format!("the data residual {at}"));
        let scale = (2.0 * (noise[at].ln() + LN_2PI_HALF) + LIKELIHOOD_OFFSET).sqrt();
        same(
            residuals[2 + at],
            scale,
            &format!("the scale residual {at}"),
        );
        let other = (column[at] - plain[at]) / [1.0, 4.0][at];
        same(
            residuals[4 + at],
            other,
            &format!("the residual {at} of the curve beside it"),
        );
    }
    // the rows of the scales of a fixed noise are the only zero ones, and the rows of the
    // second curve are its own and not the ones of the first
    assert_eq!(&block[2..4], &[0.0, 0.0], "{block:?}");
    assert!(block[..2].iter().all(|value| *value != 0.0), "{block:?}");
    assert!(block[4..].iter().all(|value| *value != 0.0), "{block:?}");
    let apart = apart(&block, &central(&problem, &x));
    assert!(apart < 1e-6, "the block of the two curves is {apart} off");
}

// The priors of a fit: a parameter may carry a prior and several of them may be correlated,
// and the engine appends their residual rows after every row of the curves, so that half the
// sum of the squares of all of them is the negative log posterior of the fit up to a constant
// of the problem, which the host subtracts.

/// The bounds of the parameter of a prior of these tests, the ones the normalizers of
/// [`CASES`] were computed between.
const PRIOR_LOWER: f64 = 0.1;
const PRIOR_UPPER: f64 = 5.0;

/// The request of the fit of `k` of one curve of two measurements, started at a value between
/// [`PRIOR_LOWER`] and [`PRIOR_UPPER`], whose priors are the given ones: the rows of the
/// priors are the last two of its residuals.
fn of_a_prior(start: f64, priors: Value) -> Value {
    let (times, values) = first(2);
    json!({
        "contract": FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": start, "lower": PRIOR_LOWER, "upper": PRIOR_UPPER}
        ],
        "conditions": [{"id": "decay", "request": request()}],
        "curves": [{
            "condition": "decay",
            "column": "y",
            "times": times,
            "values": values,
        }],
        "priors": priors,
        "threads": 1,
    })
}

/// The request of the fit of `k` and of the initial value of `y` of one curve of two
/// measurements, started at the two values, whose priors are correlated.
fn of_a_correlation(k: f64, y: f64, correlation: Value) -> Value {
    let (times, values) = first(2);
    json!({
        "contract": FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": k, "lower": PRIOR_LOWER, "upper": PRIOR_UPPER},
            {"id": "y", "start": y, "lower": PRIOR_LOWER, "upper": PRIOR_UPPER},
        ],
        "conditions": [{"id": "decay", "request": request()}],
        "curves": [{
            "condition": "decay",
            "column": "y",
            "times": times,
            "values": values,
        }],
        "prior_correlation": correlation,
        "threads": 1,
    })
}

/// The same request with every parameter searched on a scale, which the row of the jacobian
/// of a prior is the chain rule of (`Scale::derivative`): the bounds of `of_a_prior` and of
/// `of_a_correlation` are positive, so every scale takes them.
fn on_the_scale(mut request: Value, scale: &str) -> Value {
    for parameter in request["parameters"].as_array_mut().unwrap() {
        parameter["scale"] = json!(scale);
    }
    request
}

/// The derivative of the value of a parameter in the point of the search space which searches
/// it on a scale, the factor of a row of the jacobian of a prior.
fn of_the_scale(scale: &str, value: f64) -> f64 {
    match scale {
        "lin" => 1.0,
        "log10" => value * std::f64::consts::LN_10,
        other => panic!("the scale {other} is no scale of these tests"),
    }
}

/// One case of a kind of prior: the parameters of the distribution, the value of the
/// parameter of the fit, the normalizer of the truncation at the bounds and the log density
/// `petab` computes there.
struct Case {
    kind: &'static str,
    parameters: &'static [f64],
    value: f64,
    normalizer: f64,
    density: f64,
}

impl Case {
    /// The prior of the case as a host writes it, of the offset zero, so that its row is
    /// `sqrt(-2 log p)` of the log density of the truncated prior.
    fn prior(&self) -> Value {
        json!({
            "parameter": 0,
            "kind": self.kind,
            "parameters": self.parameters,
            "offset": 0.0,
            "normalizer": self.normalizer,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        })
    }
}

/// One case per kind of prior, whose `normalizer` is the logarithm of the mass of the prior
/// between the bounds and whose `density` is the log density of the prior truncated at them,
/// both computed outside the engine:
///
/// ```python
/// from scipy.stats import norm, laplace
/// from sbmlsim.fit.objects import Prior
/// Prior("normal", (1.0, 0.5)).log_density(1.3, 0.1, 5.0)
/// math.log(norm.cdf(5.0, 1.0, 0.5) - norm.cdf(0.1, 1.0, 0.5))
/// ```
///
/// and the kinds of a value which is not negative by `scipy.stats` alone, e.g.
///
/// ```python
/// from scipy.stats import gamma
/// d = gamma(2.0, scale=0.9)
/// normalizer = math.log(d.cdf(5.0) - d.cdf(0.1))
/// density = d.logpdf(1.5) - normalizer
/// ```
///
/// The normalizer is the one a host sends and is no number of the engine, which evaluates the
/// density of the kind itself: the two together are the number `petab` computes, so the
/// closed form of every kind is checked against `scipy` and nothing of it cancels.
const CASES: [Case; 11] = [
    Case {
        kind: "normal",
        parameters: &[1.0, 0.5],
        value: 1.3,
        normalizer: -0.036_591_703_906_010_09,
        density: -0.369_199_648_738_717_13,
    },
    Case {
        kind: "log-normal",
        parameters: &[0.0, 0.7],
        value: 1.4,
        normalizer: -0.011_312_617_346_251_117,
        density: -1.002_947_255_496_903_7,
    },
    Case {
        kind: "laplace",
        parameters: &[1.0, 0.4],
        value: 0.6,
        normalizer: -0.054_162_999_894_149_17,
        density: -0.722_693_448_791_641,
    },
    Case {
        kind: "log-laplace",
        parameters: &[0.2, 0.6],
        value: 1.7,
        normalizer: -0.057_044_403_638_205_81,
        density: -1.206_952_489_321_536_4,
    },
    Case {
        kind: "uniform",
        parameters: &[0.5, 3.0],
        value: 2.1,
        normalizer: 0.0,
        density: -0.916_290_731_874_155,
    },
    Case {
        kind: "log-uniform",
        parameters: &[0.5, 3.0],
        value: 2.1,
        normalizer: 0.0,
        density: -1.325_135_425_512_036_7,
    },
    Case {
        kind: "cauchy",
        parameters: &[1.0, 0.8],
        value: 1.6,
        normalizer: -0.348_324_759_607_335_94,
        density: -1.019_548_677_556_274,
    },
    Case {
        kind: "gamma",
        parameters: &[2.0, 0.9],
        value: 1.5,
        normalizer: -0.031_570_491_408_447_1,
        density: -1.018_910_035_834_402_5,
    },
    Case {
        kind: "chisquare",
        parameters: &[3.0],
        value: 1.2,
        normalizer: -0.198_401_818_667_878_57,
        density: -1.229_375_936_139_816_7,
    },
    Case {
        kind: "exponential",
        parameters: &[1.5],
        value: 0.8,
        normalizer: -0.105_546_097_672_252_72,
        density: -0.833_252_343_769_245,
    },
    Case {
        kind: "rayleigh",
        parameters: &[1.2],
        value: 1.1,
        normalizer: -0.003_642_684_231_843_451_8,
        density: -0.685_829_138_440_629_8,
    },
];

#[test]
fn a_normal_prior_is_a_row_after_the_curves() {
    let (mu, sigma) = (1.0, 0.5);
    let (offset, normalizer, value) = (2.0, -0.25, 1.3);
    let problem = problem(of_a_prior(
        value,
        json!([{
            "parameter": 0,
            "kind": "normal",
            "parameters": [mu, sigma],
            "offset": offset,
            "normalizer": normalizer,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }]),
    ))
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(problem.n_residuals(), 3, "two measurements and one prior");
    assert_eq!(residuals.len(), 3);
    let z: f64 = (value - mu) / sigma;
    let density = -0.5 * z * z - sigma.ln() - LN_2PI_HALF;
    same(
        residuals[2],
        (offset - 2.0 * (density - normalizer)).sqrt(),
        "the row of the prior",
    );
    // and half its square is the negative log density of the prior up to half the offset,
    // which the host subtracts with the rest of its offset
    let cost = problem.cost(&problem.start()).unwrap();
    let curves = 0.5 * residuals[..2].iter().map(|r| r * r).sum::<f64>();
    let negative = -(density - normalizer);
    assert!(
        (cost - curves - 0.5 * offset - negative).abs() < 1e-12,
        "the cost of the prior without its offset is {} and its negative log density \
         {negative}",
        cost - curves - 0.5 * offset
    );
}

#[test]
fn every_prior_kind_is_the_log_density_of_petab() {
    for case in &CASES {
        let problem = problem(of_a_prior(case.value, json!([case.prior()]))).unwrap();

        let residuals = problem.residuals(&problem.start()).unwrap();

        assert_eq!(residuals.len(), 3);
        let density = -0.5 * residuals[2] * residuals[2];
        assert!(
            (density - case.density).abs() <= 1e-12 * case.density.abs(),
            "the log density of the {} prior is {density} and the one of petab {}",
            case.kind,
            case.density
        );
    }
}

#[test]
fn a_prior_outside_its_bounds_is_nan() {
    // the bounds of a prior cover the ones of its parameter, so a value outside them is
    // outside the box an optimizer searches in; `residuals` evaluates such a point anyway,
    // and the row of the prior says that it is not defined there and not that the point
    // costs nothing
    let normal = problem(of_a_prior(
        1.3,
        json!([{
            "parameter": 0,
            "kind": "normal",
            "parameters": [1.0, 0.5],
            "offset": 2.0,
            "normalizer": 0.0,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }]),
    ))
    .unwrap();
    let row = |value: f64| normal.residuals(&[value]).unwrap()[2];

    for value in [PRIOR_LOWER - 0.01, PRIOR_UPPER + 0.01] {
        assert!(
            row(value).is_nan(),
            "the row of the value {value} outside the bounds of its prior is not NaN"
        );
    }
    // a value at a bound of its prior is a value of it, which is where an optimizer sits
    // when the data pushes a parameter against its bound
    for value in [PRIOR_LOWER, PRIOR_UPPER] {
        let at = row(value);
        assert!(
            at.is_finite(),
            "the row of the value {value} at a bound of its prior is {at}"
        );
    }
    // and a value inside the bounds whose density is zero, outside the support of a uniform
    // prior, is `NaN` as well: that point is one of the box
    let uniform = problem(of_a_prior(
        1.3,
        json!([{
            "parameter": 0,
            "kind": "uniform",
            "parameters": [2.0, 3.0],
            "offset": 2.0,
            "normalizer": 0.0,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }]),
    ))
    .unwrap();

    assert!(
        uniform.residuals(&uniform.start()).unwrap()[2].is_nan(),
        "the row of a value outside the support of its uniform prior is not NaN"
    );
}

#[test]
fn a_prior_which_does_not_cover_the_bounds_of_its_parameter_is_refused() {
    // the row of a value a prior is not defined at is `NaN`, so a prior inside the box an
    // optimizer searches in would be a cost of `NaN` at points of the fit and no error
    // anywhere: a prior of narrower bounds is refused, one of wider bounds is not, since its
    // rows are numbers everywhere in the box
    let bounded = |lower: f64, upper: f64| {
        problem(of_a_prior(
            1.3,
            json!([{
                "parameter": 0,
                "kind": "normal",
                "parameters": [1.0, 0.5],
                "offset": 2.0,
                "normalizer": 0.0,
                "lower": lower,
                "upper": upper,
            }]),
        ))
        .err()
        .unwrap_or_default()
    };

    for (lower, upper) in [(0.5, PRIOR_UPPER), (PRIOR_LOWER, 2.0), (0.5, 2.0)] {
        let refusal = bounded(lower, upper);
        assert!(
            refusal.contains("does not cover the bounds") && refusal.contains("the prior 0 of 'k'"),
            "the prior between {lower} and {upper} of a parameter between {PRIOR_LOWER} and \
             {PRIOR_UPPER} is refused with {refusal:?}"
        );
    }
    assert_eq!(
        bounded(PRIOR_LOWER - 0.05, PRIOR_UPPER + 0.5),
        String::new(),
        "a prior of wider bounds than its parameter is refused"
    );
}

#[test]
fn correlated_priors_are_whitened_by_the_inverse_in_order() {
    // the priors of `k` and of `y` are one multivariate normal of their scores, the second
    // of them a log-normal one: `inverse` is the inverse of the lower Cholesky factor
    // `[[1, 0], [0.5, 2]]` of their correlation, and the rows are the whitened scores and
    // then the one of the density of the logarithm in the value
    let (k, y) = (1.3, 1.4);
    let (location, scale) = ([1.0, 0.0], [0.5, 0.7]);
    let inverse = [[1.0, 0.0], [-0.25, 0.5]];
    let offset = 3.0;
    let problem = problem(of_a_correlation(
        k,
        y,
        json!({
            "parameters": [0, 1],
            "log": [false, true],
            "location": location,
            "scale": scale,
            "inverse": inverse,
            "offsets": [offset],
            "lower": [PRIOR_LOWER, PRIOR_LOWER],
            "upper": [PRIOR_UPPER, PRIOR_UPPER],
        }),
    ))
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(
        problem.n_residuals(),
        5,
        "two measurements, two whitened scores and one logarithm"
    );
    let scores = [
        (k - location[0]) / scale[0],
        (y.ln() - location[1]) / scale[1],
    ];
    for row in 0..2 {
        // the product is accumulated from zero with one addition per column in ascending
        // order, which is the loop the host evaluates as well, so the two agree bitwise
        let mut total = 0.0;
        for column in 0..2 {
            total += inverse[row][column] * scores[column];
        }
        same(
            residuals[2 + row],
            total,
            &format!("the whitened row {row}"),
        );
    }
    same(
        residuals[4],
        (offset + 2.0 * y.ln()).sqrt(),
        "the row of the logarithm of the log-normal prior",
    );
    // and the whitened scores are the solution of `L w = z`, which is what the inverse of
    // the factor says and which no number of the request is
    let solved = [scores[0] / 1.0, (scores[1] - 0.5 * scores[0]) / 2.0];
    for (row, expected) in solved.iter().enumerate() {
        assert!(
            (residuals[2 + row] - expected).abs() <= 1e-12 * expected.abs(),
            "the whitened row {row} is {} and the forward substitution of the factor \
             {expected}",
            residuals[2 + row]
        );
    }
}

#[test]
fn a_profile_which_holds_a_correlated_parameter_still_reads_its_value() {
    let (k, y) = (1.3, 1.4);
    let mut request = of_a_correlation(
        k,
        y,
        json!({
            "parameters": [0, 1],
            "log": [false, true],
            "location": [1.0, 0.0],
            "scale": [0.5, 0.7],
            "inverse": [[1.0, 0.0], [-0.25, 0.5]],
            "offsets": [3.0],
            "lower": [PRIOR_LOWER, PRIOR_LOWER],
            "upper": [PRIOR_UPPER, PRIOR_UPPER],
        }),
    );
    request["jacobian"] = json!("sensitivities");
    let problem = problem(request).unwrap();
    let held = problem.fixing(&[(0, k)]).unwrap();

    let residuals = problem.residuals(&[k, y]).unwrap();
    let profile = held.residuals(&[y]).unwrap();

    assert_eq!(held.parameters().len(), 1, "the profile searches `y` alone");
    assert_eq!(held.n_residuals(), problem.n_residuals());
    for row in 2..5 {
        same(
            profile[row],
            residuals[row],
            &format!("the prior row {row} of the profile"),
        );
    }
    // and the prior rows of its jacobian are the column of `y` of the whole one: the rows of
    // the correlated priors are as many as the correlated parameters and the columns of
    // their derivatives as well, which is not the `k` of the free parameters of a profile,
    // so the two counts cannot be confused for one another
    let block = held.jacobian(&[y]).unwrap();
    let whole = problem.jacobian(&[k, y]).unwrap();
    assert_eq!(block.len(), held.n_residuals(), "one column per row");
    for row in 2..5 {
        same(
            block[row],
            whole[row * 2 + 1],
            &format!("the prior row {row} of the block of the profile"),
        );
    }
    assert_eq!(
        block[2], 0.0,
        "the first whitened row does not read `y`: {block:?}"
    );
    assert!(
        block[3] != 0.0 && block[4] != 0.0,
        "the second whitened row and the one of the logarithm read `y`: {block:?}"
    );
}

#[test]
fn the_prior_rows_of_the_jacobian_are_analytic() {
    // the row of a prior follows the value of its parameter alone, and the engine writes its
    // derivative instead of a difference of two of them: every kind against the central
    // difference of the residuals themselves, on both scales a parameter of positive bounds
    // is searched on, so that the chain rule of the scale is part of the comparison
    for scale in ["lin", "log10"] {
        for case in &CASES {
            let mut request = on_the_scale(of_a_prior(case.value, json!([case.prior()])), scale);
            request["jacobian"] = json!("sensitivities");
            let problem = problem(request).unwrap();
            let x = problem.start();

            let jacobian = problem.jacobian(&x).unwrap();

            assert_eq!(problem.jacobian_of(), JacobianChoice::Sensitivities);
            assert_eq!(jacobian.len(), 3);
            let apart = apart(&jacobian, &central(&problem, &x));
            assert!(
                apart < 1e-6,
                "the row of the {} prior on the {scale} scale is {apart} off the central \
                 difference of the residuals: {jacobian:?} against {:?}",
                case.kind,
                central(&problem, &x)
            );
            // the density of a uniform prior does not move with the value, so its row is
            // zero and a difference of it says nothing; the row of every other kind moves,
            // and a row which was never written would be zero as well
            let moves = case.kind != "uniform";
            assert_eq!(
                jacobian[2] != 0.0,
                moves,
                "the row of the {} prior on the {scale} scale is {}",
                case.kind,
                jacobian[2]
            );
        }
    }
}

#[test]
fn the_row_of_a_prior_the_offset_does_not_reach_is_zero() {
    // the row is `sqrt(c - 2 log p)`, and the density of a narrow prior is larger than one,
    // whose negative logarithm has no root: an offset the density does not reach is the row
    // zero, the smallest one, which has no derivative either, as the row of the scale of a
    // tiny noise of a likelihood has none
    let (location, scale, value): (f64, f64, f64) = (1.3, 0.01, 1.31);
    let mut request = of_a_prior(
        value,
        json!([{
            "parameter": 0,
            "kind": "normal",
            "parameters": [location, scale],
            "offset": 0.0,
            "normalizer": 0.0,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }]),
    );
    request["jacobian"] = json!("sensitivities");
    let problem = problem(request).unwrap();
    let x = problem.start();

    let residuals = problem.residuals(&x).unwrap();
    let jacobian = problem.jacobian(&x).unwrap();

    // the density at one standard deviation of a scale of 0.01 is about 3.2, so the term of
    // the row, `0 - 2 * 3.2`, is negative and the row is the clamped zero, while the density
    // moves with the value there, `-z/s` of about -100
    let z: f64 = (value - location) / scale;
    let density = -0.5 * z * z - scale.ln() - LN_2PI_HALF;
    assert!(density > 0.0, "the density of the prior is {density}");
    same(residuals[2], 0.0, "the row of the prior");
    same(jacobian[2], 0.0, "the derivative of that row");
}

#[test]
fn the_row_of_a_laplace_prior_at_its_location_has_no_slope() {
    // the density of a Laplace prior has no derivative at its location, where it is largest
    // and its row smallest: the engine takes the derivative as zero there, as it takes the
    // one of the residual of the data of a Laplace likelihood at its own minimum
    let (location, scale, offset): (f64, f64, f64) = (1.3, 0.4, 2.0);
    let mut request = of_a_prior(
        location,
        json!([{
            "parameter": 0,
            "kind": "laplace",
            "parameters": [location, scale],
            "offset": offset,
            "normalizer": 0.0,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }]),
    );
    request["jacobian"] = json!("sensitivities");
    let problem = problem(request).unwrap();
    let x = problem.start();

    let residuals = problem.residuals(&x).unwrap();
    let jacobian = problem.jacobian(&x).unwrap();

    // the row itself is a number of the density at the kink, `-ln(2 b)`, and is positive, so
    // the zero of its derivative is the one of the density and no clamped row
    let density = -(2.0 * scale).ln();
    same(
        residuals[2],
        (offset - 2.0 * density).sqrt(),
        "the row of the prior",
    );
    assert!(residuals[2] > 0.0, "the row is {}", residuals[2]);
    assert_eq!(
        jacobian[2], 0.0,
        "the derivative of the row at the kink is {}",
        jacobian[2]
    );
}

#[test]
fn the_rows_of_the_correlated_priors_of_the_jacobian_are_analytic() {
    // a whitened row follows every correlated parameter and the row of a logarithm its own,
    // on both scales a parameter of positive bounds is searched on, so that the chain rule
    // of the scale is part of the comparison
    let (k, y) = (1.3, 1.4);
    for scale in ["lin", "log10"] {
        let mut request = on_the_scale(
            of_a_correlation(
                k,
                y,
                json!({
                    "parameters": [0, 1],
                    "log": [false, true],
                    "location": [1.0, 0.0],
                    "scale": [0.5, 0.7],
                    "inverse": [[1.0, 0.0], [-0.25, 0.5]],
                    "offsets": [3.0],
                    "lower": [PRIOR_LOWER, PRIOR_LOWER],
                    "upper": [PRIOR_UPPER, PRIOR_UPPER],
                }),
            ),
            scale,
        );
        request["jacobian"] = json!("sensitivities");
        let problem = problem(request).unwrap();
        let x = problem.start();

        let jacobian = problem.jacobian(&x).unwrap();

        assert_eq!(jacobian.len(), 5 * 2);
        let apart = apart(&jacobian, &central(&problem, &x));
        assert!(
            apart < 1e-6,
            "the rows of the correlated priors on the {scale} scale are {apart} off the \
             central difference of the residuals: {jacobian:?} against {:?}",
            central(&problem, &x)
        );
        // the first whitened row reads `k` alone, since the inverse of the factor is lower
        // triangular, the second both, and the logarithm reads `y` alone; its derivative is
        // the one in the value, `inverse[0][0] / scale[0]`, times the one of the scale the
        // parameter is searched on
        let expected = of_the_scale(scale, k) / 0.5;
        assert!(
            (jacobian[4] - expected).abs() <= 1e-9 * expected,
            "the first whitened row on the {scale} scale is {} and not {expected}: \
             {jacobian:?}",
            jacobian[4]
        );
        assert_eq!(jacobian[5], 0.0, "{jacobian:?}");
        assert!(
            jacobian[6] != 0.0 && jacobian[7] != 0.0,
            "the second whitened row reads one parameter: {jacobian:?}"
        );
        assert_eq!(jacobian[8], 0.0, "{jacobian:?}");
        assert!(jacobian[9] != 0.0, "{jacobian:?}");
    }
}

#[test]
fn the_prior_rows_are_the_same_for_both_jacobians() {
    // the rows of the priors are rows of the residuals, so the difference quotients of a fit
    // difference them as well: a row which is linear in the value of its parameter, a
    // whitened score of a prior which is no logarithm, is the analytic row to the rounding of
    // the quotient, and a curved one, the root of an independent prior, to the step of the
    // forward difference of `DIFF_STEP`, which is the accuracy of every other row of that
    // jacobian too (`sensitivity.rs` measures 1e-2 of the rows of a curve)
    let both = |mut request: Value, rows: std::ops::Range<usize>| {
        request["jacobian"] = json!("sensitivities");
        let sensitivities = problem(request.clone()).unwrap();
        request["jacobian"] = json!("difference_quotient");
        let differences = problem(request).unwrap();
        let x = sensitivities.start();
        let k = sensitivities.parameters().len();
        let (one, other) = (
            sensitivities.jacobian(&x).unwrap(),
            differences.jacobian(&x).unwrap(),
        );
        assert_eq!(
            differences.jacobian_of(),
            JacobianChoice::DifferenceQuotient
        );
        apart(
            &one[rows.start * k..rows.end * k],
            &other[rows.start * k..rows.end * k],
        )
    };

    let linear = both(
        of_a_correlation(
            1.3,
            1.4,
            json!({
                "parameters": [0, 1],
                "log": [false, false],
                "location": [1.0, 1.0],
                "scale": [0.5, 0.7],
                "inverse": [[1.0, 0.0], [-0.25, 0.5]],
                "offsets": [],
                "lower": [PRIOR_LOWER, PRIOR_LOWER],
                "upper": [PRIOR_UPPER, PRIOR_UPPER],
            }),
        ),
        2..4,
    );
    let curved = both(of_a_prior(1.3, json!([CASES[0].prior()])), 2..3);

    assert!(
        linear < 1e-5,
        "the whitened rows of the two jacobians are {linear} apart"
    );
    assert!(
        curved < 1e-2,
        "the row of the normal prior of the two jacobians is {curved} apart"
    );
}

#[test]
fn the_rows_of_a_posterior_are_its_curves_and_then_its_priors() {
    // the whole of what a fit of a posterior writes: a curve of a likelihood, whose rows are
    // its data and then the scales of its noise, the prior of a parameter of its own and the
    // correlated priors of two others, in that order after every row of the curve
    let (times, values) = first(2);
    let noise = [0.5f64, 2.0];
    let request: Value = json!({
        "contract": FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": 1.3, "lower": PRIOR_LOWER, "upper": PRIOR_UPPER},
            {"id": "y", "start": 1.4, "lower": PRIOR_LOWER, "upper": PRIOR_UPPER},
            {"id": "factor", "start": 2.1, "lower": PRIOR_LOWER, "upper": PRIOR_UPPER},
        ],
        "conditions": [{"id": "decay", "request": request()}],
        "curves": [{
            "condition": "decay",
            "column": "y",
            "times": times,
            "values": values,
            "noise": noise,
            "likelihood": {"distribution": "normal"},
        }],
        "priors": [{
            "parameter": 2,
            "kind": "uniform",
            "parameters": [0.5, 3.0],
            "offset": 2.0,
            "normalizer": 0.0,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }],
        "prior_correlation": {
            "parameters": [0, 1],
            "log": [false, true],
            "location": [1.0, 0.0],
            "scale": [0.5, 0.7],
            "inverse": [[1.0, 0.0], [-0.25, 0.5]],
            "offsets": [3.0],
            "lower": [PRIOR_LOWER, PRIOR_LOWER],
            "upper": [PRIOR_UPPER, PRIOR_UPPER],
        },
        "threads": 1,
    });
    let problem = problem_on(request, sigma_model(0.1)).unwrap();
    let x = problem.start();

    let residuals = problem.residuals(&x).unwrap();

    assert_eq!(
        problem.n_residuals(),
        8,
        "two measurements, two scales, one prior, two whitened scores and one logarithm"
    );
    assert_eq!(residuals.len(), 8);
    let (k, y) = (1.3f64, 1.4f64);
    for at in 0..2 {
        let scale = (2.0 * (noise[at].ln() + LN_2PI_HALF) + LIKELIHOOD_OFFSET).sqrt();
        same(
            residuals[2 + at],
            scale,
            &format!("the scale residual {at}"),
        );
    }
    same(
        residuals[4],
        (2.0 - 2.0 * -(2.5f64.ln())).sqrt(),
        "the row of the uniform prior",
    );
    let scores = [(k - 1.0) / 0.5, y.ln() / 0.7];
    same(residuals[5], scores[0], "the first whitened row");
    same(
        residuals[6],
        -0.25 * scores[0] + 0.5 * scores[1],
        "the second whitened row",
    );
    same(
        residuals[7],
        (3.0 + 2.0 * y.ln()).sqrt(),
        "the row of the logarithm",
    );
}

#[test]
fn the_priors_of_a_fit_are_written_as_they_were_read() {
    // the shape the host writes, field for field: a round trip of the priors and of their
    // correlation, the kebab-case of every kind, and a fit without them, as every request
    // written before the block existed is
    let given = of_a_prior(1.3, json!([CASES[1].prior()]));
    let request: FitRequest = serde_json::from_value(given).unwrap();

    let written = serde_json::to_value(&request).unwrap();

    assert_eq!(
        written["priors"],
        json!([{
            "parameter": 0,
            "kind": "log-normal",
            "parameters": [0.0, 0.7],
            "offset": 0.0,
            "normalizer": CASES[1].normalizer,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }])
    );
    assert!(
        written.get("prior_correlation").is_none(),
        "a fit without correlated priors carries the block: {written}"
    );
    for kind in [
        "normal",
        "log-normal",
        "uniform",
        "log-uniform",
        "laplace",
        "log-laplace",
    ] {
        let read: PriorKind = serde_json::from_value(json!(kind)).unwrap();
        assert_eq!(serde_json::to_value(read).unwrap(), json!(kind));
    }
    let correlation = json!({
        "parameters": [0, 1],
        "log": [false, true],
        "location": [1.0, 0.0],
        "scale": [0.5, 0.7],
        "inverse": [[1.0, 0.0], [-0.25, 0.5]],
        "offsets": [3.0],
        "lower": [PRIOR_LOWER, PRIOR_LOWER],
        "upper": [PRIOR_UPPER, PRIOR_UPPER],
    });
    let pair: FitRequest =
        serde_json::from_value(of_a_correlation(1.3, 1.4, correlation.clone())).unwrap();
    let pair = serde_json::to_value(&pair).unwrap();
    assert_eq!(pair["prior_correlation"], correlation);
    // and a request of no prior reads as a fit of none, which is every request of a fit of
    // the least squares of its curves
    let plain: FitRequest = serde_json::from_value(json(3.0)).unwrap();
    assert_eq!(serde_json::to_value(&plain).unwrap()["priors"], json!([]));
    let plain = problem(json(3.0)).unwrap();
    assert_eq!(plain.n_residuals(), 11, "the measurements of the curve");
}

#[test]
fn a_prior_of_a_parameter_the_fit_has_not_is_refused() {
    let refusal = problem(of_a_prior(
        1.3,
        json!([{
            "parameter": 3,
            "kind": "normal",
            "parameters": [1.0, 0.5],
            "offset": 2.0,
            "normalizer": 0.0,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        }]),
    ))
    .unwrap_err();

    assert!(
        refusal.contains("the prior 0") && refusal.contains("the parameter 3"),
        "the refusal of the prior is '{refusal}'"
    );
}

#[test]
fn a_prior_which_cannot_be_evaluated_is_refused() {
    // every refusal of a prior and of the correlation of the priors, which are the ones a
    // host hits when it writes a block of its own
    let prior = |changes: Value| {
        let mut prior = json!({
            "parameter": 0,
            "kind": "normal",
            "parameters": [1.0, 0.5],
            "offset": 2.0,
            "normalizer": 0.0,
            "lower": PRIOR_LOWER,
            "upper": PRIOR_UPPER,
        });
        for (key, value) in changes.as_object().unwrap() {
            prior[key] = value.clone();
        }
        json!([prior])
    };
    let correlation = |changes: Value| {
        let mut correlation = json!({
            "parameters": [0, 1],
            "log": [false, true],
            "location": [1.0, 0.0],
            "scale": [0.5, 0.7],
            "inverse": [[1.0, 0.0], [-0.25, 0.5]],
            "offsets": [3.0],
            "lower": [PRIOR_LOWER, PRIOR_LOWER],
            "upper": [PRIOR_UPPER, PRIOR_UPPER],
        });
        for (key, value) in changes.as_object().unwrap() {
            correlation[key] = value.clone();
        }
        correlation
    };
    let cases: Vec<(Value, &str)> = vec![
        (
            of_a_prior(1.3, json!([prior(json!({}))[0], prior(json!({}))[0]])),
            "a second prior",
        ),
        (
            of_a_prior(1.3, prior(json!({"parameters": [1.0]}))),
            "takes two of them",
        ),
        (
            of_a_prior(
                1.3,
                prior(json!({"kind": "exponential", "parameters": [1.0, 2.0]})),
            ),
            "and the kind exponential takes one",
        ),
        (
            of_a_prior(
                1.3,
                prior(json!({"kind": "gamma", "parameters": [0.0, 2.0]})),
            ),
            "which are not both positive",
        ),
        (
            of_a_prior(
                1.3,
                prior(json!({"kind": "chisquare", "parameters": [-1.0]})),
            ),
            "of the parameter -1, which is not positive",
        ),
        (
            of_a_prior(1.3, prior(json!({"parameters": [1.0, 0.0]}))),
            "which is not positive",
        ),
        (
            of_a_prior(
                1.3,
                prior(json!({"kind": "uniform", "parameters": [3.0, 1.0]})),
            ),
            "which are not increasing",
        ),
        (
            of_a_prior(
                1.3,
                prior(json!({"kind": "log-uniform", "parameters": [0.0, 3.0]})),
            ),
            "which is not positive",
        ),
        (
            of_a_prior(1.3, prior(json!({"lower": 2.0, "upper": 1.0}))),
            "which are not increasing",
        ),
        (
            of_a_correlation(
                1.3,
                1.4,
                json!({
                    "parameters": [],
                    "log": [],
                    "location": [],
                    "scale": [],
                    "inverse": [],
                    "offsets": [],
                    "lower": [],
                    "upper": [],
                }),
            ),
            "has no parameter",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"location": [1.0]}))),
            "and 1 of 'location'",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"inverse": [[1.0, 0.0]]}))),
            "the 1 rows of its inverse",
        ),
        (
            of_a_correlation(
                1.3,
                1.4,
                correlation(json!({"inverse": [[1.0], [0.0, 1.0]]})),
            ),
            "the row 0",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"offsets": []}))),
            "logarithmic prior",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"parameters": [0, 7]}))),
            "the parameter 7",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"parameters": [1, 1]}))),
            "twice",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"scale": [0.5, 0.0]}))),
            "which is not positive",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"lower": [0.1, 9.0]}))),
            "between 9 and 5, which are not increasing",
        ),
        (
            of_a_correlation(1.3, 1.4, correlation(json!({"lower": [0.1, -1.0]}))),
            "is not positive",
        ),
    ];

    for (request, expected) in cases {
        let refusal = problem(request).unwrap_err();
        assert!(
            refusal.contains(expected),
            "the refusal '{refusal}' does not say '{expected}'"
        );
    }
}

#[test]
fn a_parameter_which_is_correlated_and_carries_a_prior_of_its_own_is_refused() {
    let request = of_a_correlation(
        1.3,
        1.4,
        json!({
            "parameters": [0, 1],
            "log": [false, false],
            "location": [1.0, 1.0],
            "scale": [0.5, 0.7],
            "inverse": [[1.0, 0.0], [-0.25, 0.5]],
            "offsets": [],
            "lower": [PRIOR_LOWER, PRIOR_LOWER],
            "upper": [PRIOR_UPPER, PRIOR_UPPER],
        }),
    );
    let mut request = request;
    request["priors"] = json!([{
        "parameter": 1,
        "kind": "normal",
        "parameters": [1.0, 0.5],
        "offset": 2.0,
        "normalizer": 0.0,
        "lower": PRIOR_LOWER,
        "upper": PRIOR_UPPER,
    }]);

    let refusal = problem(request).unwrap_err();

    assert!(
        refusal.contains("correlated already"),
        "the refusal of the prior is '{refusal}'"
    );
}
