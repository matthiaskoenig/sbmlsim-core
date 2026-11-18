//! The identifiability of the parameters of a fit: the profile of the likelihood of one of
//! them and the Fisher information of the optimum.
//!
//! The fit is the decay model `y' = -k y` of two parameters, `k` and the initial value of
//! `y`, against the data of the model at `k = 0.3` and `y = 1`.

use sbmlsim_fit::{
    DELTA_95, Fisher, Problem, Profile, Scale, Settings, Stop, Walk, fisher, profile, profiles,
};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, model, next_to_each_other, one_point, pair};

/// The problem of the fit of `k` and `y` and the optimum of that fit.
fn fitted(scale: Scale) -> (Problem, Vec<f64>) {
    let problem = Problem::new(pair(0.5, 2.0, scale), model(), 4).unwrap();
    let fit = problem.optimize(&Settings::default()).unwrap();
    assert!(fit.cost < 1e-12, "the fit of the pair ends at {}", fit.cost);
    (problem, fit.x)
}

/// A walk of four points on each side, half a decade wide.
fn walk() -> Walk {
    Walk {
        points: 4,
        span: 0.5,
        delta: DELTA_95,
    }
}

#[test]
fn a_problem_which_holds_a_parameter_is_the_fit_of_the_others() {
    let problem = Problem::new(pair(0.5, 2.0, Scale::Lin), model(), 4).unwrap();

    let held = problem.fixing(&[(0, 0.3)]).unwrap();

    assert_eq!(held.parameters().len(), 1);
    assert_eq!(held.parameters()[0].id, "y");
    assert_eq!(held.n_residuals(), problem.n_residuals());
    // the model sees the value the parameter is held at and the one of the search space
    assert_eq!(
        held.residuals(&[2.0]).unwrap(),
        problem.residuals(&[0.3, 2.0]).unwrap()
    );
    // and the fit of the parameter which is left finds the `y` of the data
    let fit = held.optimize_from(&Settings::default(), &[2.0]).unwrap();
    assert!(
        (fit.values[0] - 1.0).abs() < 1e-6,
        "the held fit found y = {}",
        fit.values[0]
    );
    assert_eq!(fit.values.len(), 1);
}

#[test]
fn a_fit_cannot_hold_a_parameter_it_has_not_or_every_one_of_them() {
    let problem = Problem::new(pair(0.5, 2.0, Scale::Lin), model(), 4).unwrap();

    let unknown = problem.fixing(&[(2, 0.3)]).unwrap_err().to_string();
    let every = problem
        .fixing(&[(0, 0.3), (1, 1.0)])
        .unwrap_err()
        .to_string();
    let outside = problem.fixing(&[(0, 50.0)]).unwrap_err().to_string();

    assert!(unknown.contains("none of the 2 parameters"), "{unknown}");
    assert!(every.contains("not held at a value"), "{every}");
    assert!(outside.contains("held at 50"), "{outside}");
}

#[test]
fn the_profile_of_a_parameter_rises_on_both_sides_of_the_optimum() {
    let (problem, at) = fitted(Scale::Log10);

    let profile: Profile = profile(
        &problem,
        &Settings::default(),
        &at,
        0,
        &walk(),
        &Stop::new(),
    )
    .unwrap();

    assert_eq!(profile.id, "k");
    assert_eq!(profile.points.len(), 8, "four points on each side");
    assert!(
        (profile.at - K).abs() < 1e-6,
        "the optimum is {}",
        profile.at
    );
    assert!(
        profile
            .points
            .windows(2)
            .all(|pair| pair[0].value < pair[1].value),
        "the points of the profile are not in the order of their value"
    );
    for point in &profile.points {
        assert!(
            point.cost > profile.cost,
            "the point {} of the profile is below the optimum: {} <= {}",
            point.value,
            point.cost,
            profile.cost
        );
        assert_eq!(point.values.len(), 1, "the other parameter of every point");
    }
    // the cost rises away from the optimum on both sides
    let below: Vec<f64> = profile
        .points
        .iter()
        .filter(|point| point.value < profile.at)
        .map(|point| point.cost)
        .collect();
    assert!(
        below.windows(2).all(|pair| pair[0] > pair[1]),
        "the costs below the optimum do not fall towards it: {below:?}"
    );
}

#[test]
fn the_profile_of_a_parameter_brackets_it_at_the_threshold() {
    let (problem, at) = fitted(Scale::Log10);
    let walk = Walk {
        points: 6,
        span: 0.3,
        delta: DELTA_95,
    };

    let profile = profile(&problem, &Settings::default(), &at, 0, &walk, &Stop::new()).unwrap();

    assert!(profile.threshold > profile.cost);
    let (lower, upper) = (profile.lower.unwrap(), profile.upper.unwrap());
    assert!(
        lower < K && K < upper,
        "the interval [{lower}, {upper}] does not hold {K}"
    );
    assert!(profile.identifiable);
}

#[test]
fn the_interval_of_a_profile_is_the_error_of_the_fisher_information() {
    // where the cost is a parabola around the optimum, the two ways of bounding a parameter
    // are the same one: the threshold of the profile is `1.96` standard errors
    let (problem, at) = fitted(Scale::Log10);
    let walk = Walk {
        points: 6,
        span: 0.3,
        delta: DELTA_95,
    };

    let walked = profile(&problem, &Settings::default(), &at, 0, &walk, &Stop::new()).unwrap();
    let information = fisher(&problem, &at).unwrap();

    let error = information.errors.unwrap()[0];
    let expected = (2.0 * DELTA_95).sqrt() * error;
    let (lower, upper) = (walked.lower.unwrap(), walked.upper.unwrap());
    let (below, above) = (at[0] - lower.log10(), upper.log10() - at[0]);
    for side in [below, above] {
        assert!(
            (side / expected - 1.0).abs() < 0.1,
            "the profile bounds the parameter at {side} and the information at {expected}"
        );
    }
}

#[test]
fn the_profile_of_a_parameter_the_data_does_not_determine_is_flat() {
    // one measurement at the start of the simulation tells `y` and nothing about `k`
    let problem = Problem::new(one_point(Scale::Log10), model(), 4).unwrap();
    let at = problem.optimize(&Settings::default()).unwrap().x;

    let profile = profile(
        &problem,
        &Settings::default(),
        &at,
        0,
        &walk(),
        &Stop::new(),
    )
    .unwrap();

    assert!(
        !profile.identifiable,
        "a parameter the data says nothing about is identifiable"
    );
    assert!(profile.lower.is_none() && profile.upper.is_none());
    for point in &profile.points {
        assert!(
            point.cost < profile.threshold,
            "the profile rises at {}: {}",
            point.value,
            point.cost
        );
    }
}

#[test]
fn a_profile_which_is_stopped_ends_where_it_is() {
    let (problem, at) = fitted(Scale::Log10);
    let stop = Stop::new();
    stop.stop();

    let profile = profile(&problem, &Settings::default(), &at, 0, &walk(), &stop).unwrap();

    assert!(
        profile.points.len() < 8,
        "the stopped profile walked {} points",
        profile.points.len()
    );
}

#[test]
fn the_fisher_information_of_the_optimum_gives_the_errors_of_its_parameters() {
    let (problem, at) = fitted(Scale::Log10);

    let fisher: Fisher = fisher(&problem, &at).unwrap();

    assert_eq!(fisher.parameters, vec!["k".to_string(), "y".to_string()]);
    assert_eq!(fisher.information.len(), 4);
    assert_eq!(
        fisher.information[1], fisher.information[2],
        "the information is not symmetric"
    );
    let errors = fisher.errors.unwrap();
    assert_eq!(errors.len(), 2);
    assert!(
        errors.iter().all(|error| *error > 0.0 && error.is_finite()),
        "{errors:?}"
    );
    let correlation = fisher.correlation.unwrap();
    assert!((correlation[0] - 1.0).abs() < 1e-12);
    assert!((correlation[3] - 1.0).abs() < 1e-12);
    assert_eq!(correlation[1], correlation[2]);
    assert!(correlation[1].abs() <= 1.0 + 1e-9, "{correlation:?}");
    assert!(fisher.singular.is_none());
}

#[test]
fn the_fisher_information_of_a_parameter_the_data_does_not_determine_is_singular() {
    let problem = Problem::new(one_point(Scale::Log10), model(), 4).unwrap();
    let at = problem.optimize(&Settings::default()).unwrap().x;

    let fisher = fisher(&problem, &at).unwrap();

    assert!(fisher.covariance.is_none());
    assert!(fisher.errors.is_none());
    assert!(fisher.correlation.is_none());
    let singular = fisher.singular.unwrap();
    assert!(singular.contains("k"), "{singular}");
}

#[test]
fn the_profiles_of_the_parameters_of_a_fit_run_on_the_threads_of_its_batch() {
    // the profiles of both parameters of the fit, every point of them a fit of the other one:
    // the chains of the walk are the two sides of each of the two profiles, four of them, and
    // they run next to each other
    let (problem, at) = fitted(Scale::Log10);

    let began = std::time::Instant::now();
    let walked = profiles(
        &problem,
        &Settings::default(),
        &at,
        &[],
        &walk(),
        &Stop::new(),
    )
    .unwrap();
    let took = began.elapsed().as_secs_f64();

    assert_eq!(walked.len(), 2);
    let chains: Vec<f64> = walked.iter().flat_map(sides).collect();
    assert_eq!(chains.len(), 4);
    next_to_each_other(took, &chains, "the walk of the two profiles");
    // the two sides of one profile run next to each other as well
    let one = &walked[0];
    next_to_each_other(
        one.seconds,
        &sides(one),
        &format!("the profile of {}", one.id),
    );
}

/// The seconds of the points below the optimum and the ones above it, the two chains of a
/// profile: the fit of a point starts at the optimum of the point before it on its own side.
fn sides(profile: &Profile) -> [f64; 2] {
    let mut sides = [0.0; 2];
    for point in &profile.points {
        sides[usize::from(point.value > profile.at)] += point.seconds;
    }
    sides
}
