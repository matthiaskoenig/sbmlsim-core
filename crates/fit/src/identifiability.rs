//! The identifiability of the parameters of a fit: the profile of the likelihood of a
//! parameter and the Fisher information of an optimum.
//!
//! A fit which found an optimum has not said how well the data determines it. Two answers:
//!
//! - **The profile of the likelihood** ([`profile`]) holds one parameter at a value and fits
//!   the others again, at a grid of values around the optimum. The cost of those fits is the
//!   profile, and where it rises by [`Walk::delta`] above the optimum are the bounds of the
//!   confidence interval of the parameter. A profile which does not rise is a parameter the
//!   data does not determine, which is what the analysis is for.
//! - **The Fisher information** ([`fisher`]) is the curvature of the cost at the optimum,
//!   `JᵀJ` of the residuals there. Its inverse is the covariance of the parameters, its
//!   diagonal their standard errors and its scaled form their correlations, in the space the
//!   parameters are searched. It costs one jacobian against the hundreds of fits of a profile
//!   and holds only where the cost is a parabola around the optimum.
//!
//! The points of a profile are fits of their own, and every one of them fans its jacobian out
//! over the threads of the engine, so the profile walks them one after the other: the fit of
//! a point starts from the optimum of the point before it, which is what makes a profile
//! cheap and its path smooth.

use basin::{DenseMatrix, GramMatrix, LinearSolveSpd};
use serde::{Deserialize, Serialize};

use crate::problem::{FitError, Problem, Scale};
use crate::{Settings, Stop};

/// The rise of the cost which bounds a parameter at 95 %, `chi2(0.95, 1) / 2`.
///
/// The cost of a fit is half the sum of the squares of its residuals, so the likelihood ratio
/// of a parameter is twice the rise of the cost and the quantile of the chi square
/// distribution of one degree of freedom is halved here.
pub const DELTA_95: f64 = 1.920_729_410_347_062;

/// What a profile walks and where it stops.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Walk {
    /// The points of the profile on each side of the optimum.
    pub points: usize,
    /// How far the profile reaches on each side, in the space the parameter is searched: a
    /// decade for a parameter of the scale `log10`, the value itself for a linear one. The
    /// bounds of the parameter end the walk before it.
    pub span: f64,
    /// The rise of the cost which bounds the parameter, [`DELTA_95`] by default.
    pub delta: f64,
}

impl Default for Walk {
    fn default() -> Walk {
        Walk {
            points: 10,
            span: 1.0,
            delta: DELTA_95,
        }
    }
}

/// One point of a profile: the parameter held at a value and the fit of the others there.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProfilePoint {
    /// The value the parameter is held at.
    pub value: f64,
    /// That value in the space the parameter is searched.
    pub x: f64,
    /// Half the sum of the squares of the residuals of the fit of the other parameters.
    pub cost: f64,
    /// The values the other parameters reached, in their order in the fit.
    pub values: Vec<f64>,
    /// Why the optimizer of the point stopped.
    pub status: String,
    /// The time the point took.
    pub seconds: f64,
}

/// The profile of the likelihood of one parameter.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Profile {
    /// The id of the parameter.
    pub id: String,
    /// Its value at the optimum of the fit.
    pub at: f64,
    /// The cost of that optimum.
    pub cost: f64,
    /// The points of the profile, in the order of their value.
    pub points: Vec<ProfilePoint>,
    /// The cost which bounds the parameter, the cost of the optimum and [`Walk::delta`].
    pub threshold: f64,
    /// The value below the optimum where the profile reaches that cost, interpolated between
    /// the points around it; absent when the profile does not reach it on that side.
    pub lower: Option<f64>,
    /// The value above the optimum where it reaches that cost.
    pub upper: Option<f64>,
    /// Whether the profile bounds the parameter on both sides, which is what the data
    /// determining it means.
    pub identifiable: bool,
    /// The time the profile took.
    pub seconds: f64,
}

/// The Fisher information of an optimum and what follows from it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Fisher {
    /// The ids of the parameters, in the order of the rows and the columns.
    pub parameters: Vec<String>,
    /// `JᵀJ` of the residuals at the optimum, row major: the information of the fit about its
    /// parameters, in the space they are searched.
    pub information: Vec<f64>,
    /// Its inverse, the covariance of the parameters; absent when the information is
    /// singular. It is the covariance of the estimate where the residuals of the fit are
    /// weighted by the standard deviation of their measurement, as the noise of a curve does.
    pub covariance: Option<Vec<f64>>,
    /// The square roots of the diagonal of that covariance, the standard errors.
    pub errors: Option<Vec<f64>>,
    /// The covariance scaled by those errors, the correlations of the parameters, row major.
    pub correlation: Option<Vec<f64>>,
    /// Why there is no covariance: the information is singular, with the parameters the data
    /// says nothing about when there are any.
    pub singular: Option<String>,
}

/// The profile of the likelihood of one parameter of a fit, around an optimum.
///
/// `at` is the optimum in the space the parameters are searched, as [`crate::Fit::x`] gives
/// it, and `parameter` the index of the parameter in [`Problem::parameters`]. A profile holds
/// that parameter at the values of the walk and fits the others from the optimum of the point
/// before, so the points of a profile cost about as much as one fit each and not more. A
/// [`Stop`] ends the profile where it is.
///
/// # Errors
///
/// If the index is none of the parameters of the fit, if the fit has only that one parameter,
/// if the optimum is not its parameters, or as [`Problem::optimize_with`].
pub fn profile(
    problem: &Problem,
    settings: &Settings,
    at: &[f64],
    parameter: usize,
    walk: &Walk,
    stop: &Stop,
) -> Result<Profile, FitError> {
    let began = std::time::Instant::now();
    let parameters = problem.parameters();
    if at.len() != parameters.len() {
        return Err(FitError::Optimizer(format!(
            "has the optimum of {} values of the {} parameters of the fit",
            at.len(),
            parameters.len()
        )));
    }
    let subject = parameters
        .get(parameter)
        .ok_or_else(|| FitError::Parameter {
            id: format!("{parameter}"),
            message: format!("is none of the {} parameters of the fit", parameters.len()),
        })?;
    if parameters.len() < 2 {
        return Err(FitError::Empty(
            "parameter to fit again while one is held at a value",
        ));
    }
    if walk.points == 0 || !walk.span.is_finite() || walk.span <= 0.0 {
        return Err(FitError::Optimizer(format!(
            "walks {} points of a span of {}, which is no profile",
            walk.points, walk.span
        )));
    }
    let scale = subject.scale;
    let cost = problem.cost(at)?;
    let x = at[parameter];
    let bounds = (
        scale.to_search(subject.lower),
        scale.to_search(subject.upper),
    );
    let step = walk.span / walk.points as f64;
    // the optimum of the parameters which are fitted again, which the first point starts from
    let free: Vec<f64> = at
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != parameter)
        .map(|(_, value)| *value)
        .collect();
    let mut points: Vec<ProfilePoint> = Vec::with_capacity(2 * walk.points);
    for direction in [-1.0, 1.0] {
        let mut from = free.clone();
        for point in 1..=walk.points {
            if stop.stopped() {
                break;
            }
            let x = x + direction * step * point as f64;
            if x < bounds.0 || x > bounds.1 {
                break;
            }
            let value = scale.to_model(x);
            let held = problem.fixing(&[(parameter, value)])?;
            let fit = held.optimize_with(settings, &from, stop)?;
            from = fit.x.clone();
            points.push(ProfilePoint {
                value,
                x,
                cost: fit.cost,
                values: fit.values,
                status: fit.status,
                seconds: fit.seconds,
            });
        }
    }
    points.sort_by(|one, two| one.x.total_cmp(&two.x));
    let threshold = cost + walk.delta;
    let lower = crossing(
        (x, cost),
        points
            .iter()
            .filter(|point| point.x < x)
            .rev()
            .map(|point| (point.x, point.cost)),
        threshold,
        scale,
    );
    let upper = crossing(
        (x, cost),
        points
            .iter()
            .filter(|point| point.x > x)
            .map(|point| (point.x, point.cost)),
        threshold,
        scale,
    );
    Ok(Profile {
        id: subject.id.clone(),
        at: scale.to_model(x),
        cost,
        points,
        threshold,
        identifiable: lower.is_some() && upper.is_some(),
        lower,
        upper,
        seconds: began.elapsed().as_secs_f64(),
    })
}

/// The profiles of the parameters of a fit, in the order of `which`, or of every parameter
/// when it is empty.
///
/// # Errors
///
/// As [`profile`].
pub fn profiles(
    problem: &Problem,
    settings: &Settings,
    at: &[f64],
    which: &[usize],
    walk: &Walk,
    stop: &Stop,
) -> Result<Vec<Profile>, FitError> {
    let every: Vec<usize> = (0..problem.parameters().len()).collect();
    let which = if which.is_empty() { &every } else { which };
    let mut profiles = Vec::with_capacity(which.len());
    for parameter in which {
        profiles.push(profile(problem, settings, at, *parameter, walk, stop)?);
        if stop.stopped() {
            break;
        }
    }
    Ok(profiles)
}

/// The value where the cost of a walk reaches the threshold, interpolated between the point
/// below it and the one above it in the space of the search.
///
/// The interpolation runs on the square root of the rise of the cost, which is a line in the
/// parameter wherever the cost is a parabola around the optimum, so a grid of a few points
/// per side gives the crossing of a quadratic profile exactly. On the cost itself the same
/// grid would cut the interval in half: between a point of a rise of 8 and the optimum, a
/// line reaches a threshold of 1.92 at a quarter of the step, where the parabola reaches it
/// at a half.
fn crossing(
    from: (f64, f64),
    walk: impl Iterator<Item = (f64, f64)>,
    threshold: f64,
    scale: Scale,
) -> Option<f64> {
    let (optimum, cost) = from;
    let target = (threshold - cost).max(0.0).sqrt();
    let mut last = (optimum, 0.0);
    for (x, at) in walk {
        let rise = (at - cost).max(0.0).sqrt();
        if at >= threshold {
            let share = if rise > last.1 {
                (target - last.1) / (rise - last.1)
            } else {
                0.0
            };
            return Some(scale.to_model(last.0 + share * (x - last.0)));
        }
        last = (x, rise);
    }
    None
}

/// The Fisher information of an optimum of a fit, and the errors and correlations of its
/// parameters.
///
/// One jacobian of the residuals at `at` ([`crate::Fit::x`]), which is one batch of the
/// engine, and the linear algebra of a `k x k` matrix.
///
/// # Errors
///
/// If the optimum is not the parameters of the fit, or as [`Problem::jacobian`]. A singular
/// information is no error: it is [`Fisher::singular`], with the parameters the data says
/// nothing about.
pub fn fisher(problem: &Problem, at: &[f64]) -> Result<Fisher, FitError> {
    let parameters = problem.parameters();
    if at.len() != parameters.len() {
        return Err(FitError::Optimizer(format!(
            "has the optimum of {} values of the {} parameters of the fit",
            at.len(),
            parameters.len()
        )));
    }
    let k = parameters.len();
    let jacobian = DenseMatrix::from_row_slice(problem.n_residuals(), k, &problem.jacobian(at)?);
    let gram = jacobian.gram();
    let mut information = vec![0.0; k * k];
    for row in 0..k {
        for column in 0..k {
            information[row * k + column] = gram.get(row, column);
        }
    }
    // the parameters whose own information is nothing: the residuals do not move with them,
    // whatever the other parameters do, which is the plainest case of a singular information
    let flat: Vec<&str> = (0..k)
        .filter(|at| information[at * k + at] <= 0.0)
        .map(|at| parameters[at].id.as_str())
        .collect();
    let mut covariance = vec![0.0; k * k];
    for column in 0..k {
        let mut unit = vec![0.0; k];
        unit[column] = 1.0;
        match gram.solve_spd(&unit) {
            Ok(values) => {
                for row in 0..k {
                    covariance[row * k + column] = values[row];
                }
            }
            Err(error) => {
                let message = if flat.is_empty() {
                    format!("the information of the fit cannot be inverted: {error}")
                } else {
                    format!(
                        "the information of the fit cannot be inverted ({error}): the data \
                         does not determine {}",
                        flat.join(", ")
                    )
                };
                return Ok(Fisher {
                    parameters: parameters.iter().map(|p| p.id.clone()).collect(),
                    information,
                    covariance: None,
                    errors: None,
                    correlation: None,
                    singular: Some(message),
                });
            }
        }
    }
    // the columns are solved one at a time, so the matrix of their values is symmetric only
    // to the last bits of its doubles, and a covariance is symmetric
    for row in 0..k {
        for column in 0..row {
            let mean = 0.5 * (covariance[row * k + column] + covariance[column * k + row]);
            covariance[row * k + column] = mean;
            covariance[column * k + row] = mean;
        }
    }
    let errors: Vec<f64> = (0..k).map(|at| covariance[at * k + at].sqrt()).collect();
    let mut correlation = vec![0.0; k * k];
    for row in 0..k {
        for column in 0..k {
            let scale = errors[row] * errors[column];
            correlation[row * k + column] = if scale > 0.0 {
                covariance[row * k + column] / scale
            } else {
                f64::NAN
            };
        }
    }
    Ok(Fisher {
        parameters: parameters.iter().map(|p| p.id.clone()).collect(),
        information,
        covariance: Some(covariance),
        errors: Some(errors),
        correlation: Some(correlation),
        singular: None,
    })
}
