//! The fit driven ask/tell ([`Asking`]) is the fit of the batch: the same optimizer, the same
//! evaluations, the same optimum to the bit, when the host simulates as the batch does.
//!
//! The host here is the native engine of the batch, which simulates the plain request of every
//! simulation the fit asks for: the request of its condition with the values of its varying
//! targets as pre-initialization values, which is what the prepared request of the batch
//! executes.

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, model, pair};
use sbmlsim_abi::{EngineError, SimulationResult};
use sbmlsim_fit::{
    Algorithm, Ask, Asking, Evolution, Fit, Opening, Problem, Scale, Settings, Simulation,
};

/// Simulates what a fit asks for on the native engine.
fn simulate(simulations: &[Simulation]) -> Vec<Result<SimulationResult, EngineError>> {
    let model = model();
    let mut instance = model.instantiate_native().unwrap();
    simulations
        .iter()
        .map(|simulation| instance.simulate(&simulation.request))
        .collect()
}

/// Drives a fit to its end, and counts the asks.
fn drive(mut asking: Asking) -> (Fit, usize) {
    let mut asks = 0;
    loop {
        match asking.ask().unwrap() {
            Ask::Simulate(simulations) => {
                asks += 1;
                assert!(!simulations.is_empty());
                asking.tell(simulate(&simulations)).unwrap();
            }
            Ask::Done(fit) => return (fit, asks),
        }
    }
}

/// The fit of the batch, from the start of its parameters.
fn batch(settings: &Settings) -> Fit {
    let problem = Problem::new(pair(1.0, 2.0, Scale::Log10), model(), 2).unwrap();
    problem.optimize(settings).unwrap()
}

/// The fit driven ask/tell, from the definition, which it resolves itself.
fn asked(settings: &Settings) -> (Fit, usize) {
    let opening = Opening::new(pair(1.0, 2.0, Scale::Log10), &["model".to_string()]).unwrap();
    drive(Asking::open(opening, settings.clone()))
}

fn assert_same(batch: &Fit, asked: &Fit) {
    assert_eq!(batch.x, asked.x);
    assert_eq!(batch.values, asked.values);
    assert_eq!(batch.cost.to_bits(), asked.cost.to_bits());
    assert_eq!(batch.residuals, asked.residuals);
    assert_eq!(batch.iterations, asked.iterations);
    assert_eq!(batch.evaluations, asked.evaluations);
    assert_eq!(batch.jacobians, asked.jacobians);
    assert_eq!(batch.status, asked.status);
}

#[test]
fn the_trust_region_asked_for_its_evaluations_is_the_one_of_the_batch() {
    let settings = Settings::default();
    let (fit, asks) = asked(&settings);
    assert_same(&batch(&settings), &fit);
    assert!((fit.values[0] - K).abs() < 1e-6, "{:?}", fit.values);
    // the start, then one ask per evaluation the optimizer misses: the residuals and the
    // jacobian of a point are one ask
    assert!(
        asks > 2 && asks < 3 * (fit.iterations as usize + 2),
        "{asks}"
    );
}

#[test]
fn the_levenberg_marquardt_asked_for_its_evaluations_is_the_one_of_the_batch() {
    let settings = Settings {
        algorithm: Algorithm::LevenbergMarquardt,
        ..Settings::default()
    };
    let (fit, _) = asked(&settings);
    assert_same(&batch(&settings), &fit);
}

#[test]
fn the_evolution_and_its_polish_asked_for_their_evaluations_are_the_ones_of_the_batch() {
    let settings = Settings {
        algorithm: Algorithm::DifferentialEvolution(Evolution {
            seed: 7,
            population: Some(8),
            ..Evolution::default()
        }),
        max_iterations: 6,
        ..Settings::default()
    };
    let (fit, asks) = asked(&settings);
    assert_same(&batch(&settings), &fit);
    assert!(fit.status.contains("polish"), "{}", fit.status);
    // a generation is one ask of all of its members
    assert!(asks >= 1 + 1 + 6, "{asks}");
}

#[test]
fn the_tolerances_of_a_run_are_the_ones_of_the_batch() {
    let settings = Settings {
        step_tolerance: Some(1e-8),
        cost_tolerance: Some(1e-10),
        gradient_tolerance: Some(1e-12),
        ..Settings::default()
    };
    let (fit, _) = asked(&settings);
    assert_same(&batch(&settings), &fit);
}

#[test]
fn a_fit_of_a_resolved_problem_starts_where_it_is_told() {
    let opening = Opening::new(pair(1.0, 2.0, Scale::Log10), &["model".to_string()]).unwrap();
    let tables: Vec<SimulationResult> = simulate(&opening.simulations())
        .into_iter()
        .map(Result::unwrap)
        .collect();
    let problem = opening.resolve(&tables).unwrap();
    let from = vec![0.5_f64.log10(), 1.5_f64.log10()];
    let settings = Settings::default();
    let (fit, _) = drive(Asking::new(problem, settings.clone(), &from).unwrap());
    let batch = Problem::new(pair(1.0, 2.0, Scale::Log10), model(), 1)
        .unwrap()
        .optimize_from(&settings, &from)
        .unwrap();
    assert_same(&batch, &fit);
}

#[test]
fn a_fit_which_is_stopped_answers_with_the_point_it_reached() {
    let opening = Opening::new(pair(1.0, 2.0, Scale::Log10), &["model".to_string()]).unwrap();
    let mut asking = Asking::open(opening, Settings::default());
    let mut asks = 0;
    let fit = loop {
        match asking.ask().unwrap() {
            Ask::Simulate(simulations) => {
                asks += 1;
                if asks == 4 {
                    asking.stop();
                }
                asking.tell(simulate(&simulations)).unwrap();
            }
            Ask::Done(fit) => break fit,
        }
    };
    assert_eq!(fit.status, "UserRequested");
    assert!(fit.iterations < 3, "{}", fit.iterations);
}

/// A fit whose start is told, up to its first evaluation.
fn at_the_first_evaluation() -> (Asking, Vec<Simulation>) {
    let opening = Opening::new(pair(1.0, 2.0, Scale::Log10), &["model".to_string()]).unwrap();
    let mut asking = Asking::open(opening, Settings::default());
    let Ask::Simulate(start) = asking.ask().unwrap() else {
        panic!("the start is asked for first");
    };
    asking.tell(simulate(&start)).unwrap();
    let Ask::Simulate(first) = asking.ask().unwrap() else {
        panic!("the residuals and the jacobian of the start are asked for");
    };
    (asking, first)
}

#[test]
fn a_failed_integration_is_a_point_the_fit_does_not_take() {
    let (mut asking, first) = at_the_first_evaluation();
    let failed = first
        .iter()
        .map(|_| {
            Err(EngineError::Integration {
                flag: -1,
                message: "failed".to_string(),
                time: 0.0,
            })
        })
        .collect();
    asking.tell(failed).unwrap();
    // the residuals are `FAILED_RESIDUAL`, which the optimizer reads as any other
    let fit = drive(asking).0;
    assert!(
        fit.residuals
            .iter()
            .all(|r| *r == sbmlsim_fit::FAILED_RESIDUAL)
    );
}

#[test]
fn another_failure_of_the_engine_refuses_the_fit() {
    let (mut asking, first) = at_the_first_evaluation();
    let refused = first
        .iter()
        .map(|_| Err(EngineError::Request("refused".to_string())))
        .collect();
    assert!(asking.tell(refused).is_err());
}

#[test]
fn a_tell_is_one_result_per_simulation_asked_for() {
    let (mut asking, first) = at_the_first_evaluation();
    assert!(asking.tell(Vec::new()).is_err());
    // the ask is the same until it is told
    let Ask::Simulate(again) = asking.ask().unwrap() else {
        panic!("the same simulations are asked for");
    };
    assert_eq!(first, again);
}

#[test]
fn a_stopped_fit_of_an_optimizer_which_runs_again_from_its_start_keeps_its_steps() {
    // the Levenberg-Marquardt runs every step again from its start, which a stop must not
    // take for the boundary it ends at
    let settings = Settings {
        algorithm: Algorithm::LevenbergMarquardt,
        ..Settings::default()
    };
    let (full, _) = asked(&settings);
    let opening = Opening::new(pair(1.0, 2.0, Scale::Log10), &["model".to_string()]).unwrap();
    let mut asking = Asking::open(opening, settings);
    let mut asks = 0;
    let fit = loop {
        match asking.ask().unwrap() {
            Ask::Simulate(simulations) => {
                asks += 1;
                asking.tell(simulate(&simulations)).unwrap();
                if asks == 6 {
                    asking.stop();
                }
            }
            Ask::Done(fit) => break fit,
        }
    };
    assert_eq!(fit.status, "UserRequested");
    assert!(
        fit.iterations > 0 && fit.iterations < full.iterations,
        "{}",
        fit.iterations
    );
}
