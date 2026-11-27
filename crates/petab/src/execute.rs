//! The execution of a plan on the engine: its simulations, its evaluation and its fit.

use std::collections::HashMap;
use std::sync::Arc;

use rayon::prelude::*;
use sbmlsim_abi::SimulationResult;
use sbmlsim_fit::{
    Condition, Curve, Definition, Derived, Fit, JacobianChoice, Likelihood, NoiseKind, Parameter,
    Problem, Scale, Settings, Term,
};
use sbmlsim_host::{LoadedModel, Runtime};
use sbmlsim_sbml::math::{Node, Op};

use crate::PetabError;
use crate::likelihood::{NoiseDistribution, PriorKind};
use crate::math::{self, Formula};
use crate::plan::{Evaluation, Noise, Plan};

/// A plan whose models are compiled and loaded into the engine.
pub struct Executor {
    plan: Plan,
    models: Vec<Arc<LoadedModel>>,
}

/// The answer of a fit of a problem.
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

    /// The definition of the fit of the problem in `sbmlsim-fit`, starting at the values of
    /// the estimated parameters `start`, see [`Executor::problem`].
    ///
    /// # Errors
    ///
    /// What the fit of the engine cannot carry: a value of a first period which is no linear
    /// form of the estimated parameters, a prior of a kind the engine does not evaluate and a
    /// problem without estimated parameters or without data.
    #[allow(clippy::too_many_lines)]
    pub fn definition(&self, start: &[f64]) -> Result<Definition, PetabError> {
        let plan = &self.plan;
        let estimated = plan.estimated();
        let values = plan.values(start);
        let position: HashMap<usize, usize> = estimated
            .iter()
            .enumerate()
            .map(|(k, &parameter)| (parameter, k))
            .collect();
        let gap = |message: String| PetabError::Unsupported(format!("in a fit: {message}"));

        let parameters = estimated
            .iter()
            .zip(start)
            .map(|(&k, &value)| {
                let parameter = &plan.parameters[k];
                let scale = if parameter.lower > 0.0 && parameter.upper.is_finite() {
                    Scale::Log10
                } else {
                    Scale::Lin
                };
                Parameter {
                    id: parameter.target.clone(),
                    start: value.clamp(parameter.lower, parameter.upper),
                    lower: parameter.lower,
                    upper: parameter.upper,
                    scale,
                }
            })
            .collect();

        let mut conditions = Vec::with_capacity(plan.simulations.len());
        for (index, simulation) in plan.simulations.iter().enumerate() {
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
                let (constant, terms) = linear_form(&initial.node, &estimated_of, &fixed_of)
                    .ok_or_else(|| {
                        gap(format!(
                            "the value of '{}' of the first period of the simulation {index} is no \
                             linear form of the estimated parameters",
                            initial.target
                        ))
                    })?;
                derived.push(Derived {
                    target: initial.target.clone(),
                    constant,
                    terms: terms
                        .into_iter()
                        .map(|(parameter, factor)| Term { parameter, factor })
                        .collect(),
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
        for (position, &k) in estimated.iter().enumerate() {
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
                other => {
                    return Err(gap(format!(
                        "the prior '{}' of '{}' is no prior the engine evaluates",
                        other.name(),
                        parameter.id
                    )));
                }
            };
            priors.push(sbmlsim_fit::Prior {
                parameter: position,
                kind,
                parameters: prior.parameters.clone(),
                offset: prior_offset(prior).ok_or_else(|| {
                    gap(format!(
                        "the prior of '{}' has no largest density between its bounds",
                        parameter.id
                    ))
                })?,
                normalizer: prior.log_mass,
                lower: parameter.lower,
                upper: parameter.upper,
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

    /// The problem of `sbmlsim-fit` of the problem, whose optimizers fit its estimated
    /// parameters, starting at their nominal values.
    ///
    /// Every measurement is a curve of a likelihood, so half the sum of the squares of the
    /// residuals of the problem is the negative log posterior of PEtab plus a constant of the
    /// data and the priors, [`Executor::cost_offset`]. A parameter whose bounds are positive is
    /// searched on the scale `log10`, any other one on the linear scale.
    ///
    /// # Errors
    ///
    /// As [`Executor::definition`], and the refusals of the fit of the engine, e.g. a bound
    /// which is not finite.
    pub fn problem(&self, threads: usize) -> Result<Problem, PetabError> {
        let definition = self.definition(&self.plan.nominal())?;
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

    /// What the cost of [`Executor::problem`] carries on top of the negative log posterior:
    /// the offsets of the rows of the scale of every measurement and of every prior, and the
    /// log density of the uniform priors PEtab gives a parameter without one when another
    /// parameter has one, which no row carries.
    ///
    /// # Errors
    ///
    /// As [`Executor::definition`].
    pub fn cost_offset(&self) -> Result<f64, PetabError> {
        let definition = self.definition(&self.plan.nominal())?;
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
        let estimated = self.plan.estimated();
        if estimated
            .iter()
            .any(|&k| self.plan.parameters[k].prior.is_some())
        {
            for &k in &estimated {
                let parameter = &self.plan.parameters[k];
                if parameter.prior.is_none() {
                    // the cost lacks the negative log density of the uniform prior
                    offset -= (parameter.upper - parameter.lower).ln();
                }
            }
        }
        Ok(offset)
    }

    /// Fits the estimated parameters with an optimizer of `sbmlsim-fit` from their nominal
    /// values.
    ///
    /// # Errors
    ///
    /// As [`Executor::problem`], and a failure of the optimizer.
    pub fn fit(&self, settings: &Settings, threads: usize) -> Result<FitAnswer, PetabError> {
        let problem = self.problem(threads)?;
        let offset = self.cost_offset()?;
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

/// The offset of the row of a prior: twice its largest truncated log density between the
/// bounds, so the row is real wherever the parameter goes.
fn prior_offset(prior: &crate::Prior) -> Option<f64> {
    let p = &prior.parameters;
    let mode = match prior.kind {
        PriorKind::Normal | PriorKind::Laplace => p[0],
        PriorKind::LogNormal => (p[0] - p[1] * p[1]).exp(),
        PriorKind::LogLaplace => p[0].exp(),
        _ => prior.lower,
    };
    let mut candidates = vec![
        prior.lower,
        prior.upper,
        mode.clamp(prior.lower, prior.upper),
    ];
    if matches!(prior.kind, PriorKind::Uniform) {
        candidates.push(p[0].max(prior.lower));
    }
    let largest = candidates
        .into_iter()
        .filter(|x| x.is_finite())
        .map(|x| prior.log_density(x))
        .filter(|density| !density.is_nan())
        .fold(f64::NEG_INFINITY, f64::max);
    largest.is_finite().then(|| (2.0 * largest).max(0.0))
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
