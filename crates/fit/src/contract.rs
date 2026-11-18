//! The contract `fit/1`: a fit as a file, which a host writes and the answer it reads back.
//!
//! A host of the engine (sbmlsim, the command line, JavaScript) resolves its own problem into
//! this schema: the parameters with their bounds and their scale, a request of `simulation/1`
//! per condition, the data of every curve and the optimizer to run. The semantics a fit of
//! the engine and a fit of its host have to agree on lives in the schema, so that a fit is a
//! file one can replay, as `model/2` and `simulation/1` are for a model and a simulation.

use std::sync::Arc;

use sbmlsim_abi::SimulationRequest;
use sbmlsim_host::LoadedModel;
use serde::{Deserialize, Serialize};

use crate::{Curve, Definition, Fit, FitError, Parameter, Problem, Scale, Settings, Stop};

/// The contract of a fit: the version of the schema of a request and of its answer.
pub const FIT_CONTRACT: &str = "fit/1";

/// A start of a fit: the values of its parameters as a caller gave them and the point of the
/// search space they are.
struct Start {
    values: Vec<f64>,
    point: Vec<f64>,
}

/// The weight of a curve which does not give one.
fn one() -> f64 {
    1.0
}

/// A fit as a host hands it over.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FitRequest {
    /// The contract, [`FIT_CONTRACT`].
    pub contract: String,
    /// The parameters of the fit, in the order of the vector of an optimizer.
    pub parameters: Vec<ParameterRequest>,
    /// The conditions of the fit, every one of them a request of `simulation/1`.
    pub conditions: Vec<ConditionRequest>,
    /// The curves of data of the fit.
    pub curves: Vec<CurveRequest>,
    /// The optimizer of the fit and what it may spend; the defaults of [`Settings`] when it
    /// is absent.
    #[serde(default)]
    pub optimizer: Settings,
    /// The points the fit starts from, the values of its parameters on the linear scale, one
    /// run per point and the best of their runs the answer; the start of the parameters
    /// themselves when it is empty.
    #[serde(default)]
    pub starts: Vec<Vec<f64>>,
    /// The threads of the batch of an evaluation; the CPUs of the machine when it is absent,
    /// and what the caller gives takes precedence.
    #[serde(default)]
    pub threads: Option<usize>,
}

/// A parameter of a fit.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParameterRequest {
    /// The id of the entity the parameter sets, a selection of the model.
    pub id: String,
    /// The value the fit starts from, on the linear scale.
    pub start: f64,
    /// The smallest value.
    pub lower: f64,
    /// The largest value.
    pub upper: f64,
    /// The scale an optimizer searches the parameter on; the value itself when it is absent.
    #[serde(default)]
    pub scale: Scale,
}

/// A condition of a fit: what to simulate, under an id its curves read.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConditionRequest {
    /// The id of the condition, which the curves of the fit read it by.
    pub id: String,
    /// The simulation of the condition, a request of `simulation/1` whose output is the times
    /// of the data of its curves.
    pub request: SimulationRequest,
}

/// A curve of data of a fit.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CurveRequest {
    /// The id of the curve, which the answer names it by.
    #[serde(default)]
    pub id: Option<String>,
    /// The id of the condition the curve is read from.
    pub condition: String,
    /// The column of the simulation of that condition the curve is read from.
    pub column: String,
    /// The times of the measurements, which have to be output times of the condition.
    pub times: Vec<f64>,
    /// The measurements, one per time.
    pub values: Vec<f64>,
    /// The standard deviation of every measurement, which its residual is divided by.
    #[serde(default)]
    pub noise: Option<Vec<f64>>,
    /// The weight of the curve, which multiplies its residuals; one when it is absent.
    #[serde(default = "one")]
    pub weight: f64,
}

/// What a fit found.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FitAnswer {
    /// The contract, [`FIT_CONTRACT`].
    pub contract: String,
    /// Why the optimizer of the best run stopped.
    pub status: String,
    /// Half the sum of the squares of the residuals of the optimum.
    pub cost: f64,
    /// The parameters of the optimum.
    pub parameters: Vec<FittedParameter>,
    /// The iterations the optimizer of the best run took.
    pub iterations: u64,
    /// The evaluations of the objective it took.
    pub evaluations: u64,
    /// The jacobians it took.
    pub jacobians: u64,
    /// The time every run of the fit took together.
    pub seconds: f64,
    /// The residuals of the optimum, in the order of the curves of the request.
    pub residuals: Vec<f64>,
    /// Every run of the fit, in the order of its starts.
    pub runs: Vec<RunAnswer>,
}

/// A parameter of the optimum of a fit.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FittedParameter {
    /// The id of the parameter.
    pub id: String,
    /// Its value, on the linear scale, which the model sees.
    pub value: f64,
    /// Its value in the space the optimizer searched.
    pub x: f64,
}

/// One run of a fit, from one of its starts.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunAnswer {
    /// The point the run started from, the values of the parameters.
    pub start: Vec<f64>,
    /// The values of the parameters it ended at.
    pub values: Vec<f64>,
    /// Half the sum of the squares of its residuals there.
    pub cost: f64,
    /// Why its optimizer stopped.
    pub status: String,
    /// The iterations it took.
    pub iterations: u64,
    /// The evaluations of the objective it took.
    pub evaluations: u64,
    /// The jacobians it took.
    pub jacobians: u64,
    /// The time it took.
    pub seconds: f64,
}

/// Runs a fit of the contract `fit/1` against a model and answers what it found.
///
/// The threads of the batch of an evaluation are the ones given, else the ones of the
/// request, else the CPUs of the machine.
///
/// # Errors
///
/// If the request is of another contract or does not resolve (a curve of a condition the fit
/// has not, a start which is not its parameters), if the problem is refused
/// ([`Problem::new`]) or if the engine fails ([`Problem::residuals`]).
pub fn fit(
    request: &FitRequest,
    model: Arc<LoadedModel>,
    threads: Option<usize>,
) -> Result<FitAnswer, FitError> {
    fit_with(request, model, threads, &Stop::new())
}

/// Runs a fit of the contract `fit/1` until a caller stops it, see [`fit`] and [`Stop`].
///
/// # Errors
///
/// As [`fit`].
pub fn fit_with(
    request: &FitRequest,
    model: Arc<LoadedModel>,
    threads: Option<usize>,
    stop: &Stop,
) -> Result<FitAnswer, FitError> {
    if request.contract != FIT_CONTRACT {
        return Err(FitError::Request(format!(
            "is of the contract {} and not of {FIT_CONTRACT}",
            request.contract
        )));
    }
    let definition = definition_of(request)?;
    let threads = threads
        .or(request.threads)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, Into::into));
    let problem = Problem::new(definition, model, threads)?;
    let starts = starts_of(request, &problem)?;
    let began = std::time::Instant::now();
    let mut runs: Vec<RunAnswer> = Vec::with_capacity(starts.len());
    let mut best: Option<Fit> = None;
    for start in &starts {
        let fit = problem.optimize_with(&request.optimizer, &start.point, stop)?;
        runs.push(RunAnswer {
            start: start.values.clone(),
            values: fit.values.clone(),
            cost: fit.cost,
            status: fit.status.clone(),
            iterations: fit.iterations,
            evaluations: fit.evaluations,
            jacobians: fit.jacobians,
            seconds: fit.seconds,
        });
        if best.as_ref().is_none_or(|best| fit.cost < best.cost) {
            best = Some(fit);
        }
        if stop.stopped() {
            break;
        }
    }
    let best = best.ok_or_else(|| FitError::Request("has no start to run".to_string()))?;
    Ok(FitAnswer {
        contract: FIT_CONTRACT.to_string(),
        status: best.status,
        cost: best.cost,
        parameters: request
            .parameters
            .iter()
            .zip(best.values.iter())
            .zip(best.x.iter())
            .map(|((parameter, value), x)| FittedParameter {
                id: parameter.id.clone(),
                value: *value,
                x: *x,
            })
            .collect(),
        iterations: best.iterations,
        evaluations: best.evaluations,
        jacobians: best.jacobians,
        seconds: began.elapsed().as_secs_f64(),
        residuals: best.residuals,
        runs,
    })
}

/// Resolves a request into the definition of a fit: the ids of its conditions into their
/// index, and the defaults of its curves.
fn definition_of(request: &FitRequest) -> Result<Definition, FitError> {
    let mut at_of: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::with_capacity(request.conditions.len());
    for (at, condition) in request.conditions.iter().enumerate() {
        if at_of.insert(condition.id.as_str(), at).is_some() {
            return Err(FitError::Request(format!(
                "has two conditions of the id '{}'",
                condition.id
            )));
        }
    }
    let mut curves = Vec::with_capacity(request.curves.len());
    for (at, curve) in request.curves.iter().enumerate() {
        let condition = *at_of.get(curve.condition.as_str()).ok_or_else(|| {
            FitError::Request(format!(
                "has the curve {at}{}, which reads the condition '{}' it has not",
                curve
                    .id
                    .as_ref()
                    .map_or(String::new(), |id| format!(" ('{id}')")),
                curve.condition
            ))
        })?;
        curves.push(Curve {
            condition,
            column: curve.column.clone(),
            times: curve.times.clone(),
            values: curve.values.clone(),
            noise: curve.noise.clone(),
            weight: curve.weight,
        });
    }
    Ok(Definition {
        parameters: request
            .parameters
            .iter()
            .map(|parameter| Parameter {
                id: parameter.id.clone(),
                start: parameter.start,
                lower: parameter.lower,
                upper: parameter.upper,
                scale: parameter.scale,
            })
            .collect(),
        conditions: request
            .conditions
            .iter()
            .map(|condition| condition.request.clone())
            .collect(),
        curves,
    })
}

/// The starts of a fit, the values of its parameters as given and the point of the search
/// space of every one of them; the start of the parameters themselves when the request gives
/// none.
fn starts_of(request: &FitRequest, problem: &Problem) -> Result<Vec<Start>, FitError> {
    if request.starts.is_empty() {
        return Ok(vec![Start {
            values: problem
                .parameters()
                .iter()
                .map(|parameter| parameter.start)
                .collect(),
            point: problem.start(),
        }]);
    }
    let parameters = problem.parameters();
    let mut starts = Vec::with_capacity(request.starts.len());
    for (at, start) in request.starts.iter().enumerate() {
        if start.len() != parameters.len() {
            return Err(FitError::Request(format!(
                "has the start {at} of {} values and {} parameters",
                start.len(),
                parameters.len()
            )));
        }
        let mut point = Vec::with_capacity(start.len());
        for (parameter, value) in parameters.iter().zip(start.iter()) {
            if !value.is_finite() || *value < parameter.lower || *value > parameter.upper {
                return Err(FitError::Request(format!(
                    "has the start {at}, which sets '{}' to {value}, outside its bounds \
                     [{}, {}]",
                    parameter.id, parameter.lower, parameter.upper
                )));
            }
            point.push(parameter.scale.to_search(*value));
        }
        starts.push(Start {
            values: start.clone(),
            point,
        });
    }
    Ok(starts)
}
