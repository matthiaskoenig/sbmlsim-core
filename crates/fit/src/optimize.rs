//! The optimizers of a fit: the least squares of [basin] over a [`Problem`].
//!
//! A least squares optimizer asks for the residuals of a point of the search space and for
//! their jacobian, which is what [`Problem`] answers out of one batch of the engine. The
//! optimizers themselves are basin's: `TrustRegionReflective`, the `trf` of
//! `scipy.optimize.least_squares` of sbmlsim, and the Levenberg-Marquardt of a box.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use basin::{
    ApplicationStop, BoundedLevenbergMarquardt, BoxBounds, ConstraintDescription, ConstraintSet,
    ConstraintSolveError, Constraints, CostFunction, De, DenseMatrix, Executor, Jacobian,
    PopulationProgress, Residual, TrustRegionReflective,
};

use serde::{Deserialize, Serialize};

use crate::problem::{FitError, Problem};

/// A flag which stops a fit at its next iteration.
///
/// A fit of a large model runs for minutes, and a caller which cannot wait for it (the Ctrl-C
/// of a notebook, the budget of a web request) sets the flag instead of killing the thread:
/// the optimizer stops where it is and the fit answers with the best point it reached.
#[derive(Clone, Debug, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    /// A flag which is not set.
    #[must_use]
    pub fn new() -> Stop {
        Stop::default()
    }

    /// Stops the fits which hold the flag at their next iteration.
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether the flag is set.
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Which optimizer runs a fit.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    /// Trust region reflective, the `trf` of `scipy.optimize.least_squares`, which keeps its
    /// steps inside the bounds by reflecting them.
    #[default]
    TrustRegion,
    /// The Levenberg-Marquardt of a box, which solves a damped normal equation per step.
    LevenbergMarquardt,
    /// The differential evolution of Storn and Price (`DE/rand/1/bin`), the global search of
    /// a fit whose optimum is unknown: a generation of its population is one batch of the
    /// engine, and none of its members needs a derivative.
    DifferentialEvolution(Evolution),
}

/// The population of a differential evolution and how it is drawn.
///
/// The defaults are basin's: a population of `max(4, 10 * parameters)`, the factor of the
/// mutation dithered between 0.5 and 1.0 per generation and a crossover of 0.9, which are the
/// ones of `scipy.optimize.differential_evolution` as well.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Evolution {
    /// The seed of the population, which repeats a run bit for bit.
    pub seed: u64,
    /// The members of the population, at least four; the default when it is absent.
    pub population: Option<usize>,
    /// The factor of the mutation, which fixes it instead of dithering it.
    pub f: Option<f64>,
    /// The probability of the crossover.
    pub cr: Option<f64>,
    /// Whether a least squares optimizer sharpens the best member at the end, as the
    /// `polish` of `scipy.optimize.differential_evolution`: a differential evolution finds
    /// the valley of the optimum and a gradient finds its bottom.
    pub polish: bool,
}

impl Default for Evolution {
    fn default() -> Evolution {
        Evolution {
            seed: 0,
            population: None,
            f: None,
            cr: None,
            polish: true,
        }
    }
}

/// What an optimizer may spend and when it stops.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// The optimizer of the fit.
    pub algorithm: Algorithm,
    /// The largest number of iterations.
    pub max_iterations: u64,
    /// The time the fit may take, after which the optimizer stops at its best point.
    pub max_seconds: Option<f64>,
    /// The tolerance of the scaled gradient a step converges at; the one of the optimizer
    /// when it is absent.
    pub gradient_tolerance: Option<f64>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            algorithm: Algorithm::default(),
            max_iterations: 100,
            max_seconds: None,
            gradient_tolerance: None,
        }
    }
}

/// What a fit found.
#[derive(Clone, Debug)]
pub struct Fit {
    /// The values of the parameters the optimizer searched, on the linear scale, in the
    /// order of [`Problem::parameters`]; a parameter a profile holds at a value is none of
    /// them.
    pub values: Vec<f64>,
    /// The optimum in the space the optimizer searched.
    pub x: Vec<f64>,
    /// Half the sum of the squares of the residuals of the optimum.
    pub cost: f64,
    /// The residuals of the optimum.
    pub residuals: Vec<f64>,
    /// The iterations the optimizer took.
    pub iterations: u64,
    /// The evaluations of the residuals it took.
    pub evaluations: u64,
    /// The jacobians it took.
    pub jacobians: u64,
    /// Why the optimizer stopped, its termination code.
    pub status: String,
    /// The time the fit took.
    pub seconds: f64,
}

/// A [`Problem`] as a least squares problem of basin.
///
/// The residuals and their jacobian are the ones of the problem, so an evaluation of the
/// optimizer is one batch of the engine and the jacobian of a step is one batch of
/// `(parameters + 1) * conditions` points.
struct LeastSquares<'a> {
    problem: &'a Problem,
    lower: Vec<f64>,
    upper: Vec<f64>,
}

impl Residual for LeastSquares<'_> {
    type Param = Vec<f64>;
    type Output = Vec<f64>;
    type Error = FitError;

    fn residual(&self, x: &Vec<f64>) -> Result<Vec<f64>, FitError> {
        self.problem.residuals(x)
    }
}

impl Jacobian for LeastSquares<'_> {
    type Jacobian = DenseMatrix;

    fn jacobian(&self, x: &Vec<f64>) -> Result<DenseMatrix, FitError> {
        let values = self.problem.jacobian(x)?;
        Ok(DenseMatrix::from_row_slice(
            self.problem.n_residuals(),
            self.problem.parameters().len(),
            &values,
        ))
    }
}

impl CostFunction for LeastSquares<'_> {
    type Param = Vec<f64>;
    type Output = f64;
    type Error = FitError;

    fn cost(&self, x: &Vec<f64>) -> Result<f64, FitError> {
        self.problem.cost(x)
    }
}

impl Constraints for LeastSquares<'_> {
    type Set = ConstraintSet<BoxBounds<Vec<f64>>>;

    fn constraints(&self) -> <Self::Set as ConstraintDescription>::Borrowed<'_> {
        ConstraintSet::new().with_bounds(BoxBounds::new(Some(&self.lower), Some(&self.upper)))
    }
}

/// The error of the optimizer, which carries the one of an evaluation.
fn fault(error: ConstraintSolveError<FitError>) -> FitError {
    match error {
        ConstraintSolveError::Evaluation(error) => error,
        ConstraintSolveError::InvalidConstraints(error) => {
            FitError::Optimizer(format!("refused the bounds of the fit: {error}"))
        }
        other => FitError::Optimizer(format!("failed: {other}")),
    }
}

/// Runs the optimizer of the settings over the problem, from a point of the search space.
///
/// The point is the start of the parameters of the problem for a fit, the best member of a
/// population for the polish of an evolution and the optimum of a fit for a step of a profile.
pub(crate) fn run(
    problem: &Problem,
    settings: &Settings,
    from: &[f64],
    stop: &Stop,
) -> Result<Fit, FitError> {
    let (lower, upper) = problem.bounds();
    let start: Vec<f64> = from.to_vec();
    if start.len() != problem.parameters().len() {
        return Err(FitError::Optimizer(format!(
            "was started at {} values of the {} parameters of the fit",
            start.len(),
            problem.parameters().len()
        )));
    }
    let began = Instant::now();
    // the time of a fit is the limit of the optimizer itself (`MaxTime`), the flag of a
    // caller which stops it is one check per iteration
    let budget = settings.max_seconds.map(Duration::from_secs_f64);
    let halt = {
        let stop = stop.clone();
        move || -> Option<ApplicationStop> {
            stop.stopped()
                .then(|| ApplicationStop::new("the fit was stopped"))
        }
    };
    // the two optimizers are two types, and their driver is the same five lines
    macro_rules! drive {
        ($solver:expr) => {{
            let least_squares = LeastSquares {
                problem,
                lower: lower.clone(),
                upper: upper.clone(),
            };
            let halt = halt.clone();
            let mut executor = Executor::from_start(least_squares, $solver, start.clone())
                .max_iter(settings.max_iterations)
                .stop_when(move |_| halt());
            if let Some(budget) = budget {
                executor = executor.max_time(budget);
            }
            executor.run().map_err(fault)?
        }};
    }
    let (mut x, mut iterations, mut counts, mut status) = match settings.algorithm {
        Algorithm::TrustRegion => {
            let mut solver = TrustRegionReflective::new();
            if let Some(tolerance) = settings.gradient_tolerance {
                solver = solver.with_absolute_scaled_gradient_tolerance(tolerance);
            }
            let result = drive!(solver);
            (
                result.param().clone(),
                result.iter(),
                *result.state.counts(),
                format!("{:?}", result.report.code()),
            )
        }
        Algorithm::LevenbergMarquardt => {
            let mut solver = BoundedLevenbergMarquardt::new();
            if let Some(tolerance) = settings.gradient_tolerance {
                solver = solver.with_absolute_scaled_gradient_tolerance(tolerance);
            }
            let result = drive!(solver);
            (
                result.param().clone(),
                result.iter(),
                *result.state.counts(),
                format!("{:?}", result.report.code()),
            )
        }
        Algorithm::DifferentialEvolution(evolution) => {
            if let Some(population) = evolution.population
                && population < 4
            {
                return Err(FitError::Optimizer(format!(
                    "needs a population of four members or more, not {population}"
                )));
            }
            let mut solver = De::<f64>::new(evolution.seed);
            if let Some(population) = evolution.population {
                solver = solver.with_pop_size(population);
            }
            if let Some(f) = evolution.f {
                solver = solver.with_f(f);
            }
            if let Some(cr) = evolution.cr {
                solver = solver.with_cr(cr);
            }
            // the generation of the population spreads over the threads of the engine, and
            // every member of it fans its own conditions out on them; the executor of the
            // run is built on a thread of the pool, as what it holds does not cross threads
            let max_iterations = settings.max_iterations;
            let result = problem
                .install(move || {
                    let least_squares = LeastSquares {
                        problem,
                        lower: lower.clone(),
                        upper: upper.clone(),
                    };
                    let mut executor =
                        Executor::new(least_squares, solver, PopulationProgress::empty())
                            .max_iter(max_iterations)
                            .stop_when(move |_| halt());
                    if let Some(budget) = budget {
                        executor = executor.max_time(budget);
                    }
                    executor.run()
                })
                .map_err(fault)?;
            (
                result.best_param().clone(),
                result.iter(),
                *result.state.counts(),
                format!("{:?}", result.report.code()),
            )
        }
    };
    // the polish of an evolution: a least squares optimizer from the best member it found
    if let Algorithm::DifferentialEvolution(evolution) = settings.algorithm
        && evolution.polish
    {
        let polished = run(
            problem,
            &Settings {
                algorithm: Algorithm::TrustRegion,
                ..settings.clone()
            },
            &x,
            stop,
        )?;
        status = format!("{status}, polish {}", polished.status);
        iterations += polished.iterations;
        counts.cost_evals += polished.evaluations;
        counts.residual_evals += polished.evaluations;
        counts.jacobian_evals += polished.jacobians;
        x = polished.x;
    }
    let residuals = problem.residuals(&x)?;
    Ok(Fit {
        values: problem.parameter_values(&x),
        cost: 0.5 * residuals.iter().map(|value| value * value).sum::<f64>(),
        residuals,
        x,
        iterations,
        evaluations: counts.residual_evals.max(counts.cost_evals),
        jacobians: counts.jacobian_evals,
        status,
        seconds: began.elapsed().as_secs_f64(),
    })
}
