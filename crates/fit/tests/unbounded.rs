//! A fit of parameters which are not bounded: a bound of `-inf` or `inf`, and a lower bound of
//! zero of a parameter searched on a logarithmic scale, whose search space is the real line.
//!
//! The least squares optimizers of basin search a space which is not bounded as they search a
//! box; a differential evolution draws its population in the box and refuses it.

use sbmlsim_fit::{Algorithm, Evolution, Prior, PriorKind, Problem, Scale, Settings};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, definition, model};

/// The problem of the fit of `k` from 0.5 between two bounds, on a scale.
fn unbounded(lower: f64, upper: f64, scale: Scale) -> Problem {
    let mut definition = definition(0.5, scale);
    definition.parameters[0].lower = lower;
    definition.parameters[0].upper = upper;
    Problem::new(definition, model(), 2).unwrap()
}

#[test]
fn a_parameter_without_bounds_is_fitted() {
    for (lower, upper, scale) in [
        (f64::NEG_INFINITY, f64::INFINITY, Scale::Lin),
        (0.0, f64::INFINITY, Scale::Lin),
        (0.0, f64::INFINITY, Scale::Log10),
        (0.0, f64::INFINITY, Scale::Log),
        (0.01, f64::INFINITY, Scale::Log10),
        (0.0, 10.0, Scale::Log10),
    ] {
        let problem = unbounded(lower, upper, scale);
        let (low, high) = problem.bounds();
        assert_eq!(
            (low[0], high[0]),
            (scale.to_search(lower), scale.to_search(upper)),
            "the bounds of the search space"
        );
        for algorithm in [Algorithm::TrustRegion, Algorithm::LevenbergMarquardt] {
            let fit = problem
                .optimize(&Settings {
                    algorithm,
                    max_iterations: 200,
                    ..Settings::default()
                })
                .unwrap();

            assert!(
                (fit.values[0] - K).abs() < 1e-6,
                "{algorithm:?} of k in [{lower}, {upper}] on {scale:?} found {} ({})",
                fit.values[0],
                fit.status
            );
        }
    }
}

#[test]
fn a_differential_evolution_of_a_parameter_without_bounds_is_refused() {
    for (lower, upper, scale) in [
        (0.01, f64::INFINITY, Scale::Lin),
        (f64::NEG_INFINITY, 10.0, Scale::Lin),
        (0.0, 10.0, Scale::Log10),
    ] {
        let problem = unbounded(lower, upper, scale);

        let refusal = problem
            .optimize(&Settings {
                algorithm: Algorithm::DifferentialEvolution(Evolution::default()),
                max_iterations: 5,
                ..Settings::default()
            })
            .unwrap_err()
            .to_string();

        assert!(
            refusal.starts_with("the parameter 'k' is searched between")
                && refusal.contains("a differential evolution draws its population in a box"),
            "{refusal}"
        );
    }
}

#[test]
fn a_prior_of_a_parameter_without_bounds_is_its_row() {
    // a normal prior over the real line: the row of the prior is the density of the kind,
    // which no truncation normalizes
    let mut definition = definition(0.5, Scale::Lin);
    definition.parameters[0].lower = f64::NEG_INFINITY;
    definition.parameters[0].upper = f64::INFINITY;
    let (mu, sigma, offset) = (0.4, 0.2, 2.0);
    definition.priors = vec![Prior {
        parameter: 0,
        kind: PriorKind::Normal,
        parameters: vec![mu, sigma],
        offset,
        normalizer: 0.0,
        lower: f64::NEG_INFINITY,
        upper: f64::INFINITY,
    }];
    let problem = Problem::new(definition, model(), 2).unwrap();

    for value in [-3.0, 0.5, 7.0] {
        let residuals = problem.residuals(&[value]).unwrap();
        let row = residuals[residuals.len() - 1];
        let z: f64 = (value - mu) / sigma;
        let density = -0.5 * z * z - sigma.ln() - sbmlsim_fit::LN_2PI_HALF;
        assert!(
            (0.5 * row * row - 0.5 * offset + density).abs() < 1e-12,
            "the row at {value} is {row}"
        );
    }
    let fit = problem
        .optimize(&Settings {
            max_iterations: 200,
            ..Settings::default()
        })
        .unwrap();
    assert!(fit.cost.is_finite() && fit.values[0] > 0.0, "{fit:?}");
}

#[test]
fn a_start_decades_below_the_optimum_reaches_it() {
    // a start three decades below the optimum, on the scale whose search space is the real
    // line: the trust region walks up to it with no bound in the way
    let mut definition = definition(3e-4, Scale::Log10);
    definition.parameters[0].lower = 0.0;
    definition.parameters[0].upper = f64::INFINITY;
    let problem = Problem::new(definition, model(), 2).unwrap();

    let fit = problem
        .optimize(&Settings {
            max_iterations: 200,
            ..Settings::default()
        })
        .unwrap();

    assert!((fit.values[0] - K).abs() < 1e-6, "{fit:?}");
}
