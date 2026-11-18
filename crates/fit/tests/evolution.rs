//! The differential evolution of a fit: the global search of a model whose optimum is
//! unknown, where one generation of the population is one batch of the engine.

use sbmlsim_fit::{Algorithm, Evolution, Scale, Settings};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, problem};

/// The settings of an evolution of `generations` generations.
fn settings(evolution: Evolution, generations: u64) -> Settings {
    Settings {
        algorithm: Algorithm::DifferentialEvolution(evolution),
        max_iterations: generations,
        ..Settings::default()
    }
}

/// The evolution of a population of twelve, the same every time it runs.
fn evolution() -> Evolution {
    Evolution {
        seed: 7,
        population: Some(12),
        ..Evolution::default()
    }
}

#[test]
fn the_evolution_finds_the_parameter_of_the_data() {
    let problem = problem(3.0, Scale::Log10, 4);

    let fit = problem.optimize(&settings(evolution(), 40)).unwrap();

    assert!(
        (fit.values[0] - K).abs() < 1e-6,
        "the evolution found k = {} instead of {K} in {} evaluations, {}",
        fit.values[0],
        fit.evaluations,
        fit.status
    );
    assert!(fit.cost < 1e-12, "the cost of the optimum is {}", fit.cost);
    assert!(
        fit.evaluations >= 12,
        "{} evaluations are less than one generation",
        fit.evaluations
    );
}

#[test]
fn the_threads_of_the_batch_do_not_reach_the_optimum_of_an_evolution() {
    // the same seed is the same population, and the points of a generation are reduced in the
    // order of the generation whatever thread ran them, so the arithmetic is the same
    let one = problem(3.0, Scale::Log10, 1);
    let many = problem(3.0, Scale::Log10, 8);
    let settings = settings(
        Evolution {
            polish: false,
            ..evolution()
        },
        20,
    );

    let serial = one.optimize(&settings).unwrap();
    let parallel = many.optimize(&settings).unwrap();

    assert_eq!(serial.x, parallel.x);
    assert_eq!(serial.cost, parallel.cost);
    assert_eq!(serial.evaluations, parallel.evaluations);
}

#[test]
fn another_seed_is_another_population() {
    let problem = problem(3.0, Scale::Log10, 4);
    let plain = settings(
        Evolution {
            polish: false,
            ..evolution()
        },
        8,
    );
    let other = settings(
        Evolution {
            seed: 99,
            polish: false,
            ..evolution()
        },
        8,
    );

    let one = problem.optimize(&plain).unwrap();
    let two = problem.optimize(&other).unwrap();

    assert_ne!(one.x, two.x, "two seeds searched the same points");
    for fit in [&one, &two] {
        assert!(
            (fit.values[0] - K).abs() < 0.1,
            "{} is not near {K}",
            fit.values[0]
        );
    }
}

#[test]
fn an_evolution_of_one_generation_gives_the_best_of_its_population() {
    let problem = problem(3.0, Scale::Log10, 4);

    let fit = problem
        .optimize(&settings(
            Evolution {
                polish: false,
                ..evolution()
            },
            1,
        ))
        .unwrap();

    assert_eq!(fit.status, "MaxIter");
    assert!(
        fit.cost.is_finite(),
        "the cost of one generation is not finite"
    );
    assert_eq!(fit.iterations, 1);
    assert_eq!(fit.jacobians, 0, "an evolution takes no jacobian");
}

#[test]
fn the_polish_of_an_evolution_sharpens_its_optimum() {
    let problem = problem(3.0, Scale::Log10, 4);
    let generations = 6;

    let rough = problem
        .optimize(&settings(
            Evolution {
                polish: false,
                ..evolution()
            },
            generations,
        ))
        .unwrap();
    let polished = problem
        .optimize(&settings(
            Evolution {
                polish: true,
                ..evolution()
            },
            generations,
        ))
        .unwrap();

    assert!(
        polished.cost < rough.cost,
        "the polish of {} did not sharpen it: {}",
        rough.cost,
        polished.cost
    );
    assert!(
        (polished.values[0] - K).abs() < (rough.values[0] - K).abs(),
        "the polish of k = {} is {}, further from {K}",
        rough.values[0],
        polished.values[0]
    );
    assert!(
        polished.jacobians > 0,
        "the polish took no jacobian of its own"
    );
}
