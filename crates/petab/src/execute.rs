//! The execution of a plan on the engine: its simulations, its evaluation and its fit.
//!
//! The definition of the fit of a plan in `sbmlsim-fit` ([`Plan::definition`]) is pure, so a
//! host of JavaScript which simulates the fit itself (`sbmlsim_fit::Asking`) takes it as well;
//! the rest is the feature `native`.

use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "native")]
use rayon::prelude::*;
#[cfg(feature = "native")]
use sbmlsim_abi::SimulationResult;
use sbmlsim_fit::{
    Condition, Curve, Definition, Derived, Form, JacobianChoice, Likelihood, NoiseKind, Parameter,
    Scale, Term,
};
#[cfg(feature = "native")]
use sbmlsim_fit::{Fit, Problem, Settings};
#[cfg(feature = "native")]
use sbmlsim_host::{LoadedModel, Runtime};
use sbmlsim_sbml::math::{Node, Op};

use crate::PetabError;
use crate::likelihood::{NoiseDistribution, PriorKind};
use crate::math::{self, Formula};
#[cfg(feature = "native")]
use crate::plan::Evaluation;
use crate::plan::{Noise, ParameterScale, Plan};

/// A plan whose models are compiled and loaded into the engine.
#[cfg(feature = "native")]
pub struct Executor {
    plan: Plan,
    models: Vec<Arc<LoadedModel>>,
}

/// The answer of a fit of a problem.
#[cfg(feature = "native")]
#[derive(Debug, Clone)]
pub struct FitAnswer {
    /// The values of the estimated parameters at the optimum, in the order of
    /// [`Plan::estimated`].
    pub x: Vec<f64>,
    /// The negative log posterior of the optimum, the negative log-likelihood for a problem
    /// without priors: PEtab's objective.
    pub objective: f64,
    /// The fit of `sbmlsim-fit`, whose cost carries the offsets of its rows.
    pub fit: Fit,
}

#[cfg(feature = "native")]
impl Executor {
    /// Compiles the models of a plan and loads them into the embedded engine.
    ///
    /// # Errors
    ///
    /// A model which does not compile and an engine which cannot be loaded.
    pub fn new(plan: Plan) -> Result<Executor, PetabError> {
        let runtime = Runtime::embedded().map_err(|err| PetabError::Engine(err.to_string()))?;
        Executor::with_runtime(plan, &runtime)
    }

    /// Compiles the models of a plan and loads them into a runtime.
    ///
    /// # Errors
    ///
    /// As [`Executor::new`].
    pub fn with_runtime(plan: Plan, runtime: &Runtime) -> Result<Executor, PetabError> {
        let mut models = Vec::with_capacity(plan.models.len());
        for k in 0..plan.models.len() {
            let compiled = plan.compile(k)?;
            let loaded = runtime
                .load(&compiled.wasm, &compiled.description)
                .map_err(|err| PetabError::Engine(err.to_string()))?;
            models.push(Arc::new(loaded));
        }
        Ok(Executor { plan, models })
    }

    /// The plan.
    #[must_use]
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Simulates every simulation of the plan at the values of the estimated parameters, next
    /// to each other on the threads of rayon.
    #[must_use]
    pub fn simulate(&self, x: &[f64]) -> Vec<Result<SimulationResult, String>> {
        let requests = self.plan.requests(x);
        requests
            .par_iter()
            .zip(self.plan.simulations.par_iter())
            .map(|(request, simulation)| {
                let mut instance = self.models[simulation.model]
                    .instantiate()
                    .map_err(|err| err.to_string())?;
                instance.simulate(request).map_err(|err| err.to_string())
            })
            .collect()
    }

    /// Evaluates the problem at the values of the estimated parameters.
    #[must_use]
    pub fn evaluate(&self, x: &[f64]) -> Evaluation {
        let results = self.simulate(x);
        self.plan.evaluate(x, &results)
    }

    /// The definition of the fit of the problem, [`Plan::definition`].
    ///
    /// # Errors
    ///
    /// As [`Plan::definition`].
    pub fn definition(&self, start: &[f64]) -> Result<Definition, PetabError> {
        self.plan.definition(start)
    }
}

impl Plan {
    /// The definition of the fit of the problem in `sbmlsim-fit`, starting at the values of
    /// the estimated parameters `start`, see [`Executor::problem`].
    ///
    /// # Errors
    ///
    /// What the fit of the engine cannot carry: a prior whose density is not bounded between
    /// the bounds of its parameter, so that the posterior has no largest value (a gamma prior
    /// of a shape below one, a chi-squared prior of fewer than two degrees of freedom and a
    /// log-Laplace prior of a scale above one, at a lower bound of zero, e.g. the
    /// `log-laplace(3, 5)` of the cases 0024 and 0025 of the test suite).
    #[allow(clippy::too_many_lines)]
    pub fn definition(&self, start: &[f64]) -> Result<Definition, PetabError> {
        let plan = self;
        let estimated = plan.estimated();
        let values = plan.values(start);
        let position: HashMap<usize, usize> = estimated
            .iter()
            .enumerate()
            .map(|(k, &parameter)| (parameter, k))
            .collect();
        let gap = |message: String| PetabError::Unsupported(format!("in a fit: {message}"));

        // the box a parameter is searched in: its bounds, within the support of its prior,
        // outside which the posterior is zero and the row of the prior is none
        let boxes: Vec<(f64, f64)> = estimated
            .iter()
            .map(|&k| {
                let parameter = &plan.parameters[k];
                match &parameter.prior {
                    Some(prior) => {
                        let (low, high) = prior.support();
                        (parameter.lower.max(low), parameter.upper.min(high))
                    }
                    None => (parameter.lower, parameter.upper),
                }
            })
            .collect();
        let parameters = estimated
            .iter()
            .zip(start)
            .zip(&boxes)
            .map(|((&k, &value), &(lower, upper))| {
                let parameter = &plan.parameters[k];
                let scale = match parameter.scale {
                    Some(ParameterScale::Linear) => Scale::Lin,
                    Some(ParameterScale::Log) => Scale::Log,
                    Some(ParameterScale::Log10) => Scale::Log10,
                    None if lower > 0.0 && upper.is_finite() => Scale::Log10,
                    None => Scale::Lin,
                };
                Parameter {
                    id: parameter.target.clone(),
                    start: value.max(lower).min(upper),
                    lower,
                    upper,
                    scale,
                }
            })
            .collect();

        let mut conditions = Vec::with_capacity(plan.simulations.len());
        for simulation in &plan.simulations {
            let mut request = simulation.request.clone();
            request.preinit = plan
                .parameters
                .iter()
                .zip(&values)
                .filter(|(parameter, _)| !parameter.estimate)
                .map(|(parameter, value)| (parameter.target.clone(), *value))
                .collect();
            let mut derived = Vec::new();
            for initial in &simulation.initial {
                let reads_estimated = initial
                    .parameters
                    .iter()
                    .any(|k| plan.parameters[*k].estimate);
                if !reads_estimated {
                    let read: Vec<f64> = initial.parameters.iter().map(|&k| values[k]).collect();
                    request.preinit.push((
                        initial.target.clone(),
                        initial.formula.evaluate(&read, request.start),
                    ));
                    continue;
                }
                let estimated_of = |symbol: &str| {
                    plan.parameters
                        .iter()
                        .position(|p| p.target == symbol)
                        .and_then(|k| position.get(&k).copied())
                };
                let fixed_of = |symbol: &str| {
                    plan.parameters
                        .iter()
                        .position(|p| p.target == symbol)
                        .map_or(f64::NAN, |k| values[k])
                };
                // a linear form is exact and needs nothing of the plan, every other value is
                // its formula, evaluated where the optimizer is
                derived.push(match linear_form(&initial.node, &estimated_of, &fixed_of) {
                    Some((constant, terms)) => Derived {
                        target: initial.target.clone(),
                        constant,
                        terms: terms
                            .into_iter()
                            .map(|(parameter, factor)| Term { parameter, factor })
                            .collect(),
                        form: None,
                    },
                    None => Derived {
                        target: initial.target.clone(),
                        constant: 0.0,
                        terms: Vec::new(),
                        form: Some(Arc::new(
                            InitialForm::new(
                                &initial.formula,
                                initial
                                    .parameters
                                    .iter()
                                    .map(|k| match position.get(k) {
                                        Some(at) => Read::Estimated(*at),
                                        None => Read::Fixed(values[*k]),
                                    })
                                    .collect(),
                                request.start,
                            )
                            .map_err(|message| {
                                gap(format!(
                                    "the value of '{}' of a first period has no derivative \
                                         in the estimated parameters: {message}",
                                    initial.target
                                ))
                            })?,
                        )),
                    },
                });
            }
            conditions.push(Condition {
                request,
                model: Some(plan.models[simulation.model].id.clone()),
                derived,
                pinned: Vec::new(),
            });
        }

        // a curve per column, noise and distribution of the measurements of a simulation
        let mut curves: Vec<Curve> = Vec::new();
        let mut curve_of: HashMap<(usize, String, Option<String>, NoiseDistribution), usize> =
            HashMap::new();
        for m in plan.measurements.iter().filter(|m| m.enters) {
            let column = m.log_column.clone().unwrap_or_else(|| m.column.clone());
            let noise_column = match &m.noise {
                Noise::Column(column) => Some(column.clone()),
                Noise::Number(_) => None,
            };
            let key = (
                m.simulation,
                column.clone(),
                noise_column.clone(),
                m.distribution,
            );
            let at = *curve_of.entry(key).or_insert_with(|| {
                curves.push(Curve {
                    condition: m.simulation,
                    column,
                    times: Vec::new(),
                    values: Vec::new(),
                    noise: noise_column.is_none().then(Vec::new),
                    weight: 1.0,
                    after_change: Some(Vec::new()),
                    likelihood: Some(Likelihood {
                        distribution: noise_kind(m.distribution),
                        noise_column,
                    }),
                    reduction: None,
                    dimension: None,
                });
                curves.len() - 1
            });
            let curve = &mut curves[at];
            curve.times.push(m.time);
            curve.values.push(if m.distribution.logarithmic() {
                m.value.ln()
            } else {
                m.value
            });
            if let (Some(noise), Noise::Number(value)) = (curve.noise.as_mut(), &m.noise) {
                noise.push(*value);
            }
            if let Some(after) = curve.after_change.as_mut() {
                after.push(m.after);
            }
        }

        let mut priors = Vec::new();
        for (position, (&k, &(lower, upper))) in estimated.iter().zip(&boxes).enumerate() {
            let parameter = &plan.parameters[k];
            let Some(prior) = &parameter.prior else {
                continue;
            };
            let kind = match prior.kind {
                PriorKind::Normal => sbmlsim_fit::PriorKind::Normal,
                PriorKind::LogNormal => sbmlsim_fit::PriorKind::LogNormal,
                PriorKind::Uniform => sbmlsim_fit::PriorKind::Uniform,
                PriorKind::LogUniform => sbmlsim_fit::PriorKind::LogUniform,
                PriorKind::Laplace => sbmlsim_fit::PriorKind::Laplace,
                PriorKind::LogLaplace => sbmlsim_fit::PriorKind::LogLaplace,
                PriorKind::Cauchy => sbmlsim_fit::PriorKind::Cauchy,
                PriorKind::Gamma => sbmlsim_fit::PriorKind::Gamma,
                PriorKind::ChiSquare => sbmlsim_fit::PriorKind::ChiSquare,
                PriorKind::Exponential => sbmlsim_fit::PriorKind::Exponential,
                PriorKind::Rayleigh => sbmlsim_fit::PriorKind::Rayleigh,
            };
            priors.push(sbmlsim_fit::Prior {
                parameter: position,
                kind,
                parameters: prior.parameters.clone(),
                offset: prior_offset(prior, lower, upper).ok_or_else(|| {
                    gap(format!(
                        "the {} prior of '{}' has a density which is not bounded between \
                         {lower} and {upper}, so the posterior has no largest value and a fit \
                         no optimum",
                        prior.kind.name(),
                        parameter.id
                    ))
                })?,
                // the mass of the prior between the bounds of the parameter, which is its mass
                // in the box of its support as well
                normalizer: prior.log_mass,
                lower,
                upper,
            });
        }

        Ok(Definition {
            parameters,
            conditions,
            curves,
            jacobian: JacobianChoice::Auto,
            priors,
            prior_correlation: None,
        })
    }
}

#[cfg(feature = "native")]
impl Executor {
    /// The problem of `sbmlsim-fit` of the problem, whose optimizers fit its estimated
    /// parameters, starting at their nominal values.
    ///
    /// Every measurement is a curve of a likelihood, so half the sum of the squares of the
    /// residuals of the problem is the negative log posterior of PEtab plus a constant of the
    /// data and the priors, [`Executor::cost_offset`].
    ///
    /// A parameter is searched between its bounds, within the support of its prior, outside
    /// which its posterior is zero (a uniform prior narrower than the bounds), and a bound may
    /// be infinite. It is searched on the scale of the extension `sbmlsim`
    /// ([`crate::PlanParameter::scale`]), else on the scale `log10` when the bounds of its box
    /// are positive and finite and on the linear scale otherwise.
    ///
    /// # Errors
    ///
    /// As [`Executor::definition`], and the refusals of the fit of the engine, e.g. an
    /// estimated parameter without a nominal value, where the fit starts, or a logarithmic
    /// scale of the extension of a parameter whose lower bound is negative.
    pub fn problem(&self, threads: usize) -> Result<Problem, PetabError> {
        self.problem_of(self.plan.definition(&self.plan.nominal())?, threads)
    }

    /// The problem of `sbmlsim-fit` of a definition of [`Executor::definition`] which a caller
    /// changed, e.g. its start or the jacobian it takes, on the models of the plan.
    ///
    /// # Errors
    ///
    /// The refusals of the fit of the engine.
    pub fn problem_of(
        &self,
        definition: Definition,
        threads: usize,
    ) -> Result<Problem, PetabError> {
        let models = self
            .plan
            .models
            .iter()
            .zip(&self.models)
            .map(|(model, loaded)| (model.id.clone(), Arc::clone(loaded)))
            .collect();
        Problem::of_models(definition, models, threads)
            .map_err(|err| PetabError::Engine(err.to_string()))
    }

    /// The offset of the cost of [`Executor::problem`], [`Plan::cost_offset`].
    ///
    /// # Errors
    ///
    /// As [`Plan::definition`].
    pub fn cost_offset(&self) -> Result<f64, PetabError> {
        self.plan.cost_offset()
    }
}

impl Plan {
    /// What the cost of [`Executor::problem`] carries on top of the negative log posterior:
    /// the offsets of the rows of the scale of every measurement and of every prior, and the
    /// log density of the uniform priors PEtab gives a parameter without one when another
    /// parameter has one, which no row carries.
    ///
    /// The uniform prior of a parameter whose bounds are not finite has no density, so PEtab's
    /// log posterior of such a problem is `-inf` everywhere: its constant is left out, and the
    /// objective of a fit is the negative log posterior of the other priors and the data, the
    /// improper flat prior being a density of one.
    ///
    /// # Errors
    ///
    /// As [`Plan::definition`].
    pub fn cost_offset(&self) -> Result<f64, PetabError> {
        let definition = self.definition(&self.nominal())?;
        let mut offset = 0.0;
        for curve in &definition.curves {
            let log = curve.likelihood.as_ref().is_some_and(|l| {
                matches!(l.distribution, NoiseKind::LogNormal | NoiseKind::LogLaplace)
            });
            for value in &curve.values {
                offset += 0.5
                    * (sbmlsim_fit::LIKELIHOOD_OFFSET
                        + if log { (-2.0 * value).max(0.0) } else { 0.0 });
            }
        }
        for prior in &definition.priors {
            offset += 0.5 * prior.offset;
        }
        let estimated = self.estimated();
        if estimated
            .iter()
            .any(|&k| self.parameters[k].prior.is_some())
        {
            for &k in &estimated {
                let parameter = &self.parameters[k];
                if parameter.prior.is_none() && (parameter.upper - parameter.lower).is_finite() {
                    // the cost lacks the negative log density of the uniform prior
                    offset -= (parameter.upper - parameter.lower).ln();
                }
            }
        }
        Ok(offset)
    }
}

#[cfg(feature = "native")]
impl Executor {
    /// Fits the estimated parameters with an optimizer of `sbmlsim-fit` from their nominal
    /// values.
    ///
    /// # Errors
    ///
    /// As [`Executor::problem`], and a failure of the optimizer.
    pub fn fit(&self, settings: &Settings, threads: usize) -> Result<FitAnswer, PetabError> {
        let problem = self.problem(threads)?;
        let offset = self.plan.cost_offset()?;
        let fit = problem
            .optimize(settings)
            .map_err(|err| PetabError::Engine(err.to_string()))?;
        Ok(FitAnswer {
            x: fit.values.clone(),
            objective: fit.cost - offset,
            fit,
        })
    }
}

/// The kind of the likelihood of `sbmlsim-fit` of a noise distribution.
fn noise_kind(distribution: NoiseDistribution) -> NoiseKind {
    match distribution {
        NoiseDistribution::Normal => NoiseKind::Normal,
        NoiseDistribution::LogNormal => NoiseKind::LogNormal,
        NoiseDistribution::Laplace => NoiseKind::Laplace,
        NoiseDistribution::LogLaplace => NoiseKind::LogLaplace,
    }
}

/// The offset of the row of a prior: twice its largest truncated log density in the box
/// `[lower, upper]` it is searched in, so the row is real wherever the parameter goes.
///
/// Every kind is unimodal, so the largest density is at its mode, or at the bound of the box
/// next to it; `None` for a density which is not bounded in the box: at a lower bound of zero,
/// the density of a gamma kind of a shape below one, of a chi-squared kind of fewer than two
/// degrees of freedom and of a log-Laplace kind of a scale above one grows without a bound
/// (`x^(1/b - 1)` of the log-Laplace one), so the posterior has no largest value and a fit no
/// optimum.
fn prior_offset(prior: &crate::Prior, lower: f64, upper: f64) -> Option<f64> {
    let p = &prior.parameters;
    let unbounded = lower <= 0.0
        && match prior.kind {
            PriorKind::Gamma => p[0] < 1.0,
            PriorKind::ChiSquare => p[0] < 2.0,
            PriorKind::LogLaplace => p[1] > 1.0,
            _ => false,
        };
    if unbounded {
        return None;
    }
    let mode = match prior.kind {
        PriorKind::Normal | PriorKind::Laplace | PriorKind::Cauchy => p[0],
        PriorKind::LogNormal => (p[0] - p[1] * p[1]).exp(),
        PriorKind::LogLaplace => p[0].exp(),
        PriorKind::Gamma => ((p[0] - 1.0) * p[1]).max(0.0),
        PriorKind::ChiSquare => (p[0] - 2.0).max(0.0),
        PriorKind::Rayleigh => p[0],
        PriorKind::Exponential | PriorKind::Uniform | PriorKind::LogUniform => lower,
    };
    let candidates = [lower, upper, mode.max(lower).min(upper)];
    let largest = candidates
        .into_iter()
        .filter(|x| x.is_finite())
        .map(|x| prior.log_density(x))
        .filter(|density| !density.is_nan())
        .fold(f64::NEG_INFINITY, f64::max);
    largest.is_finite().then_some(2.0 * largest)
}

/// What a symbol of the formula of a value of a first period reads.
#[derive(Debug, Clone, Copy)]
enum Read {
    /// The estimated parameter of this position in the fit.
    Estimated(usize),
    /// A parameter which is not estimated, at its value.
    Fixed(f64),
}

/// A value of a first period which is no linear form of the estimated parameters, e.g. their
/// product, as the [`Form`] of a derived value of the fit: its formula at the values the
/// optimizer is at, which is the number an evaluation of the problem sets, and its partial
/// derivatives, the formulas of the derivatives of its operators, which every function of
/// the math of PEtab has.
#[derive(Debug)]
struct InitialForm {
    /// The formula of the value, over its symbols.
    formula: Formula,
    /// What every symbol of the formula reads.
    reads: Vec<Read>,
    /// The time the formula is evaluated at, the start of the simulation.
    time: f64,
    /// The positions of the estimated parameters the formula reads.
    parameters: Vec<usize>,
    /// The partial derivative of the formula in each of them.
    derivatives: Vec<Formula>,
}

impl InitialForm {
    /// The form of a formula whose symbols read what `reads` says, at a time.
    ///
    /// # Errors
    ///
    /// An operator of the formula which has no derivative, which no function of PEtab is.
    fn new(formula: &Formula, reads: Vec<Read>, time: f64) -> Result<InitialForm, String> {
        let (symbols, parameters): (Vec<usize>, Vec<usize>) = reads
            .iter()
            .enumerate()
            .filter_map(|(symbol, read)| match read {
                Read::Estimated(at) => Some((symbol, *at)),
                Read::Fixed(_) => None,
            })
            .unzip();
        let derivatives = symbols
            .iter()
            .map(|symbol| formula.derivative(*symbol))
            .collect::<Result<_, _>>()?;
        Ok(InitialForm {
            formula: formula.clone(),
            reads,
            time,
            parameters,
            derivatives,
        })
    }

    /// The values of the symbols at the values of the parameters of the fit.
    fn read(&self, parameters: &[f64]) -> Vec<f64> {
        self.reads
            .iter()
            .map(|read| match read {
                Read::Estimated(at) => parameters[*at],
                Read::Fixed(value) => *value,
            })
            .collect()
    }
}

impl Form for InitialForm {
    fn parameters(&self) -> &[usize] {
        &self.parameters
    }

    fn value(&self, parameters: &[f64]) -> f64 {
        self.formula.evaluate(&self.read(parameters), self.time)
    }

    fn gradient(&self, parameters: &[f64]) -> Vec<f64> {
        let read = self.read(parameters);
        self.derivatives
            .iter()
            .map(|derivative| derivative.evaluate(&read, self.time))
            .collect()
    }
}

/// The linear form of an expression in the estimated parameters, `constant + Σ factor * p`:
/// `None` for an expression which is not linear in them.
fn linear_form(
    node: &Node,
    estimated: &dyn Fn(&str) -> Option<usize>,
    fixed: &dyn Fn(&str) -> f64,
) -> Option<(f64, Vec<(usize, f64)>)> {
    let reads_estimated = |node: &Node| {
        math::symbols(node)
            .iter()
            .any(|symbol| estimated(symbol).is_some())
    };
    if !reads_estimated(node) {
        let formula = Formula::new(node).ok()?;
        let read: Vec<f64> = formula.symbols().iter().map(|s| fixed(s)).collect();
        return Some((formula.evaluate(&read, 0.0), Vec::new()));
    }
    let scale = |(constant, terms): (f64, Vec<(usize, f64)>), by: f64| {
        (
            constant * by,
            terms.into_iter().map(|(k, f)| (k, f * by)).collect(),
        )
    };
    let add = |(c1, mut t1): (f64, Vec<(usize, f64)>), (c2, t2): (f64, Vec<(usize, f64)>)| {
        for (k, f) in t2 {
            match t1.iter_mut().find(|(j, _)| *j == k) {
                Some(term) => term.1 += f,
                None => t1.push((k, f)),
            }
        }
        (c1 + c2, t1)
    };
    match node {
        Node::Symbol(symbol) => estimated(symbol).map(|k| (0.0, vec![(k, 1.0)])),
        Node::Apply { op, args } => match (op, args.as_slice()) {
            (Op::Plus, _) => args.iter().try_fold((0.0, Vec::new()), |sum, arg| {
                Some(add(sum, linear_form(arg, estimated, fixed)?))
            }),
            (Op::Minus, [a]) => Some(scale(linear_form(a, estimated, fixed)?, -1.0)),
            (Op::Minus, [a, b]) => Some(add(
                linear_form(a, estimated, fixed)?,
                scale(linear_form(b, estimated, fixed)?, -1.0),
            )),
            (Op::Times, _) => {
                let mut form = (1.0, Vec::new());
                let mut variable = false;
                for arg in args {
                    let factor = linear_form(arg, estimated, fixed)?;
                    if factor.1.is_empty() {
                        form = scale(form, factor.0);
                    } else if variable {
                        return None;
                    } else {
                        variable = true;
                        form = scale(factor, form.0);
                    }
                }
                Some(form)
            }
            (Op::Divide, [a, b]) if !reads_estimated(b) => {
                let (denominator, _) = linear_form(b, estimated, fixed)?;
                Some(scale(linear_form(a, estimated, fixed)?, 1.0 / denominator))
            }
            _ => None,
        },
        _ => None,
    }
}
