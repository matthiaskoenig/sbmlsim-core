//! The integration of a model without events: the outputs, a negative start, the linear solvers,
//! the pre-initialization, the reuse of an engine and the errors.

use sbmlsim_abi::{
    IntegratorSettings, JacobianChoice, JacobianKind, LinearSolver, MODEL_CONTRACT, Output,
    SIMULATION_CONTRACT, SimulationRequest, SimulationResult, Tolerance,
};
use sbmlsim_engine::model::{Decay, Robertson};
use sbmlsim_engine::{Engine, EngineError, simulate};

/// A request from `start` to `end` with the given output and selections, `rtol 1e-10` and the
/// non-stiff method.
///
/// The global error of BDF on the decay at `rtol 1e-10` is `2.8e-8` relative at `t = 10` (as in
/// libroadrunner, whose values are bitwise the ones of the engine with the difference quotient
/// up to `t = 9`), the one of Adams-Moulton `1.2e-10` (bitwise the value of libroadrunner).
fn request(start: f64, end: f64, output: Output, selections: &[&str]) -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        contract: SIMULATION_CONTRACT.to_string(),
        start,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: selections.iter().map(|s| s.to_string()).collect(),
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            stiff: false,
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

/// The relative difference of `a` from `b`.
fn relative(a: f64, b: f64) -> f64 {
    ((a - b) / b).abs()
}

/// The values of column `name` of a result.
fn column(result: &SimulationResult, name: &str) -> Vec<f64> {
    let k = result.columns.len();
    let j = result
        .columns
        .iter()
        .position(|c| c == name)
        .unwrap_or_else(|| panic!("no column {name}"));
    (0..result.n_rows)
        .map(|i| result.values[i * k + j])
        .collect()
}

/// Simulates the decay model with the rate `k`.
fn decay(k: f64, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
    let model = Decay { k };
    simulate(&model, &model.description(), req)
}

/// Asserts `y(t) = exp(-k (t - start))` within `tolerance` relative at every row.
fn assert_decay(result: &SimulationResult, k: f64, start: f64, tolerance: f64) {
    for (t, y) in column(result, "time").iter().zip(column(result, "y")) {
        let expected = (-k * (t - start)).exp();
        assert!(
            relative(y, expected) < tolerance,
            "t = {t}: y = {y:e}, expected {expected:e}"
        );
    }
}

#[test]
fn decay_uniform_output() {
    let req = request(0.0, 10.0, Output::Uniform { steps: 10 }, &["time", "y"]);
    let result = decay(0.5, &req).unwrap();
    assert_eq!(result.columns, ["time", "y"]);
    assert_eq!(result.n_rows, 11);
    assert_eq!(result.values.len(), 22);
    let times = column(&result, "time");
    for (i, t) in times.iter().enumerate() {
        assert_eq!(*t, i as f64);
    }
    assert_eq!(column(&result, "y")[0], 1.0);
    assert_decay(&result, 0.5, 0.0, 1e-9);
    // BDF with the analytic Jacobian, within its global error
    let mut stiff = req.clone();
    stiff.settings.stiff = true;
    let result = decay(0.5, &stiff).unwrap();
    assert_eq!(result.n_rows, 11);
    assert_decay(&result, 0.5, 0.0, 1e-7);
}

#[test]
fn negative_start() {
    let req = request(-5.0, 5.0, Output::Uniform { steps: 10 }, &["time", "y"]);
    let result = decay(0.5, &req).unwrap();
    assert_eq!(result.n_rows, 11);
    let times = column(&result, "time");
    assert_eq!(times[0], -5.0);
    assert_eq!(times[5], 0.0);
    assert_eq!(times[10], 5.0);
    assert_decay(&result, 0.5, -5.0, 1e-9);
}

#[test]
fn output_times() {
    let req = request(
        0.0,
        10.0,
        Output::Times(vec![0.0, 0.5, 7.25]),
        &["time", "y"],
    );
    let result = decay(0.5, &req).unwrap();
    assert_eq!(column(&result, "time"), [0.0, 0.5, 7.25]);
    assert_decay(&result, 0.5, 0.0, 1e-9);
    // a time outside of the simulation and times out of order are refused
    for times in [vec![0.0, 11.0], vec![-1.0], vec![2.0, 1.0], vec![f64::NAN]] {
        let req = request(0.0, 10.0, Output::Times(times.clone()), &["time", "y"]);
        assert!(
            matches!(decay(0.5, &req), Err(EngineError::Request(_))),
            "{times:?}"
        );
    }
}

#[test]
fn integrator_steps_end_at_end() {
    let req = request(0.0, 10.0, Output::IntegratorSteps, &["time", "y"]);
    let result = decay(0.5, &req).unwrap();
    let times = column(&result, "time");
    assert!(result.n_rows > 10, "{} rows", result.n_rows);
    assert_eq!(times[0], 0.0);
    assert_eq!(*times.last().unwrap(), 10.0);
    assert!(times.windows(2).all(|w| w[0] < w[1]), "{times:?}");
    assert_decay(&result, 0.5, 0.0, 1e-8);
}

/// The Robertson problem with the given linear solver at `rtol 1e-8` and the absolute
/// tolerances of `cvRoberts_dns`.
fn robertson(solver: LinearSolver, end: f64, times: Vec<f64>) -> SimulationResult {
    let model = Robertson;
    let mut req = request(0.0, end, Output::Times(times), &[]);
    req.settings = IntegratorSettings {
        relative_tolerance: 1e-8,
        absolute_tolerance: Tolerance::PerState(vec![1e-12, 1e-14, 1e-10]),
        linear_solver: solver,
        ..IntegratorSettings::default()
    };
    simulate(&model, &model.description(), &req).unwrap()
}

#[test]
fn dense_and_klu_same_result() {
    // Up to t = 4 CVODE takes the same steps with both linear solvers (measured in
    // sbmlsim-csolvers), the solutions differ only by the rounding of the factorizations.
    let times = vec![0.0, 0.04, 0.4, 1.0, 4.0];
    let dense = robertson(LinearSolver::Dense, 4.0, times.clone());
    let klu = robertson(LinearSolver::Klu, 4.0, times);
    assert_eq!(dense.columns, ["time", "y1", "y2", "y3"]);
    assert_eq!(dense.columns, klu.columns);
    assert_eq!(dense.n_rows, klu.n_rows);
    for (d, k) in dense.values.iter().zip(&klu.values) {
        assert!(
            *d == *k || relative(*k, *d) < 1e-10,
            "dense {d:e} and KLU {k:e} differ"
        );
    }
    // Later the step sequences part; at 4e10 both are within the tolerances of the integration
    // of each other (the weighted RMS norm of the difference with the weights `rtol |y| +
    // 10 atol` is below 1) and of the reference `y1(4e10) = 5.2083e-08` of `cvRoberts_dns`.
    let times: Vec<f64> = (0..12).map(|i| 0.4 * 10f64.powi(i)).collect();
    let dense = robertson(LinearSolver::Dense, 4e10, times.clone());
    let klu = robertson(LinearSolver::Klu, 4e10, times);
    assert_eq!(*column(&dense, "time").last().unwrap(), 4e10);
    let atol = [1e-12, 1e-14, 1e-10];
    let last = |r: &SimulationResult| r.values[(r.n_rows - 1) * 4 + 1..].to_vec();
    let (yd, yk) = (last(&dense), last(&klu));
    let norm = (yd
        .iter()
        .zip(&yk)
        .zip(atol)
        .map(|((d, k), a)| ((d - k) / (1e-8 * d.abs() + 10.0 * a)).powi(2))
        .sum::<f64>()
        / 3.0)
        .sqrt();
    assert!(norm < 1.0, "dense {yd:?} and KLU {yk:?} differ ({norm:e})");
    for y in [&yd, &yk] {
        assert!(relative(y[0], 5.2083e-08) < 1e-4, "y1(4e10) = {:e}", y[0]);
    }
}

#[test]
fn gmres_agrees_with_dense_on_stiff_robertson() {
    let times = vec![0.0, 1e-4, 0.04, 4.0, 400.0, 4e4];
    let end = *times.last().unwrap();
    let reference = robertson(LinearSolver::Dense, end, times.clone());
    let mut req = request(0.0, end, Output::Times(times), &[]);
    let atol = [1e-12, 1e-14, 1e-10];
    req.settings = IntegratorSettings {
        relative_tolerance: 1e-8,
        absolute_tolerance: Tolerance::PerState(atol.to_vec()),
        linear_solver: LinearSolver::Gmres,
        ..IntegratorSettings::default()
    };
    let mut engine = Engine::new(Box::new(Robertson), Robertson.description()).unwrap();
    for choice in [JacobianChoice::Analytic, JacobianChoice::DifferenceQuotient] {
        req.settings.jacobian = choice;
        let result = engine.simulate(&req).unwrap();
        assert_eq!(result.columns, reference.columns);
        assert_eq!(result.n_rows, reference.n_rows);
        for (row, expected) in result
            .values
            .as_chunks::<4>()
            .0
            .iter()
            .zip(reference.values.as_chunks::<4>().0)
        {
            assert_eq!(row[0], expected[0]);
            for ((v, r), a) in row[1..].iter().zip(&expected[1..]).zip(atol) {
                assert!(
                    (v - r).abs() < 50.0 * (1e-8 * r.abs() + a),
                    "{choice:?} at t={}: {v:e} versus {r:e}",
                    row[0]
                );
            }
        }
        let stats = engine.stats();
        assert!(stats.steps > 0 && stats.rhs_evals > 0 && stats.linear_setups > 0);
        assert!(stats.jacobian_evals > 0);
        assert!(stats.linear_iterations > 0 && stats.jacobian_vector_evals > 0);
        assert!(stats.preconditioner_evals > 0 && stats.preconditioner_solves > 0);
        assert_eq!(
            stats.jacobian_rhs_evals > 0,
            choice == JacobianChoice::DifferenceQuotient
        );
        assert_eq!(engine.simulate(&req).unwrap(), result);
        assert_eq!(engine.stats(), stats);
    }
    req.settings.linear_solver = LinearSolver::Dense;
    engine.simulate(&req).unwrap();
    let direct = engine.stats();
    assert_eq!(direct.linear_iterations, 0);
    assert_eq!(direct.linear_convergence_failures, 0);
    assert_eq!(direct.jacobian_vector_evals, 0);
    assert_eq!(direct.preconditioner_evals, 0);
    assert_eq!(direct.preconditioner_solves, 0);
}

#[test]
fn preinit_sets_before_init() {
    let mut req = request(0.0, 2.0, Output::Uniform { steps: 4 }, &[]);
    req.preinit = vec![("k".to_string(), 2.0)];
    let result = decay(0.5, &req).unwrap();
    assert_eq!(result.columns, ["time", "y", "t_half"]);
    assert_decay(&result, 2.0, 0.0, 1e-9);
    // `t_half` is an assignment rule of `k`, written by `init` and `assign`
    for t_half in column(&result, "t_half") {
        assert_eq!(t_half, 2f64.ln() / 2.0);
    }
    // the initial state of `y` can be set as well
    req.preinit = vec![("y".to_string(), 3.0)];
    let result = decay(0.5, &req).unwrap();
    assert_eq!(column(&result, "y")[0], 3.0);
    // an entity which is not settable and an unknown one are refused, naming the id and, for
    // an entity a rule determines, that rule
    for (id, rule) in [
        ("t_half", Some("its assignment rule determines it")),
        ("nothing", None),
    ] {
        req.preinit = vec![(id.to_string(), 1.0)];
        match decay(0.5, &req) {
            Err(EngineError::Request(message)) => {
                assert!(message.contains(id), "{message}");
                if let Some(rule) = rule {
                    assert!(message.contains(rule), "{message}");
                }
            }
            other => panic!("{id}: {other:?}"),
        }
    }
}

#[test]
fn engine_reuse_is_bitwise_identical() {
    let model = Robertson;
    let desc = model.description();
    let mut engine = Engine::new(Box::new(Robertson), desc.clone()).unwrap();
    let mut req = request(0.0, 100.0, Output::IntegratorSteps, &[]);
    req.settings.relative_tolerance = 1e-8;
    req.settings.stiff = true;
    let free = simulate(&model, &desc, &req).unwrap();
    for solver in [LinearSolver::Dense, LinearSolver::Klu, LinearSolver::Dense] {
        let mut r = req.clone();
        r.settings.linear_solver = solver;
        let first = engine.simulate(&r).unwrap();
        let second = engine.simulate(&r).unwrap();
        assert_eq!(first, second, "{solver:?}");
        if solver == LinearSolver::Dense {
            assert_eq!(first, free);
        }
    }
    // another output, another tolerance and another method in between change nothing of a
    // later run
    let mut adams = req.clone();
    adams.settings.stiff = false;
    adams.end = 0.01;
    let first = engine.simulate(&adams).unwrap();
    assert_eq!(engine.simulate(&adams).unwrap(), first);
    let mut other = req.clone();
    other.output = Output::Uniform { steps: 3 };
    other.settings.relative_tolerance = 1e-4;
    other.settings.maximum_time_step = 0.5;
    engine.simulate(&other).unwrap();
    assert_eq!(engine.simulate(&req).unwrap(), free);
}

#[test]
fn difference_quotient_without_analytic_jacobian() {
    // a model whose Jacobian is not analytic integrates with the sparse difference quotient,
    // with every linear solver
    let model = Decay { k: 0.5 };
    let mut desc = model.description();
    let JacobianKind::Analytic(pattern) = desc.jacobian.clone() else {
        unreachable!()
    };
    desc.jacobian = JacobianKind::ColoredDifference {
        pattern,
        colors: vec![0],
        reason: "a test".to_string(),
    };
    for solver in [LinearSolver::Auto, LinearSolver::Dense, LinearSolver::Klu] {
        let mut req = request(0.0, 10.0, Output::Uniform { steps: 10 }, &["time", "y"]);
        req.settings.linear_solver = solver;
        req.settings.stiff = true;
        let result = simulate(&model, &desc, &req).unwrap();
        assert_decay(&result, 0.5, 0.0, 1e-7);
    }
}

#[test]
fn contracts_are_checked() {
    let model = Decay { k: 0.5 };
    let mut desc = model.description();
    desc.contract = "model/0".to_string();
    match Engine::new(Box::new(Decay { k: 0.5 }), desc) {
        Err(EngineError::Request(m)) => {
            assert!(m.contains("model/0") && m.contains(MODEL_CONTRACT))
        }
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("accepted model/0"),
    }
    let mut req = request(0.0, 1.0, Output::Uniform { steps: 1 }, &[]);
    req.contract = "simulation/9".to_string();
    match decay(0.5, &req) {
        Err(EngineError::Request(m)) => {
            assert!(m.contains("simulation/9") && m.contains(SIMULATION_CONTRACT))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn integration_error_carries_the_message() {
    let mut req = request(0.0, 1e6, Output::Uniform { steps: 1 }, &[]);
    req.settings.maximum_num_steps = 5;
    match decay(0.5, &req) {
        Err(EngineError::Integration {
            flag,
            time,
            message,
        }) => {
            assert_eq!(flag, -1, "CV_TOO_MUCH_WORK");
            assert!(time > 0.0 && time < 1e6, "{time}");
            assert!(message.contains("mxstep"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn time_is_the_first_column() {
    let cases: [(&[&str], &[&str]); 3] = [
        (&["y"], &["time", "y"]),
        (&["time", "y"], &["time", "y"]),
        (&["y", "time"], &["time", "y", "time"]),
    ];
    for (selections, columns) in cases {
        let req = request(0.0, 10.0, Output::Uniform { steps: 10 }, selections);
        let result = decay(0.5, &req).unwrap();
        assert_eq!(result.columns, columns, "{selections:?}");
        assert_eq!(result.values.len(), result.n_rows * columns.len());
        assert_eq!(column(&result, "time")[10], 10.0);
        assert_decay(&result, 0.5, 0.0, 1e-9);
        if columns.len() == 3 {
            // a later `time` is a copy of the time of the row, bitwise
            for i in 0..result.n_rows {
                let row = &result.values[i * 3..i * 3 + 3];
                assert_eq!(row[2].to_bits(), row[0].to_bits(), "row {i}");
            }
        }
    }
}
