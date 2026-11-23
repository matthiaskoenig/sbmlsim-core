//! The contract `fit/1`: a fit as a file, which a host writes and the answer it reads back.
//!
//! A host of the engine (sbmlsim, the command line, JavaScript) resolves its own problem into
//! this schema: the parameters with their bounds and their scale, a request of `simulation/1`
//! per condition, the data of every curve and the optimizer to run. The semantics a fit of
//! the engine and a fit of its host have to agree on lives in the schema, so that a fit is a
//! file one can replay, as `model/2` and `simulation/1` are for a model and a simulation.

use std::sync::Arc;

use rayon::prelude::*;
use sbmlsim_abi::SimulationRequest;
use sbmlsim_host::LoadedModel;
use serde::{Deserialize, Serialize};

use crate::{
    Condition, Curve, DEFAULT_MODEL, Definition, Derived, Fisher, Fit, FitError, JacobianChoice,
    Likelihood, NoiseKind, Parameter, Problem, Profile, Scale, Settings, Stop, Term, Walk, fisher,
    profiles,
};

/// Whether the jacobian of a request is the one of [`JacobianChoice::Auto`], which its
/// serialization leaves out.
fn is_auto(choice: &JacobianChoice) -> bool {
    *choice == JacobianChoice::Auto
}

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
    /// Where the jacobian of a step of the optimizer comes from: `"auto"` what is fastest
    /// for the model, which is the finite differences of the residuals today,
    /// `"sensitivities"` the forward sensitivities of the conditions, where a condition which
    /// does not carry them is refused, `"difference_quotient"` the differences.
    /// `"auto"` when it is absent.
    #[serde(default, skip_serializing_if = "is_auto")]
    pub jacobian: JacobianChoice,
    /// The identifiability of the optimum the fit finds; none of it when the block is absent.
    #[serde(default)]
    pub identifiability: Option<Identifiability>,
}

/// What to ask of the optimum of a fit: how well its data determines it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Identifiability {
    /// The ids of the parameters to profile, in the order of the profiles; every parameter of
    /// the fit when it is empty.
    pub parameters: Vec<String>,
    /// What a profile walks, see [`Walk`].
    pub walk: Walk,
    /// Whether the profiles are walked, which is a fit per point of every one of them.
    pub profiles: bool,
    /// Whether the Fisher information of the optimum is computed, which is one jacobian.
    pub fisher: bool,
    /// The optimizer of the points of a profile, which fit the other parameters while one is
    /// held at a value; the optimizer of the fit itself when it is absent. A caller which
    /// profiles an optimum it already has runs the fit of `max_iterations: 0`, whose answer
    /// is the start it was given, and gives the points an optimizer of their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizer: Option<Settings>,
}

impl Default for Identifiability {
    fn default() -> Identifiability {
        Identifiability {
            parameters: Vec::new(),
            walk: Walk::default(),
            profiles: true,
            fisher: true,
            optimizer: None,
        }
    }
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
    /// The id of the model the condition simulates, one of the models of the fit.
    ///
    /// A fit of one model needs none, and every request written before 0.22.0 has none: the
    /// condition simulates that model. A fit of several models names the model of every
    /// condition, since nothing else says which one it simulates, e.g. the male and the
    /// female body of a model whose data is the data of both.
    ///
    /// Absent, which is the request every release before 0.22.0 reads, there is one model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The values of the condition which follow the parameters of the fit, each a linear form
    /// over them which is evaluated before every simulation and set on its target.
    ///
    /// A value of a request of `simulation/1` is a number, so a condition whose value is a
    /// parameter the fit estimates, e.g. the initial amount of a species which a PEtab
    /// condition assigns an estimated parameter, could not be a condition of a fit: a number
    /// folded when the request was built would hold that value at the start of the fit while
    /// the optimizer moves the parameter. Such a value is given here instead, and the fit
    /// sets it on its target as a pre-initialization of the condition, with the values the
    /// optimizer is at.
    ///
    /// Absent, which is the request every release before 0.17.0 reads, there are none.
    #[serde(default)]
    pub derived: Option<Vec<DerivedRequest>>,
}

/// A value of a condition which follows the parameters of a fit, `constant + Σ factor * p`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DerivedRequest {
    /// The entity of the model the value is set on, a settable selection, which must be no
    /// parameter of the fit and no other derived value of the same condition.
    pub target: String,
    /// The constant term of the linear form, `0` when it is absent.
    #[serde(default)]
    pub constant: f64,
    /// The terms over the parameters of the fit, which may be empty for a constant value.
    #[serde(default)]
    pub terms: Vec<TermRequest>,
}

/// One term of a linear form of a derived value, the parameter it reads and its factor.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TermRequest {
    /// The position of the parameter in the parameters of the fit.
    pub parameter: usize,
    /// What the value of that parameter is multiplied by.
    pub factor: f64,
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
    /// Every value is positive; a positive infinity is allowed and means the point carries
    /// no information: it stays in the residual vector and contributes nothing (its
    /// residual is `+-0.0` and its row of the jacobian is zeros). Zero, a negative value
    /// and `NaN` are refused.
    #[serde(default)]
    pub noise: Option<Vec<f64>>,
    /// The weight of the curve, which multiplies its residuals; one when it is absent.
    #[serde(default = "one")]
    pub weight: f64,
    /// Whether a measurement reads the state after a change of its condition rather than
    /// the one before it, one flag per time in their order.
    ///
    /// The output of a condition has two rows at the time of a change after its start, the
    /// state before the change and the state after it. A measurement reads the first of
    /// them, the state before the change, and one whose flag is true reads the last row of
    /// its time, the state after it and after every event the change fired. Every
    /// measurement reads the state before a change when the field is absent, which is the
    /// behaviour of the releases before 0.16.0.
    #[serde(default)]
    pub after_change: Option<Vec<bool>>,
    /// The noise distribution of the measurements of the curve, which makes its residuals the
    /// ones of a likelihood.
    ///
    /// A curve with a likelihood has **twice** its measurements as residuals: the residual of
    /// every measurement and then, in the same order, the residual of the scale of its noise,
    /// so that half the sum of their squares is the negative log-likelihood of the curve up
    /// to `0.5 * LIKELIHOOD_OFFSET` per measurement, a constant of the data which the host
    /// subtracts (a measurement of a log kind carries `(-2 value).max(0)` more of it). A
    /// curve without one is the weighted least squares of [`CurveRequest::noise`], one
    /// residual per measurement, which no field of this block changes.
    ///
    /// The log kinds mean that the column of the curve and its values are logarithms: the
    /// engine transforms nothing, a host which fits data of a geometric standard deviation
    /// selects a column which is a logarithm and gives the logarithms of its measurements,
    /// and the density of such a kind carries the measurement itself.
    ///
    /// The standard deviation of a measurement is the `noise` of the curve or the
    /// `noise_column` of this block; a curve which carries both, and one which carries
    /// neither, is refused.
    ///
    /// Absent, which is the request every release before 0.23.0 reads, there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub likelihood: Option<LikelihoodRequest>,
}

/// The noise distribution of the measurements of a curve, see [`CurveRequest::likelihood`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LikelihoodRequest {
    /// The distribution of the noise of every measurement of the curve: `"normal"`,
    /// `"log-normal"`, `"laplace"` or `"log-laplace"`.
    pub distribution: NoiseKind,
    /// The column of the simulation of the condition which is the standard deviation of a
    /// measurement, read in the row of that measurement, e.g. a parameter of the fit itself
    /// or an assignment rule of the model over it, which is the noise a fit moves.
    ///
    /// The `noise` of the curve when it is absent, and a curve which carries the two together
    /// is refused. The condition has to select the column, as it selects the one of the
    /// curve, and the standard deviation of a measurement has to be positive and finite there
    /// ([`crate::FAILED_RESIDUAL`] for both rows of a measurement where it is not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_column: Option<String>,
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
    /// The simulations every run of the fit ran together, and the ones of the profiles of its
    /// identifiability: what the fit cost the engine.
    pub simulations: u64,
    /// Where the jacobian of a step came from: `"sensitivities"` or `"difference_quotient"`,
    /// which `"auto"` of the request resolves to.
    pub jacobian: JacobianChoice,
    /// The time every run of the fit took together, without the identifiability of its
    /// optimum, which every profile of [`FitAnswer::profiles`] carries its own share of.
    pub seconds: f64,
    /// The residuals of the optimum, in the order of the curves of the request.
    pub residuals: Vec<f64>,
    /// Every run of the fit, in the order of its starts.
    pub runs: Vec<RunAnswer>,
    /// The profiles of the likelihood of the parameters, when the fit asked for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Profile>,
    /// The Fisher information of the optimum, when the fit asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fisher: Option<Fisher>,
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
    /// Where they came from: `"sensitivities"` or `"difference_quotient"`, which `"auto"` of
    /// the request resolves to, the same for every run of a fit.
    pub jacobian: JacobianChoice,
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

/// The problem of a fit of the contract `fit/1`, resolved against a model and the pool of
/// its batch, for a caller whose optimizer is its own ([`Problem::residuals`],
/// [`Problem::jacobian`]).
///
/// The starts and the identifiability of the request are none of it: they belong to a run of
/// the fit, which [`fit`] does.
///
/// # Errors
///
/// As [`fit`], without the ones of a run.
pub fn problem_of(
    request: &FitRequest,
    model: Arc<LoadedModel>,
    threads: Option<usize>,
) -> Result<Problem, FitError> {
    problem_of_models(request, vec![(DEFAULT_MODEL.to_string(), model)], threads)
}

/// The problem of a request of `fit/1` of several models, each with the id its conditions
/// name it by, see [`Problem::of_models`].
///
/// # Errors
///
/// As [`problem_of`], and for a condition which names a model the fit has not or names none
/// although the fit has several.
pub fn problem_of_models(
    request: &FitRequest,
    models: Vec<(String, Arc<LoadedModel>)>,
    threads: Option<usize>,
) -> Result<Problem, FitError> {
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
    Problem::of_models(definition, models, threads)
}

/// What a fit calls with a run of it which finished, see [`fit_reporting`].
///
/// It is called on the thread of the pool which ran that start, so a caller of several
/// threads is called from several of them, and the index is the start of the run in the
/// request. A fit which is stopped reports the runs which finished before it was.
pub type OnRun<'a> = &'a (dyn Fn(usize, &RunAnswer) + Send + Sync);

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
    fit_reporting(request, model, threads, stop, None)
}

/// Runs a fit of the contract `fit/1` of several models, each with the id its conditions name
/// it by, see [`Problem::of_models`].
///
/// # Errors
///
/// As [`fit_reporting_models`].
pub fn fit_models(
    request: &FitRequest,
    models: Vec<(String, Arc<LoadedModel>)>,
    threads: Option<usize>,
    stop: &Stop,
) -> Result<FitAnswer, FitError> {
    fit_reporting_models(request, models, threads, stop, None)
}

/// Runs a fit of the contract `fit/1` which reports every run of it as it finishes.
///
/// The answer is the one of [`fit_with`] and carries every run as well: `on_run` is what a
/// caller which cannot wait for the answer needs, e.g. to report the progress of a long fit,
/// to store its runs while it runs or to keep the runs of a fit which is stopped.
///
/// # Errors
///
/// As [`fit`].
pub fn fit_reporting(
    request: &FitRequest,
    model: Arc<LoadedModel>,
    threads: Option<usize>,
    stop: &Stop,
    on_run: Option<OnRun<'_>>,
) -> Result<FitAnswer, FitError> {
    fit_reporting_models(
        request,
        vec![(DEFAULT_MODEL.to_string(), model)],
        threads,
        stop,
        on_run,
    )
}

/// Runs a fit of the contract `fit/1` of several models which reports every run of it as it
/// finishes, see [`fit_reporting`] and [`Problem::of_models`].
///
/// # Errors
///
/// As [`fit_reporting`], and for a condition which names a model the fit has not or names
/// none although the fit has several.
pub fn fit_reporting_models(
    request: &FitRequest,
    models: Vec<(String, Arc<LoadedModel>)>,
    threads: Option<usize>,
    stop: &Stop,
    on_run: Option<OnRun<'_>>,
) -> Result<FitAnswer, FitError> {
    let problem = problem_of_models(request, models, threads)?;
    let starts = starts_of(request, &problem)?;
    let began = std::time::Instant::now();
    // the starts do not depend on each other, so they run next to each other on the pool of
    // the batch: a start whose evaluation fans fewer conditions out than the pool has threads
    // leaves them to the starts beside it. A fit which was stopped before a start began runs
    // none of it, as a sequence of them would have left it out.
    let fits: Vec<Option<Result<(Fit, RunAnswer), FitError>>> = problem.install(|| {
        starts
            .par_iter()
            .enumerate()
            .map(|(index, start)| {
                (!stop.stopped()).then(|| {
                    let fit = problem.optimize_with(&request.optimizer, &start.point, stop)?;
                    let run = answer_of(start, &fit, problem.jacobian_of());
                    // the run is reported where it finished, before the starts beside it
                    // finished, which is what a caller of a long fit follows it by
                    if let Some(on_run) = on_run {
                        on_run(index, &run);
                    }
                    Ok((fit, run))
                })
            })
            .collect()
    });
    let mut runs: Vec<RunAnswer> = Vec::with_capacity(starts.len());
    let mut best: Option<Fit> = None;
    for fit in fits {
        let Some(fit) = fit else { continue };
        let (fit, run) = fit?;
        runs.push(run);
        if best.as_ref().is_none_or(|best| fit.cost < best.cost) {
            best = Some(fit);
        }
    }
    let best = best.ok_or_else(|| FitError::Request("has no start to run".to_string()))?;
    let ran = began.elapsed().as_secs_f64();
    // how well the data determines the optimum of the best run, when the fit asks for it
    let (walked, information) = match &request.identifiability {
        None => (Vec::new(), None),
        Some(analysis) => {
            let which = which_of(analysis, &problem)?;
            // the points of a profile fit the other parameters, which the optimizer of the
            // analysis runs where it has one of its own
            let optimizer = analysis.optimizer.as_ref().unwrap_or(&request.optimizer);
            let walked = if analysis.profiles {
                profiles(&problem, optimizer, &best.x, &which, &analysis.walk, stop)?
            } else {
                Vec::new()
            };
            let information = if analysis.fisher {
                Some(fisher(&problem, &best.x)?)
            } else {
                None
            };
            (walked, information)
        }
    };
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
        simulations: problem.simulations(),
        jacobian: problem.jacobian_of(),
        seconds: ran,
        residuals: best.residuals,
        runs,
        profiles: walked,
        fisher: information,
    })
}

/// The parameters of an identifiability analysis as their indices in the fit.
fn which_of(analysis: &Identifiability, problem: &Problem) -> Result<Vec<usize>, FitError> {
    let mut which = Vec::with_capacity(analysis.parameters.len());
    for id in &analysis.parameters {
        let at = problem
            .parameters()
            .iter()
            .position(|parameter| parameter.id == *id)
            .ok_or_else(|| {
                FitError::Request(format!(
                    "asks for the profile of '{id}', which is none of its parameters"
                ))
            })?;
        which.push(at);
    }
    Ok(which)
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
            after_change: curve.after_change.clone(),
            likelihood: curve.likelihood.as_ref().map(|likelihood| Likelihood {
                distribution: likelihood.distribution,
                noise_column: likelihood.noise_column.clone(),
            }),
        });
    }
    Ok(Definition {
        jacobian: request.jacobian,
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
            .map(|condition| Condition {
                request: condition.request.clone(),
                model: condition.model.clone(),
                derived: condition
                    .derived
                    .as_ref()
                    .map(|derived| {
                        derived
                            .iter()
                            .map(|value| Derived {
                                target: value.target.clone(),
                                constant: value.constant,
                                terms: value
                                    .terms
                                    .iter()
                                    .map(|term| Term {
                                        parameter: term.parameter,
                                        factor: term.factor,
                                    })
                                    .collect(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
        curves,
    })
}

/// The starts of a fit, the values of its parameters as given and the point of the search
/// space of every one of them; the start of the parameters themselves when the request gives
/// none.
/// The answer of a run of a fit: where it started, where it ended and what it took.
fn answer_of(start: &Start, fit: &Fit, jacobian: JacobianChoice) -> RunAnswer {
    RunAnswer {
        start: start.values.clone(),
        values: fit.values.clone(),
        cost: fit.cost,
        status: fit.status.clone(),
        iterations: fit.iterations,
        evaluations: fit.evaluations,
        jacobians: fit.jacobians,
        jacobian,
        seconds: fit.seconds,
    }
}

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
