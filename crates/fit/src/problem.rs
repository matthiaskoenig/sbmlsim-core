//! The problem of a fit: its definition, its residuals and its jacobian on the batch.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sbmlsim_abi::{EngineError, Output, SimulationRequest, SimulationResult};
use sbmlsim_batch::{BatchError, PointFailure, Workers};
use sbmlsim_host::{LoadedModel, PreparedSimulation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The residual of a point whose simulation failed, the one sbmlsim scores a failed
/// evaluation with (`LIKELIHOOD_FAILURE_RESIDUAL`): a large finite number, so that the step
/// of an optimizer which reaches it is rejected and the fit goes on, where an infinite
/// residual would end the run.
pub const FAILED_RESIDUAL: f64 = 1e3;

/// The relative step of the finite differences of a jacobian, the `diff_step` of sbmlsim.
///
/// A simulation is exact only to the tolerance of its integrator, so a step below the square
/// root of that tolerance measures the noise of the integrator instead of the slope of the
/// residuals and an optimizer stops next to its start.
pub const DIFF_STEP: f64 = 1e-3;

/// What is wrong with a fit, or with the engine under it.
#[derive(Debug, Error)]
pub enum FitError {
    /// The definition has no parameter, no condition or no curve.
    #[error("the fit has no {0}")]
    Empty(&'static str),
    /// A parameter of the fit cannot be fitted.
    #[error("the parameter '{id}' {message}")]
    Parameter {
        /// The id of the parameter.
        id: String,
        /// What is wrong with it.
        message: String,
    },
    /// A condition of the fit cannot be simulated.
    #[error("the condition {at} {message}")]
    Condition {
        /// The index of the condition in the definition.
        at: usize,
        /// What is wrong with it.
        message: String,
    },
    /// A curve of the fit cannot be read off the simulation of its condition.
    #[error("the curve {at} {message}")]
    Curve {
        /// The index of the curve in the definition.
        at: usize,
        /// What is wrong with it.
        message: String,
    },
    /// The pool of the batch was refused, e.g. for its number of threads.
    #[error("the pool of the batch: {0}")]
    Pool(#[from] BatchError),
    /// The engine refused a request or failed in itself; a failed integration is no error of
    /// the fit, it is [`FAILED_RESIDUAL`] for the points of its curve.
    #[error("the engine: {0}")]
    Engine(#[from] EngineError),
    /// A point of the batch failed outside the engine, e.g. its thread panicked.
    #[error("a point of the batch: {0}")]
    Point(String),
    /// The optimizer refused the problem.
    #[error("the optimizer of the fit {0}")]
    Optimizer(String),
    /// The request of a fit of the contract `fit/1` does not resolve.
    #[error("the request of the fit {0}")]
    Request(String),
}

/// Where the jacobian of the residuals of a fit comes from.
///
/// The forward sensitivities of a condition are one simulation per condition and step of an
/// optimizer instead of one per parameter and condition, and the derivative of a residual
/// instead of a difference of two of them (1e-8 of itself against [`DIFF_STEP`]). What a run
/// of sensitivities does not carry (the events of a model, a discontinuity of its math, an
/// algebraic rule, a delay, the changes of a request, its steady state) is the finite
/// differences instead.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JacobianChoice {
    /// What is fastest for the model, which is the differences today: the difference
    /// quotient of `df/dp` inside a run of sensitivities costs about as much as the
    /// integrations the run saves (`bench/OPTIMIZATION.md`), so the sensitivities are worth
    /// their accuracy and not their time until a model carries an analytic `df/dp`.
    #[default]
    Auto,
    /// The sensitivities, and a condition which does not carry them is refused.
    Sensitivities,
    /// The finite differences of the residuals, whatever the conditions carry.
    DifferenceQuotient,
}

/// The scale an optimizer searches a parameter on.
///
/// The model always sees the value of the parameter ([`Scale::to_model`]); a parameter whose
/// bounds span decades is searched as its logarithm, so that a relative step of the optimizer
/// is a relative step of the parameter, which is what a fit of a PBPK model needs.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// The value itself.
    #[default]
    Lin,
    /// The natural logarithm of the value, which has to be positive.
    Log,
    /// The decadic logarithm of the value, which has to be positive.
    Log10,
}

impl Scale {
    /// The value of the parameter of a point of the search space.
    #[must_use]
    pub fn to_model(self, value: f64) -> f64 {
        match self {
            Scale::Lin => value,
            Scale::Log => value.exp(),
            Scale::Log10 => 10f64.powf(value),
        }
    }

    /// The point of the search space of the value of a parameter.
    #[must_use]
    pub fn to_search(self, value: f64) -> f64 {
        match self {
            Scale::Lin => value,
            Scale::Log => value.ln(),
            Scale::Log10 => value.log10(),
        }
    }

    /// The derivative of the value of the parameter in the point `value` of the search space,
    /// which the jacobian of the sensitivities of a run multiplies by: the chain rule of the
    /// scale, since a sensitivity is the derivative in the value the model sees.
    #[must_use]
    pub fn derivative(self, value: f64) -> f64 {
        match self {
            Scale::Lin => 1.0,
            Scale::Log => value.exp(),
            Scale::Log10 => 10f64.powf(value) * std::f64::consts::LN_10,
        }
    }

    /// Whether the scale needs a positive value.
    #[must_use]
    fn positive(self) -> bool {
        !matches!(self, Scale::Lin)
    }
}

/// A parameter of a fit: what it sets in the model, where it starts and where it may go.
#[derive(Clone, Debug)]
pub struct Parameter {
    /// The id of the entity the parameter sets, a selection of the model: the varying target
    /// of the prepared request of every condition.
    pub id: String,
    /// The value the fit starts from, on the linear scale.
    pub start: f64,
    /// The smallest value, on the linear scale.
    pub lower: f64,
    /// The largest value, on the linear scale.
    pub upper: f64,
    /// The scale an optimizer searches the parameter on.
    pub scale: Scale,
}

/// A curve of data of a fit: the measurements of one observable of one condition.
#[derive(Clone, Debug)]
pub struct Curve {
    /// The index of the condition in [`Definition::conditions`] whose simulation the curve
    /// is read from.
    pub condition: usize,
    /// The column of the simulation the curve is read from, a selection of its request.
    pub column: String,
    /// The times of the measurements, which have to be output times of the condition.
    pub times: Vec<f64>,
    /// The measurements, one per time.
    pub values: Vec<f64>,
    /// The standard deviation of every measurement, the residuals are divided by it; one
    /// for every point when it is absent.
    pub noise: Option<Vec<f64>>,
    /// The weight of the curve, which multiplies its residuals.
    pub weight: f64,
    /// Whether a measurement reads the state after a change rather than the one before it,
    /// one flag per time; every measurement reads the state before a change when it is
    /// absent. See [`crate::CurveRequest::after_change`].
    pub after_change: Option<Vec<bool>>,
}

/// A fit as a host hands it over: the parameters, the conditions and the data.
#[derive(Clone, Debug)]
pub struct Definition {
    /// The parameters of the fit, in the order of the vector of an optimizer.
    pub parameters: Vec<Parameter>,
    /// The request of every condition of the fit. Its output has to be the given times or a
    /// uniform grid, so that the row of a measurement does not depend on the parameters.
    pub conditions: Vec<SimulationRequest>,
    /// The curves of data of the fit.
    pub curves: Vec<Curve>,
    /// Where the jacobian of a step of the optimizer comes from.
    pub jacobian: JacobianChoice,
}

/// A curve resolved against the simulation of its condition.
#[derive(Debug)]
struct Resolved {
    /// Where the residuals of the curve start in the vector of the residuals.
    offset: usize,
    /// The column of the observable in the table of the condition.
    column: usize,
    /// The row of every measurement in that table.
    rows: Vec<usize>,
    /// The measurements.
    values: Vec<f64>,
    /// The weight of a point over the standard deviation of its measurement.
    scales: Vec<f64>,
}

/// One condition of the fit: what to simulate and which residuals its table carries.
#[derive(Debug)]
struct Point {
    /// The request of the condition, with the parameters of the fit as its varying targets.
    prepared: PreparedSimulation,
    /// The same condition with the forward sensitivities of those targets, which give the
    /// block of the jacobian of the condition in one simulation; `None` for a condition the
    /// engine does not carry them for.
    sensitive: Option<PreparedSimulation>,
    /// The number of columns of its table, for the index of a value.
    columns: usize,
    /// The curves which are read off its table.
    curves: Vec<Resolved>,
    /// The number of residuals of those curves.
    n_residuals: usize,
}

impl Point {
    /// The residuals of the curves of the condition, in the order of the curves.
    fn residuals(&self, table: &SimulationResult) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.n_residuals);
        for curve in &self.curves {
            for ((row, value), scale) in curve
                .rows
                .iter()
                .zip(curve.values.iter())
                .zip(curve.scales.iter())
            {
                out.push((table.values[row * self.columns + curve.column] - value) * scale);
            }
        }
        out
    }

    /// The block of the jacobian of the condition out of the sensitivities of its table: the
    /// derivative of every residual of its curves in every parameter of the search space,
    /// `n_residuals * k` row major in the order of its curves.
    ///
    /// `factors` is, per varying target of the condition, the parameter of the search space it
    /// takes its value from and the derivative of that value in it ([`Scale::derivative`]),
    /// `None` for a target which is held at a value.
    fn block(
        &self,
        table: &SimulationResult,
        factors: &[Option<(usize, f64)>],
        k: usize,
    ) -> Result<Vec<f64>, EngineError> {
        let Some(sensitivities) = &table.sensitivities else {
            return Err(EngineError::Internal(
                "the simulation of a condition of a fit answered no sensitivities".to_string(),
            ));
        };
        let targets = sensitivities.parameters.len();
        let mut out = vec![0.0; self.n_residuals * k];
        let mut at = 0;
        for curve in &self.curves {
            for (row, scale) in curve.rows.iter().zip(curve.scales.iter()) {
                let values = &sensitivities.values[(row * self.columns + curve.column) * targets..]
                    [..targets];
                for (value, factor) in values.iter().zip(factors) {
                    if let Some((parameter, factor)) = factor {
                        out[at * k + parameter] = value * scale * factor;
                    }
                }
                at += 1;
            }
        }
        Ok(out)
    }

    /// Writes the block of the jacobian of the condition into the one of the fit.
    fn write_block(&self, into: &mut [f64], block: &[f64], k: usize) {
        let mut at = 0;
        for curve in &self.curves {
            for row in 0..curve.rows.len() {
                let (from, to) = ((at + row) * k, (curve.offset + row) * k);
                into[to..to + k].copy_from_slice(&block[from..from + k]);
            }
            at += curve.rows.len();
        }
    }

    /// Writes the residuals of the condition into the vector of the fit.
    fn write(&self, into: &mut [f64], residuals: &[f64]) {
        let mut at = 0;
        for curve in &self.curves {
            let n = curve.rows.len();
            into[curve.offset..curve.offset + n].copy_from_slice(&residuals[at..at + n]);
            at += n;
        }
    }
}

/// One point of a batch: a condition and the values of the parameters to simulate it at.
struct Shot<'a> {
    point: &'a Point,
    values: &'a [f64],
}

/// The problem of a fit, resolved against a loaded model and the pool of its batch.
///
/// The conditions are prepared once (`prepare` of the host), so an evaluation transfers the
/// values of the parameters and nothing else, and the column and the row of every measurement
/// are resolved once as well: an evaluation is one fan out of the conditions and the
/// arithmetic of the residuals, see [`Problem::residuals`].
pub struct Problem {
    /// The parameters an optimizer searches, which are the ones of the fit without the ones
    /// a profile holds at a value ([`Problem::fixing`]).
    free: Vec<Parameter>,
    /// One slot per varying target of the prepared requests, in their order: the parameter of
    /// the search space whose value it takes, or the value it is held at.
    slots: Vec<Slot>,
    /// The conditions of the fit and the curves they carry, shared with the problems which
    /// fix a parameter of it.
    points: Arc<Vec<Point>>,
    n_residuals: usize,
    /// Where the jacobian of a step comes from: [`JacobianChoice::Sensitivities`] or
    /// [`JacobianChoice::DifferenceQuotient`], resolved once for the whole problem.
    jacobian: JacobianChoice,
    /// The simulations the problem ran, shared with the problems which fix a parameter of it
    /// so that a profile counts the simulations of its fits as well.
    simulations: Arc<AtomicU64>,
    /// The pool of the batch, shared with those problems as well: the instance of a thread is
    /// made once, whatever is fitted on it.
    workers: Arc<Workers>,
}

/// Where a varying target of the prepared requests takes its value from.
#[derive(Clone, Copy, Debug)]
enum Slot {
    /// The parameter at this index of the search space, on its scale.
    Free { at: usize, scale: Scale },
    /// A value the parameter is held at, on the linear scale.
    Fixed(f64),
}

impl std::fmt::Debug for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Problem")
            .field("parameters", &self.free.len())
            .field("fixed", &(self.slots.len() - self.free.len()))
            .field("conditions", &self.points.len())
            .field(
                "curves",
                &self.points.iter().map(|p| p.curves.len()).sum::<usize>(),
            )
            .field("residuals", &self.n_residuals)
            .field("threads", &self.workers.threads())
            .finish()
    }
}

impl Problem {
    /// Resolves a definition against a model and the pool of its batch.
    ///
    /// Every condition is prepared with the parameters of the fit as its varying targets and
    /// simulated once at the start, which gives the columns and the output times its curves
    /// are resolved against.
    ///
    /// # Errors
    ///
    /// If the definition is empty, if a parameter cannot be fitted (its bounds, its scale or
    /// its start), if a condition cannot be prepared or simulated at the start, or if a curve
    /// does not fit the table of its condition (its column, its times or its noise).
    pub fn new(
        definition: Definition,
        model: Arc<LoadedModel>,
        threads: usize,
    ) -> Result<Problem, FitError> {
        let Definition {
            parameters,
            conditions,
            curves,
            jacobian,
        } = definition;
        check_parameters(&parameters)?;
        if conditions.is_empty() {
            return Err(FitError::Empty("condition"));
        }
        if curves.is_empty() {
            return Err(FitError::Empty("curve"));
        }
        let varying: Vec<String> = parameters.iter().map(|p| p.id.clone()).collect();
        let workers = Workers::new(model, threads)?;
        let mut prepared = Vec::with_capacity(conditions.len());
        for (at, request) in conditions.iter().enumerate() {
            if matches!(request.output, Output::IntegratorSteps) {
                return Err(FitError::Condition {
                    at,
                    message: "outputs the steps of the integrator, whose times depend on the \
                              parameters: a fit needs the times of its data or a uniform grid"
                        .to_string(),
                });
            }
            prepared.push(workers.model().prepare(request, &varying).map_err(|err| {
                FitError::Condition {
                    at,
                    message: format!("cannot be prepared: {err}"),
                }
            })?);
        }
        let (sensitive, jacobian) = if jacobian == JacobianChoice::Sensitivities {
            let prepared = sensitive_conditions(&workers, &conditions, &varying)?;
            (prepared, JacobianChoice::Sensitivities)
        } else {
            (
                conditions.iter().map(|_| None).collect(),
                JacobianChoice::DifferenceQuotient,
            )
        };
        // the table of every condition at the start, which the curves are resolved against
        let start: Vec<f64> = parameters
            .iter()
            .map(|p| p.scale.to_model(p.scale.to_search(p.start)))
            .collect();
        let tables = simulate(&workers, &prepared, &start)?;
        let mut points: Vec<Point> = prepared
            .into_iter()
            .zip(sensitive)
            .zip(tables.iter())
            .map(|((prepared, sensitive), table)| Point {
                prepared,
                sensitive,
                columns: table.columns.len(),
                curves: Vec::new(),
                n_residuals: 0,
            })
            .collect();
        let mut n_residuals = 0;
        for (at, curve) in curves.iter().enumerate() {
            let point = points
                .get_mut(curve.condition)
                .ok_or_else(|| FitError::Curve {
                    at,
                    message: format!(
                        "reads the condition {}, which the fit has not",
                        curve.condition
                    ),
                })?;
            let resolved = resolve(at, curve, &tables[curve.condition], n_residuals)?;
            n_residuals += resolved.rows.len();
            point.n_residuals += resolved.rows.len();
            point.curves.push(resolved);
        }
        Ok(Problem {
            slots: (0..parameters.len())
                .map(|at| Slot::Free {
                    at,
                    scale: parameters[at].scale,
                })
                .collect(),
            free: parameters,
            points: Arc::new(points),
            n_residuals,
            jacobian,
            simulations: Arc::new(AtomicU64::new(0)),
            workers: Arc::new(workers),
        })
    }

    /// The same fit with parameters held at a value, which is the problem of a profile.
    ///
    /// The indices are the ones of [`Problem::parameters`] of this problem, and the problem
    /// it gives has the parameters around them: an optimizer searches those and the model
    /// sees the values the others are held at. The conditions, the curves and the pool of the
    /// batch are shared, so a profile of hundreds of points prepares and instantiates
    /// nothing again.
    ///
    /// # Errors
    ///
    /// If an index is none of the parameters of the fit, if the fit would have no parameter
    /// left, or if a value is outside the bounds of its parameter.
    pub fn fixing(&self, fixed: &[(usize, f64)]) -> Result<Problem, FitError> {
        let mut held: Vec<Option<f64>> = vec![None; self.free.len()];
        for (at, value) in fixed {
            let parameter = self.free.get(*at).ok_or_else(|| FitError::Parameter {
                id: format!("{at}"),
                message: format!("is none of the {} parameters of the fit", self.free.len()),
            })?;
            if !value.is_finite() || *value < parameter.lower || *value > parameter.upper {
                return Err(FitError::Parameter {
                    id: parameter.id.clone(),
                    message: format!(
                        "is held at {value}, outside its bounds [{}, {}]",
                        parameter.lower, parameter.upper
                    ),
                });
            }
            held[*at] = Some(*value);
        }
        let free: Vec<Parameter> = self
            .free
            .iter()
            .zip(held.iter())
            .filter(|(_, held)| held.is_none())
            .map(|(parameter, _)| parameter.clone())
            .collect();
        if free.is_empty() {
            return Err(FitError::Empty("parameter which is not held at a value"));
        }
        // the index of every parameter which is still searched, in the order of the fit
        let mut at_of: Vec<usize> = Vec::with_capacity(self.free.len());
        let mut next = 0;
        for value in &held {
            at_of.push(next);
            if value.is_none() {
                next += 1;
            }
        }
        let slots = self
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Fixed(value) => Slot::Fixed(*value),
                Slot::Free { at, scale } => match held[*at] {
                    Some(value) => Slot::Fixed(value),
                    None => Slot::Free {
                        at: at_of[*at],
                        scale: *scale,
                    },
                },
            })
            .collect();
        Ok(Problem {
            free,
            slots,
            points: self.points.clone(),
            n_residuals: self.n_residuals,
            jacobian: self.jacobian,
            simulations: self.simulations.clone(),
            workers: self.workers.clone(),
        })
    }

    /// The parameters of the fit, in the order of the vector of an optimizer.
    #[must_use]
    pub fn parameters(&self) -> &[Parameter] {
        &self.free
    }

    /// The number of residuals of one evaluation.
    #[must_use]
    pub fn n_residuals(&self) -> usize {
        self.n_residuals
    }

    /// The threads of the batch of an evaluation.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.workers.threads()
    }

    /// The start of the fit, in the space an optimizer searches.
    #[must_use]
    pub fn start(&self) -> Vec<f64> {
        self.free
            .iter()
            .map(|p| p.scale.to_search(p.start))
            .collect()
    }

    /// The bounds of the parameters, in the space an optimizer searches.
    #[must_use]
    pub fn bounds(&self) -> (Vec<f64>, Vec<f64>) {
        (
            self.free
                .iter()
                .map(|p| p.scale.to_search(p.lower))
                .collect(),
            self.free
                .iter()
                .map(|p| p.scale.to_search(p.upper))
                .collect(),
        )
    }

    /// The values of the parameters an optimizer searches at a point of the search space,
    /// on the linear scale: the ones of [`Problem::parameters`], in their order.
    #[must_use]
    pub fn parameter_values(&self, x: &[f64]) -> Vec<f64> {
        self.free
            .iter()
            .zip(x.iter())
            .map(|(parameter, value)| parameter.scale.to_model(*value))
            .collect()
    }

    /// The values of the varying targets of the conditions at a point of the search space:
    /// the parameters of the fit on their scale and the ones a profile holds at a value.
    #[must_use]
    pub fn values(&self, x: &[f64]) -> Vec<f64> {
        self.slots
            .iter()
            .map(|slot| match slot {
                Slot::Free { at, scale } => scale.to_model(x[*at]),
                Slot::Fixed(value) => *value,
            })
            .collect()
    }

    /// The residuals of a point of the search space, one per measurement.
    ///
    /// The conditions are one batch of the engine: every point simulates on the instance of
    /// its thread and reduces its table to the residuals of its curves there, so the tables
    /// never leave the thread which made them. The residuals of a condition whose integration
    /// failed are [`FAILED_RESIDUAL`].
    ///
    /// # Errors
    ///
    /// If the engine refuses a request or fails in itself, or if a point of the batch failed
    /// outside the engine; a failed integration is no error.
    pub fn residuals(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        let values = self.values(x);
        let shots: Vec<Shot<'_>> = self
            .points
            .iter()
            .map(|point| Shot {
                point,
                values: &values,
            })
            .collect();
        let points = self.run(&shots)?;
        let mut residuals = vec![0.0; self.n_residuals];
        for (shot, point) in shots.iter().zip(points.iter()) {
            shot.point.write(&mut residuals, point);
        }
        Ok(residuals)
    }

    /// The cost of a point of the search space, half the sum of the squares of its residuals.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    pub fn cost(&self, x: &[f64]) -> Result<f64, FitError> {
        Ok(0.5
            * self
                .residuals(x)?
                .iter()
                .map(|value| value * value)
                .sum::<f64>())
    }

    /// Runs an optimizer over the problem, from the start of its parameters.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`], or if the optimizer refuses the bounds of the fit.
    pub fn optimize(&self, settings: &crate::Settings) -> Result<crate::Fit, FitError> {
        crate::optimize::run(self, settings, &self.start(), &crate::Stop::new())
    }

    /// Runs an optimizer over the problem, from a point of the search space.
    ///
    /// The start of a fit of many starts, the optimum of a fit a profile steps away from and
    /// the best member of a population are such points.
    ///
    /// # Errors
    ///
    /// As [`Problem::optimize`], or if the point is not one of the parameters of the fit.
    pub fn optimize_from(
        &self,
        settings: &crate::Settings,
        from: &[f64],
    ) -> Result<crate::Fit, FitError> {
        crate::optimize::run(self, settings, from, &crate::Stop::new())
    }

    /// Runs an optimizer over the problem, from a point of the search space, until a caller
    /// stops it.
    ///
    /// # Errors
    ///
    /// As [`Problem::optimize_from`].
    pub fn optimize_with(
        &self,
        settings: &crate::Settings,
        from: &[f64],
        stop: &crate::Stop,
    ) -> Result<crate::Fit, FitError> {
        crate::optimize::run(self, settings, from, stop)
    }

    /// Runs `work` on the threads of the batch of the problem.
    pub(crate) fn install<R: Send>(&self, work: impl FnOnce() -> R + Send) -> R {
        self.workers.install(work)
    }

    /// The step of the finite difference of the parameter `k` at a point of the search space.
    ///
    /// A relative step of [`DIFF_STEP`], as the `diff_step` of sbmlsim, which points away
    /// from the bound it would cross.
    #[must_use]
    pub fn step(&self, k: usize, x: &[f64]) -> f64 {
        let parameter = &self.free[k];
        let step = DIFF_STEP * x[k].abs().max(1.0);
        let upper = parameter.scale.to_search(parameter.upper);
        if x[k] + step > upper { -step } else { step }
    }

    /// Where the jacobian of the problem comes from, resolved: the sensitivities of its
    /// conditions or the finite differences of its residuals.
    #[must_use]
    pub fn jacobian_of(&self) -> JacobianChoice {
        self.jacobian
    }

    /// The simulations the problem ran, and the ones of the problems which hold a parameter
    /// of it ([`Problem::fixing`]): what a fit costs, which the rate of an evaluation does
    /// not say once the jacobian of a step is one simulation per condition.
    #[must_use]
    pub fn simulations(&self) -> u64 {
        self.simulations.load(Ordering::Relaxed)
    }

    /// The jacobian of the residuals at a point of the search space, row major
    /// (`n_residuals * parameters`), of [`Problem::jacobian_of`].
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    pub fn jacobian(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        match self.jacobian {
            JacobianChoice::DifferenceQuotient => self.jacobian_of_differences(x),
            _ => self.jacobian_of_sensitivities(x),
        }
    }

    /// The jacobian out of the forward sensitivities of the conditions: one simulation per
    /// condition, whatever the number of the parameters, and the derivative of a residual
    /// instead of a difference of two of them.
    ///
    /// The sensitivities of a condition are the derivatives of its columns in the values of
    /// its varying targets, which the scale of a parameter turns into the derivative in the
    /// point of the search space ([`Scale::derivative`]). The block of a condition is reduced
    /// on the thread which simulated it, as its residuals are, so the sensitivities of a
    /// table never leave it either.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    fn jacobian_of_sensitivities(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        let k = self.free.len();
        let values = self.values(x);
        let factors: Vec<Option<(usize, f64)>> = self
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Free { at, scale } => Some((*at, scale.derivative(x[*at]))),
                Slot::Fixed(_) => None,
            })
            .collect();
        let shots: Vec<Shot<'_>> = self
            .points
            .iter()
            .map(|point| Shot {
                point,
                values: &values,
            })
            .collect();
        self.simulations
            .fetch_add(shots.len() as u64, Ordering::Relaxed);
        let outcomes = self.workers.map(&shots, |instance, shot| {
            let Some(prepared) = &shot.point.sensitive else {
                return Err(EngineError::Internal(
                    "a condition of the fit carries no sensitivities".to_string(),
                ));
            };
            let table = instance.simulate_prepared(prepared, shot.values)?;
            shot.point.block(&table, &factors, k)
        });
        let mut jacobian = vec![0.0; self.n_residuals * k];
        for (shot, outcome) in shots.iter().zip(outcomes) {
            match outcome {
                Ok(block) => shot.point.write_block(&mut jacobian, &block, k),
                // a failed integration is a point of the search space the fit may not take:
                // its residuals are `FAILED_RESIDUAL` and its block of the jacobian says
                // nothing, so the optimizer rejects the step by its cost
                Err(PointFailure::Engine(EngineError::Integration { .. })) => {}
                Err(PointFailure::Engine(err)) => return Err(FitError::Engine(err)),
                Err(PointFailure::Internal(message)) => return Err(FitError::Point(message)),
            }
        }
        Ok(jacobian)
    }

    /// The jacobian by the finite differences of the residuals: the column of a parameter is
    /// the forward difference of the residuals over [`Problem::step`]. The whole jacobian is
    /// one batch of the engine, `(parameters + 1) * conditions` points which do not depend on
    /// each other, so it fans out over the threads at once instead of once per column.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    fn jacobian_of_differences(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        let k = self.free.len();
        let steps: Vec<f64> = (0..k).map(|column| self.step(column, x)).collect();
        // the values of the base point and of every column of the difference
        let mut moved: Vec<Vec<f64>> = Vec::with_capacity(k + 1);
        moved.push(self.values(x));
        for (column, step) in steps.iter().enumerate() {
            let mut ahead = x.to_vec();
            ahead[column] += step;
            moved.push(self.values(&ahead));
        }
        let mut shots: Vec<Shot<'_>> = Vec::with_capacity((k + 1) * self.points.len());
        for values in &moved {
            for point in self.points.iter() {
                shots.push(Shot { point, values });
            }
        }
        let g = self.points.len();
        let n = self.n_residuals;
        let points = self.run(&shots)?;
        // the residuals of the base point and of every column of the difference, one after
        // the other: the points of a column are the conditions of one evaluation
        let mut residuals = vec![0.0; (k + 1) * n];
        for (at, (shot, point)) in shots.iter().zip(points.iter()).enumerate() {
            let column = at / g;
            shot.point
                .write(&mut residuals[column * n..(column + 1) * n], point);
        }
        let mut jacobian = vec![0.0; self.n_residuals * k];
        for row in 0..self.n_residuals {
            let at = residuals[row];
            for (column, step) in steps.iter().enumerate() {
                let ahead = residuals[(column + 1) * self.n_residuals + row];
                jacobian[row * k + column] = (ahead - at) / step;
            }
        }
        Ok(jacobian)
    }

    /// Runs the points of a batch and reduces every one of them on the thread of the point.
    ///
    /// The residuals of a point whose integration failed are [`FAILED_RESIDUAL`]; any other
    /// failure of the engine is an error of the fit.
    fn run(&self, shots: &[Shot<'_>]) -> Result<Vec<Vec<f64>>, FitError> {
        self.simulations
            .fetch_add(shots.len() as u64, Ordering::Relaxed);
        let outcomes = self.workers.map(shots, |instance, shot| {
            let table = instance.simulate_prepared(&shot.point.prepared, shot.values)?;
            Ok(shot.point.residuals(&table))
        });
        let mut residuals = Vec::with_capacity(shots.len());
        for (shot, outcome) in shots.iter().zip(outcomes) {
            residuals.push(match outcome {
                Ok(residuals) => residuals,
                // a failed integration is a point of the search space the fit may not take
                Err(PointFailure::Engine(EngineError::Integration { .. })) => {
                    vec![FAILED_RESIDUAL; shot.point.n_residuals]
                }
                Err(PointFailure::Engine(err)) => return Err(FitError::Engine(err)),
                Err(PointFailure::Internal(message)) => return Err(FitError::Point(message)),
            });
        }
        Ok(residuals)
    }
}

/// Every condition of a fit prepared with its forward sensitivities.
///
/// A condition which carries none is refused: the jacobian of a fit is the one of the
/// sensitivities or the one of the differences, and never a mixture of the two, which would
/// be a jacobian of two accuracies.
fn sensitive_conditions(
    workers: &Workers,
    conditions: &[SimulationRequest],
    varying: &[String],
) -> Result<Vec<Option<PreparedSimulation>>, FitError> {
    let mut instance = workers
        .model()
        .instantiate_native()
        .map_err(|error| FitError::Request(format!("cannot be instantiated: {error}")))?;
    let mut prepared = Vec::with_capacity(conditions.len());
    for (at, request) in conditions.iter().enumerate() {
        let mut request = request.clone();
        request.sensitivities = varying.to_vec();
        match instance.prepare(&request, varying) {
            Ok(sensitive) => prepared.push(Some(sensitive)),
            Err(EngineError::Unsupported(message)) => {
                return Err(FitError::Condition {
                    at,
                    message: format!("carries no forward sensitivities: {message}"),
                });
            }
            Err(error) => {
                return Err(FitError::Condition {
                    at,
                    message: format!("cannot be prepared with its sensitivities: {error}"),
                });
            }
        }
    }
    Ok(prepared)
}

/// Simulates every prepared condition once at the values of the parameters.
fn simulate(
    workers: &Workers,
    prepared: &[PreparedSimulation],
    values: &[f64],
) -> Result<Vec<SimulationResult>, FitError> {
    let mut tables = Vec::with_capacity(prepared.len());
    let outcomes = workers.map(prepared, |instance, prepared| {
        instance.simulate_prepared(prepared, values)
    });
    for (at, outcome) in outcomes.into_iter().enumerate() {
        match outcome {
            Ok(table) => tables.push(table),
            Err(PointFailure::Engine(err)) => {
                return Err(FitError::Condition {
                    at,
                    message: format!("cannot be simulated at the start of the fit: {err}"),
                });
            }
            Err(PointFailure::Internal(message)) => return Err(FitError::Point(message)),
        }
    }
    Ok(tables)
}

/// Checks the parameters of a fit, which an optimizer searches between their bounds.
fn check_parameters(parameters: &[Parameter]) -> Result<(), FitError> {
    if parameters.is_empty() {
        return Err(FitError::Empty("parameter"));
    }
    for parameter in parameters {
        let fault = if !parameter.lower.is_finite() || !parameter.upper.is_finite() {
            Some("has a bound which is not finite".to_string())
        } else if parameter.lower >= parameter.upper {
            Some(format!(
                "has the bounds [{}, {}], which are not increasing",
                parameter.lower, parameter.upper
            ))
        } else if !parameter.start.is_finite()
            || parameter.start < parameter.lower
            || parameter.start > parameter.upper
        {
            Some(format!(
                "starts at {}, outside its bounds [{}, {}]",
                parameter.start, parameter.lower, parameter.upper
            ))
        } else if parameter.scale.positive() && parameter.lower <= 0.0 {
            Some(format!(
                "is searched on a logarithmic scale and has the lower bound {}, which is not \
                 positive",
                parameter.lower
            ))
        } else {
            None
        };
        if let Some(message) = fault {
            return Err(FitError::Parameter {
                id: parameter.id.clone(),
                message,
            });
        }
    }
    Ok(())
}

/// Resolves a curve against the table of its condition: its column and the row of a point.
fn resolve(
    at: usize,
    curve: &Curve,
    table: &SimulationResult,
    offset: usize,
) -> Result<Resolved, FitError> {
    let fault = |message: String| FitError::Curve { at, message };
    if curve.times.len() != curve.values.len() {
        return Err(fault(format!(
            "has {} times and {} values",
            curve.times.len(),
            curve.values.len()
        )));
    }
    if curve.times.is_empty() {
        return Err(fault("has no measurement".to_string()));
    }
    if !curve.weight.is_finite() || curve.weight <= 0.0 {
        return Err(fault(format!(
            "has the weight {}, which is not positive and finite",
            curve.weight
        )));
    }
    let column = table
        .columns
        .iter()
        .position(|name| *name == curve.column)
        .ok_or_else(|| {
            fault(format!(
                "reads the column '{}', which its condition does not select: {:?}",
                curve.column, table.columns
            ))
        })?;
    let noise = match &curve.noise {
        None => vec![1.0; curve.values.len()],
        Some(noise) if noise.len() == curve.values.len() => noise.clone(),
        Some(noise) => {
            return Err(fault(format!(
                "has {} values and {} standard deviations",
                curve.values.len(),
                noise.len()
            )));
        }
    };
    let mut scales = Vec::with_capacity(noise.len());
    for value in &noise {
        if !value.is_finite() || *value <= 0.0 {
            return Err(fault(format!(
                "has the standard deviation {value}, which is not positive and finite"
            )));
        }
        scales.push(curve.weight / value);
    }
    let after = match &curve.after_change {
        None => vec![false; curve.times.len()],
        Some(after) if after.len() == curve.times.len() => after.clone(),
        Some(after) => {
            return Err(fault(format!(
                "has {} times and {} flags of the state after a change",
                curve.times.len(),
                after.len()
            )));
        }
    };
    let times: Vec<f64> = (0..table.n_rows)
        .map(|row| table.values[row * table.columns.len()])
        .collect();
    let mut rows = Vec::with_capacity(curve.times.len());
    for (time, after) in curve.times.iter().zip(after.iter()) {
        if !curve.values.iter().all(|value| value.is_finite()) {
            return Err(fault("has a measurement which is not finite".to_string()));
        }
        rows.push(row_of(*time, &times, *after).ok_or_else(|| {
            fault(format!(
                "measures at t = {time}, which is no output time of its condition"
            ))
        })?);
    }
    Ok(Resolved {
        offset,
        column,
        rows,
        values: curve.values.clone(),
        scales,
    })
}

/// The row of an output time, the nearest one within the resolution of the time.
///
/// The output of a condition has two rows of one time at a change after its start, the state
/// before the change and the state after it, and a chain of a change and the events it fired
/// has more: `after` reads the last row of the nearest time, the state after the whole chain,
/// where the default reads the first of them, the state before the change.
fn row_of(time: f64, times: &[f64], after: bool) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (row, output) in times.iter().enumerate() {
        let distance = (output - time).abs();
        let closer = best.is_none_or(|(_, other)| {
            if after {
                distance <= other
            } else {
                distance < other
            }
        });
        if closer {
            best = Some((row, distance));
        }
    }
    let (row, distance) = best?;
    // the time of a row is the one the integrator reached, so a measurement matches the row
    // of its own time to the resolution of that time and not exactly
    let tolerance = 1e-9 * time.abs().max(1.0);
    (distance <= tolerance).then_some(row)
}
