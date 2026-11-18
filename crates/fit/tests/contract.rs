//! The contract `fit/1`: a fit as a file, which a host writes and the answer it reads back.

use serde_json::{Value, json};

use sbmlsim_fit::{FIT_CONTRACT, FitAnswer, FitRequest, fit};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, data, model, request};

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
        "walk": {"points": 4, "span": 0.3},
        "fisher": true,
    });

    let answer = run(request).unwrap();

    assert_eq!(answer.profiles.len(), 2, "one profile per parameter");
    let (k, y) = (&answer.profiles[0], &answer.profiles[1]);
    assert_eq!(k.id, "k");
    assert_eq!(y.id, "y");
    assert!((k.at - K).abs() < 1e-6, "the profile is at k = {}", k.at);
    assert_eq!(k.points.len(), 8);
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
        "walk": {"points": 2, "span": 0.2},
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
