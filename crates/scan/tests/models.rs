//! A dimension of models (sbmlsim's `Dimension(models=...)`): the host gives the description
//! of the model of every key, the planner plans a simulation per model, names the model of
//! every round of simulations, checks the models against the first one as sbmlsim's
//! `_check_model` does and keeps the points in the C order of the scan.

mod common;

use common::{Engine, Model, compile, request, sbml, scan, simulation, values_dimension, variable};
use sbmlsim_scan::{Ask, Scan, ScanErrorKind, ScanRequest};
use serde_json::json;

/// The model of the fold fixtures, `B' = kb B^2`.
fn fold() -> String {
    sbml("scan_fold.xml")
}

/// A request of the options of the fixture `failing_point_raise` (the settings and the
/// selections of the fold model) with a scan of `dimensions`.
fn fold_request(dimensions: &[String], options: &[(&str, serde_json::Value)]) -> ScanRequest {
    request(
        "failing_point_raise",
        &scan(&simulation(1.0, 2), dimensions),
        options,
    )
}

/// The models `fast` (k = 0.2) and `slow` (k = 0.4, B = 3).
fn two_models() -> (Model, Model) {
    let fast = compile(&fold());
    let slow = compile(
        &fold()
            .replace(r#"id="k" value="0.2""#, r#"id="k" value="0.4""#)
            .replace(r#"id="B" value="1""#, r#"id="B" value="3""#),
    );
    (fast, slow)
}

fn open(request: ScanRequest, fast: &Model, slow: &Model) -> Scan {
    Scan::open_with_models(
        request,
        None,
        vec![
            ("fast".to_string(), fast.description.clone()),
            ("slow".to_string(), slow.description.clone()),
        ],
    )
    .unwrap()
}

/// The result of a scan of `dimensions` on one model alone.
fn alone(model: &Model, dimensions: &[String]) -> sbmlsim_scan::ScanResult {
    let scan = Scan::open(fold_request(dimensions, &[]), model.description.clone()).unwrap();
    Engine::of(model).result(scan).unwrap()
}

#[test]
fn a_dimension_of_models() {
    let (fast, slow) = two_models();
    let mut scan = open(
        fold_request(&[common::models_dimension("model", &["fast", "slow"])], &[]),
        &fast,
        &slow,
    );
    let mut engine = Engine::of_models(None, &[("fast", &fast), ("slow", &slow)]);
    let (outcome, models) = engine.run(&mut scan);
    outcome.unwrap();
    assert_eq!(models, [Some("fast".to_string()), Some("slow".to_string())]);
    let result = scan.result().unwrap();
    assert_eq!(result.dims, ["model"]);
    assert_eq!(variable(&result, "k")[0], 0.2);
    let n = result.sizes.iter().find(|s| s.name == "time").unwrap().size;
    assert_eq!(variable(&result, "k")[n], 0.4);
    let model = result.coords.iter().find(|c| c.name == "model").unwrap();
    assert_eq!(model.unit.as_deref(), Some(""));
    // every point is the one of its model alone, bitwise
    for (m, single) in [(0, alone(&fast, &[])), (1, alone(&slow, &[]))] {
        for v in &single.variables {
            let all = variable(&result, &v.name);
            let k = v.values.len();
            let same = all[m * k..(m + 1) * k]
                .iter()
                .zip(&v.values)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            assert!(same, "{} of the model {m}", v.name);
        }
    }
}

#[test]
fn a_dimension_of_models_with_values_is_in_c_order() {
    let (fast, slow) = two_models();
    let rate = values_dimension("rate", r#"{"kb": [0.0, 0.1, 0.2]}"#, "{}", 3);
    let dimensions = [
        rate.clone(),
        common::models_dimension("model", &["fast", "slow"]),
    ];
    let mut engine = Engine::of_models(None, &[("fast", &fast), ("slow", &slow)]);
    let result = engine
        .result(open(fold_request(&dimensions, &[]), &fast, &slow))
        .unwrap();
    assert_eq!(result.dims, ["rate", "model"]);
    let singles = [
        alone(&fast, std::slice::from_ref(&rate)),
        alone(&slow, &[rate]),
    ];
    for v in &singles[0].variables {
        let all = variable(&result, &v.name);
        let k = v.values.len() / 3;
        for r in 0..3 {
            for (m, single) in singles.iter().enumerate() {
                let want = &variable(single, &v.name)[r * k..(r + 1) * k];
                let got = &all[(2 * r + m) * k..(2 * r + m + 1) * k];
                assert!(
                    got.iter()
                        .zip(want)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{} at rate {r}, model {m}",
                    v.name
                );
            }
        }
    }
    // a block across the models is the block of the whole scan
    let block = engine
        .result(open(
            fold_request(&dimensions, &[("points", json!([1, 4]))]),
            &fast,
            &slow,
        ))
        .unwrap();
    let models = block.coords.iter().find(|c| c.name == "model").unwrap();
    assert_eq!(
        models.values,
        sbmlsim_scan::CoordValues::Labels(vec![json!("slow"), json!("fast"), json!("slow")])
    );
    let k = variable(&block, "B").len() / 3;
    assert_eq!(variable(&block, "B"), &variable(&result, "B")[k..4 * k]);
}

#[test]
fn every_model_has_its_settings() {
    let (fast, slow) = two_models();
    let mut settings = common::fixture_request("failing_point_raise")["settings"].clone();
    settings["relative_tolerance"] = json!(1e-7);
    let request = fold_request(
        &[common::models_dimension("model", &["fast", "slow"])],
        &[
            ("model_settings", json!({ "slow": settings })),
            ("on_error", json!("flag")),
        ],
    );
    let mut scan = open(request, &fast, &slow);
    let mut tolerances = Vec::new();
    loop {
        match scan.ask() {
            Ask::Simulate { model, requests } => {
                tolerances.push((model, requests[0].settings.relative_tolerance));
                let answers = requests
                    .iter()
                    .map(|r| {
                        Ok(sbmlsim_abi::SimulationResult {
                            columns: r.selections.clone(),
                            n_rows: 0,
                            values: Vec::new(),
                            sensitivities: None,
                            local_errors: None,
                        })
                    })
                    .collect();
                scan.tell(answers).unwrap();
            }
            Ask::Observe(_) => unreachable!("no observable"),
            Ask::Done => break,
        }
    }
    assert_eq!(
        tolerances,
        [
            (Some("fast".to_string()), 1e-10),
            (Some("slow".to_string()), 1e-7)
        ]
    );
}

#[test]
fn a_model_of_the_run_or_a_dimension_of_models() {
    let (fast, slow) = two_models();
    let dimension = common::models_dimension("model", &["fast", "slow"]);
    let err = Scan::open(
        fold_request(std::slice::from_ref(&dimension), &[]),
        fast.description.clone(),
    )
    .unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Request);
    assert_eq!(
        err.message,
        "The dimension of models 'model' replaces the model of the run: give `None` as the model."
    );
    let err = Scan::open_with_models(fold_request(&[], &[]), None, Vec::new()).unwrap_err();
    assert_eq!(
        err.message,
        "The run needs a model: give the model of the run or a dimension of models."
    );
    let err = Scan::open_with_models(
        fold_request(std::slice::from_ref(&dimension), &[]),
        None,
        vec![("fast".to_string(), fast.description.clone())],
    )
    .unwrap_err();
    assert_eq!(
        err.message,
        "The dimension of models 'model' has the models ['fast', 'slow'], the host gave the \
         models ['fast']."
    );
    let err = Scan::open_with_models(
        fold_request(&[], &[]),
        Some(fast.description.clone()),
        vec![("slow".to_string(), slow.description.clone())],
    )
    .unwrap_err();
    assert_eq!(
        err.message,
        "The scan has no dimension of models, the host gave the models ['slow']."
    );
}

#[test]
fn a_model_without_a_selection_is_an_error() {
    let probe = compile(&fold());
    let other = compile(&fold().replace("kb", "kc"));
    let err = Scan::open_with_models(
        fold_request(
            &[common::models_dimension("model", &["probe", "other"])],
            &[],
        ),
        None,
        vec![
            ("probe".to_string(), probe.description.clone()),
            ("other".to_string(), other.description.clone()),
        ],
    )
    .unwrap_err();
    assert_eq!(
        err.message,
        "The model 'other' has not every selection of the first model of the dimension, it has \
         no selection ['kb']."
    );
}

#[test]
fn models_with_other_units_are_an_error() {
    let seconds = compile(&fold());
    let unitless = compile(&fold().replace(
        r#"id="scan_fold">"#,
        r#"id="scan_fold" timeUnits="dimensionless">"#,
    ));
    let err = Scan::open_with_models(
        fold_request(
            &[common::models_dimension("model", &["seconds", "unitless"])],
            &[],
        ),
        None,
        vec![
            ("seconds".to_string(), seconds.description.clone()),
            ("unitless".to_string(), unitless.description.clone()),
        ],
    )
    .unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Units);
    assert_eq!(
        err.message,
        "The models of the dimension have different units of 'time': 'second' and \
         'dimensionless' of the model 'unitless'."
    );
}

#[test]
fn every_model_of_a_dimension_has_the_targets() {
    let probe = compile(&fold());
    let other = compile(&fold().replace("kb", "kc"));
    let err = Scan::open_with_models(
        fold_request(
            &[
                common::models_dimension("model", &["probe", "other"]),
                values_dimension("d", r#"{"kb": [1.0, 2.0]}"#, "{}", 2),
            ],
            &[],
        ),
        None,
        vec![
            ("probe".to_string(), probe.description.clone()),
            ("other".to_string(), other.description.clone()),
        ],
    )
    .unwrap_err();
    assert!(
        err.message
            .starts_with("The dimension 'd' does not fit the model 'other': "),
        "{}",
        err.message
    );
}

#[test]
fn the_first_failed_point_in_scan_order_raises_across_models() {
    // B' = kb B^2 blows up at 1 / (kb B(0)): kb = 0.5 within the end 1 of `slow` (B(0) = 3)
    // only, kb = 2 within both; the chunk of `fast` (0, 2, 4) fails at 4, the one of `slow`
    // starts before it and fails at 3
    let (fast, slow) = two_models();
    let dimensions = [
        values_dimension("rate", r#"{"kb": [0.1, 0.5, 2.0]}"#, "{}", 3),
        common::models_dimension("model", &["fast", "slow"]),
    ];
    let mut scan = open(fold_request(&dimensions, &[]), &fast, &slow);
    let mut engine = Engine::of_models(None, &[("fast", &fast), ("slow", &slow)]);
    let err = engine.run(&mut scan).0.unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Point);
    assert_eq!(err.point, Some(3));
    assert!(
        err.message
            .starts_with("The point rate=1, kb=0.5, model=slow of the scan failed:"),
        "{}",
        err.message
    );
}

#[test]
fn the_changes_of_a_model_are_defaults_of_its_simulations() {
    // the same SBML twice, `b` with the change k = 0.4; the change of `kb` is overridden by the
    // pre-initialization of the simulation, which sets it itself
    let model = compile(&fold());
    let mut simulation: serde_json::Value = serde_json::from_str(&simulation(1.0, 2)).unwrap();
    simulation["preinit_changes"] = json!({"kb": 0.05});
    let request = request(
        "failing_point_raise",
        &scan(
            &simulation.to_string(),
            &[common::models_dimension("model", &["a", "b"])],
        ),
        &[("model_changes", json!({"b": {"k": 0.4, "kb": 0.5}}))],
    );
    let scan = Scan::open_with_models(
        request,
        None,
        vec![
            ("a".to_string(), model.description.clone()),
            ("b".to_string(), model.description.clone()),
        ],
    )
    .unwrap();
    let mut engine = Engine::of_models(None, &[("a", &model), ("b", &model)]);
    let result = engine.result(scan).unwrap();
    let n = result.sizes.iter().find(|s| s.name == "time").unwrap().size;
    assert_eq!(variable(&result, "k")[0], 0.2);
    assert_eq!(variable(&result, "k")[n], 0.4);
    assert_eq!(variable(&result, "kb")[0], 0.05);
    assert_eq!(variable(&result, "kb")[n], 0.05);
    let err = Scan::open_with_models(
        fold_request(
            &[common::models_dimension("model", &["a"])],
            &[("model_changes", json!({"z": {"k": 0.4}}))],
        ),
        None,
        vec![("a".to_string(), model.description.clone())],
    )
    .unwrap_err();
    assert_eq!(
        err.message,
        "The changes of the models ['z'] are of no model of the dimension of models."
    );
}

#[test]
fn the_changes_of_the_model_of_the_run_are_defaults_of_its_simulations() {
    // `changes` of the model of the run are what `model_changes` are of the model of a
    // dimension of models: the change of `kb` is overridden by the simulation which sets it
    let model = compile(&fold());
    let mut simulation: serde_json::Value = serde_json::from_str(&simulation(1.0, 2)).unwrap();
    simulation["preinit_changes"] = json!({"kb": 0.05});
    let changes = json!({"k": 0.4, "kb": 0.5});
    let run = request(
        "failing_point_raise",
        &scan(&simulation.to_string(), &[]),
        &[("changes", changes.clone())],
    );
    let scan_of_run = Scan::open(run, model.description.clone()).unwrap();
    let of_run = Engine::of(&model).result(scan_of_run).unwrap();
    let dimension = request(
        "failing_point_raise",
        &scan(
            &simulation.to_string(),
            &[common::models_dimension("model", &["a"])],
        ),
        &[("model_changes", json!({ "a": changes }))],
    );
    let scan_of_dimension = Scan::open_with_models(
        dimension,
        None,
        vec![("a".to_string(), model.description.clone())],
    )
    .unwrap();
    let of_dimension = Engine::of_models(None, &[("a", &model)])
        .result(scan_of_dimension)
        .unwrap();
    assert_eq!(variable(&of_run, "k")[0], 0.4);
    assert_eq!(variable(&of_run, "kb")[0], 0.05);
    for v in &of_run.variables {
        let other = variable(&of_dimension, &v.name);
        assert!(
            v.values
                .iter()
                .zip(other)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{}",
            v.name
        );
    }
    // a dimension of models replaces the model of the run and its changes
    let err = Scan::open_with_models(
        fold_request(
            &[common::models_dimension("model", &["a"])],
            &[("changes", json!({"k": 0.4}))],
        ),
        None,
        vec![("a".to_string(), model.description.clone())],
    )
    .unwrap_err();
    assert_eq!(
        err.message,
        "The dimension of models 'model' replaces the model of the run: give the changes of its \
         models as `model_changes`, not `changes`."
    );
}

#[test]
fn a_model_whose_points_ran_is_finished() {
    let (fast, slow) = two_models();
    let dimensions = [
        common::models_dimension("model", &["fast", "slow"]),
        values_dimension("rate", r#"{"kb": [0.0, 0.1]}"#, "{}", 2),
    ];
    let mut scan = open(fold_request(&dimensions, &[]), &fast, &slow);
    let mut engine = Engine::of_models(None, &[("fast", &fast), ("slow", &slow)]);
    assert!(scan.finished_models().is_empty());
    let mut finished = Vec::new();
    loop {
        let ask = scan.ask();
        finished.push(scan.finished_models());
        let Ask::Simulate { model, requests } = ask else {
            break;
        };
        let answers = engine.answers(&model, &requests);
        scan.tell(answers).unwrap();
    }
    assert_eq!(
        finished,
        [
            vec![],
            vec!["fast".to_string()],
            vec!["fast".to_string(), "slow".to_string()]
        ]
    );
}
