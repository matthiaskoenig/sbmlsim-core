//! The problem of a fit: its definition, its residuals and its jacobian on the batch.

use std::sync::Arc;

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
    parameters: Vec<Parameter>,
    points: Vec<Point>,
    n_residuals: usize,
    workers: Workers,
}

impl std::fmt::Debug for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Problem")
            .field("parameters", &self.parameters.len())
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
        // the table of every condition at the start, which the curves are resolved against
        let start: Vec<f64> = parameters
            .iter()
            .map(|p| p.scale.to_model(p.scale.to_search(p.start)))
            .collect();
        let tables = simulate(&workers, &prepared, &start)?;
        let mut points: Vec<Point> = prepared
            .into_iter()
            .zip(tables.iter())
            .map(|(prepared, table)| Point {
                prepared,
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
            parameters,
            points,
            n_residuals,
            workers,
        })
    }

    /// The parameters of the fit, in the order of the vector of an optimizer.
    #[must_use]
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
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
        self.parameters
            .iter()
            .map(|p| p.scale.to_search(p.start))
            .collect()
    }

    /// The bounds of the parameters, in the space an optimizer searches.
    #[must_use]
    pub fn bounds(&self) -> (Vec<f64>, Vec<f64>) {
        (
            self.parameters
                .iter()
                .map(|p| p.scale.to_search(p.lower))
                .collect(),
            self.parameters
                .iter()
                .map(|p| p.scale.to_search(p.upper))
                .collect(),
        )
    }

    /// The values of the parameters of a point of the search space.
    #[must_use]
    pub fn values(&self, x: &[f64]) -> Vec<f64> {
        self.parameters
            .iter()
            .zip(x.iter())
            .map(|(p, value)| p.scale.to_model(*value))
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
        let parameter = &self.parameters[k];
        let step = DIFF_STEP * x[k].abs().max(1.0);
        let upper = parameter.scale.to_search(parameter.upper);
        if x[k] + step > upper { -step } else { step }
    }

    /// The jacobian of the residuals at a point of the search space, row major.
    ///
    /// `n_residuals * parameters` values: the column of a parameter is the forward difference
    /// of the residuals over [`Problem::step`]. The whole jacobian is one batch of the
    /// engine, `(parameters + 1) * conditions` points which do not depend on each other, so
    /// it fans out over the threads at once instead of once per column.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    pub fn jacobian(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        let k = self.parameters.len();
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
            for point in &self.points {
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
    let times: Vec<f64> = (0..table.n_rows)
        .map(|row| table.values[row * table.columns.len()])
        .collect();
    let mut rows = Vec::with_capacity(curve.times.len());
    for time in &curve.times {
        if !curve.values.iter().all(|value| value.is_finite()) {
            return Err(fault("has a measurement which is not finite".to_string()));
        }
        rows.push(row_of(*time, &times).ok_or_else(|| {
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
fn row_of(time: f64, times: &[f64]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (row, output) in times.iter().enumerate() {
        let distance = (output - time).abs();
        if best.is_none_or(|(_, other)| distance < other) {
            best = Some((row, distance));
        }
    }
    let (row, distance) = best?;
    // the time of a row is the one the integrator reached, so a measurement matches the row
    // of its own time to the resolution of that time and not exactly
    let tolerance = 1e-9 * time.abs().max(1.0);
    (distance <= tolerance).then_some(row)
}
