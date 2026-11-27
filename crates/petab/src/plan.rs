//! The translation of a problem into simulations of the engine, and the likelihood of their
//! results.
//!
//! A problem of PEtab v2 maps onto the engine as follows:
//!
//! - **The models.** Every model of the problem is its SBML document with parameters added
//!   ([`PlannedModel::document`]): every parameter of the parameter table the model has not, as
//!   a constant parameter, and one parameter of an assignment rule per observable, per noise
//!   formula which reads more than numbers, per logarithm of an observable of a logarithmic
//!   noise and per value of a condition after the start. Their ids begin with `__petab_`, which
//!   no id of PEtab can. A species in such a rule is what PEtab reads it as, its amount for a
//!   species of `hasOnlySubstanceUnits` and its concentration otherwise, since that is what the
//!   math of SBML reads it as.
//! - **The simulations.** Every experiment of a model is one request of `simulation/1`
//!   ([`PlannedSimulation`]), and a measurement without an experiment is one of the model as
//!   it is, from the time 0. Its pre-initialization is the value of every parameter of the
//!   parameter table, the estimated ones at the values of an evaluation and the others at
//!   their nominal values, and the targets of the condition of the first period
//!   ([`InitialValue`]), whose formulas read parameters of the table alone: the engine skips
//!   the initial assignment of a value it pre-initializes, which is what PEtab asks. A first
//!   period at `-inf` is the steady state of the request before its start, which is the time
//!   of the next period, and a measurement at `inf` the steady state after its end.
//! - **The changes.** A condition of a period after the first is a change at its time. A value
//!   which is a number is set as one, every other value is a form of the engine which reads the
//!   assignment rule of the value in the state before the change, so it reads the state, the
//!   time and the parameters where the change happens, as PEtab evaluates a `targetValue`. A
//!   compartment keeps the concentration of a species of a concentration in it and the amount
//!   of a species of an amount, and an event the change triggers executes after it, both of
//!   them semantics of a change of the engine which are the ones of PEtab.
//! - **The measurements.** A measurement reads its observable in the row of its time, the
//!   state after a change at that time ([`DataAtChange::After`], PEtab), and its noise in the
//!   same row or as the number its formula is.

use std::collections::{BTreeMap, HashMap, HashSet};

use sbmlsim_abi::{
    Change, Form, IntegratorSettings, Output, SimulationRequest, SimulationResult, SteadyState,
    SteadyStateAfter, Tolerance,
};
use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_sbml::{Document, Parameter, Rule};

use crate::PetabError;
use crate::likelihood::{NoiseDistribution, Prior, PriorKind};
use crate::math::{self, Formula};
use crate::problem::PetabProblem;

/// The prefix of the ids of the parameters the plan adds to a model.
pub(crate) const PREFIX: &str = "__petab_";

/// Which state a measurement at the time of a change reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataAtChange {
    /// The state after the change, PEtab's.
    #[default]
    After,
    /// The state before a change after the start, except at the first time of an observable
    /// of an experiment which is measured at a later time as well, the `before` of the
    /// `sbmlsim` extension (`data_at_change` of its settings).
    Before,
}

/// What the plan does with the extension `sbmlsim` of a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExtensionPolicy {
    /// Read what it says about the simulation of the measurements: which state a measurement
    /// at a change reads (`settings.data_at_change`), the tolerances of the integrator
    /// (`settings.relative_tolerance`, `settings.absolute_tolerance`) and which observables are
    /// data of the likelihood (`observables.<id>.kind`: training data enter, validation data
    /// and outliers are simulated and do not; the validation data of a validation problem,
    /// `role: validation`, enter), and the scale a fit searches a parameter on
    /// (`parameters.<id>.scale`, else `settings.parameter_scale`, see [`ParameterScale`]). Its
    /// objective (the residuals, the weights and the loss of sbmlsim's least squares) is not
    /// read: the plan evaluates the likelihood of PEtab.
    #[default]
    Read,
    /// Ignore the extension, which reads the problem as a tool which does not know it.
    Ignore,
}

/// How a problem is planned.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The settings of the integrator; the ones of the extension `sbmlsim` or the defaults of
    /// the engine with a relative tolerance of `1e-8` when absent.
    pub integrator: Option<IntegratorSettings>,
    /// The criterion of a steady state; the defaults of the engine when absent.
    pub steady_state: Option<SteadyState>,
    /// Which state a measurement at a change reads, the one of the extension or of PEtab when
    /// absent.
    pub data_at_change: Option<DataAtChange>,
    /// What the plan does with the extension `sbmlsim`.
    pub extension: ExtensionPolicy,
    /// Whether a required extension the crate does not know is ignored instead of refused.
    pub ignore_unknown_extensions: bool,
}

/// The scale a fit searches a parameter on, which PEtab v2 does not hold and the extension
/// `sbmlsim` does: `LINEAR`, `LOG` or `LOG10` (the names of sbmlsim's `ParameterScaleType`,
/// in any case, and `lin` for `LINEAR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterScale {
    /// The value itself.
    Linear,
    /// The natural logarithm of the value.
    Log,
    /// The decadic logarithm of the value.
    Log10,
}

impl ParameterScale {
    /// The scale of a name of the extension.
    #[must_use]
    pub fn from_name(name: &str) -> Option<ParameterScale> {
        match name.trim().to_ascii_lowercase().as_str() {
            "linear" | "lin" => Some(ParameterScale::Linear),
            "log" | "ln" => Some(ParameterScale::Log),
            "log10" => Some(ParameterScale::Log10),
            _ => None,
        }
    }
}

/// A parameter of the parameter table.
#[derive(Debug, Clone)]
pub struct PlanParameter {
    /// Its id in the problem.
    pub id: String,
    /// The id of the entity of the models it sets, its id or the one of the mapping table.
    pub target: String,
    /// The lower bound.
    pub lower: f64,
    /// The upper bound.
    pub upper: f64,
    /// The nominal value, `NaN` for an estimated parameter without one.
    pub nominal: f64,
    /// Whether it is estimated.
    pub estimate: bool,
    /// The prior the table gives it.
    pub prior: Option<Prior>,
    /// The scale a fit searches it on, the one of the extension `sbmlsim`; `None` without
    /// one, for which the fit takes `log10` for positive finite bounds and the linear scale
    /// otherwise.
    pub scale: Option<ParameterScale>,
}

/// A model of the problem with the parameters the plan adds to it.
#[derive(Debug, Clone)]
pub struct PlannedModel {
    /// The id of the model in the problem.
    pub id: String,
    /// The SBML document with the parameters and rules of the plan.
    pub document: Document,
}

/// A value of the first period of an experiment, which follows the parameters of the table.
#[derive(Debug, Clone)]
pub struct InitialValue {
    /// The selection it is set on, `S` for the amount of a species, `[S]` for its
    /// concentration, the id of every other entity.
    pub target: String,
    /// The math of the value, over the targets of parameters of the table.
    pub node: Node,
    /// The math, compiled.
    pub formula: Formula,
    /// The parameter of the plan of every symbol of the formula, in its order.
    pub parameters: Vec<usize>,
}

/// One simulation of the plan: an experiment of a model.
#[derive(Debug, Clone)]
pub struct PlannedSimulation {
    /// The model, its index in [`Plan::models`].
    pub model: usize,
    /// The experiment, `None` for the model as it is.
    pub experiment: Option<String>,
    /// The request without the parameters and the values of the first period, which
    /// [`Plan::request`] adds.
    pub request: SimulationRequest,
    /// The values of the first period, which follow the parameters.
    pub initial: Vec<InitialValue>,
    /// The measurements of the simulation, their indices in [`Plan::measurements`].
    pub measurements: Vec<usize>,
}

/// The noise of a measurement.
#[derive(Debug, Clone, PartialEq)]
pub enum Noise {
    /// A number, the noise formula of a measurement which reads nothing of the simulation.
    Number(f64),
    /// The column of the simulation which is the noise.
    Column(String),
}

/// A measurement of the problem, as the plan reads it off its simulation.
#[derive(Debug, Clone)]
pub struct PlannedMeasurement {
    /// The simulation, its index in [`Plan::simulations`].
    pub simulation: usize,
    /// The id of the observable.
    pub observable: String,
    /// The experiment, `None` for the model as it is.
    pub experiment: Option<String>,
    /// The column of the simulated observable.
    pub column: String,
    /// The column of its natural logarithm, for a logarithmic noise.
    pub log_column: Option<String>,
    /// The noise.
    pub noise: Noise,
    /// The distribution of the noise.
    pub distribution: NoiseDistribution,
    /// The time.
    pub time: f64,
    /// The measured value.
    pub value: f64,
    /// Whether the measurement reads the state after a change at its time.
    pub after: bool,
    /// Whether the measurement is data of the likelihood.
    pub enters: bool,
}

/// What a problem is at the values of its parameters: the simulated observables and the
/// densities of PEtab.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Evaluation {
    /// The simulated value of every measurement, in the order of the measurement table, `NaN`
    /// where its simulation failed.
    pub simulated: Vec<f64>,
    /// The scale of the noise of every measurement.
    pub sigma: Vec<f64>,
    /// The log-likelihood of the measurements which are data of it.
    pub llh: f64,
    /// The sum of the squared residuals of those measurements in units of their noise.
    pub chi2: f64,
    /// The log prior of every estimated parameter, by its id.
    pub log_priors: Vec<(String, f64)>,
    /// The sum of the log priors.
    pub log_prior: f64,
    /// The failures of simulations, by the index of the simulation.
    pub failures: Vec<(usize, String)>,
}

impl Evaluation {
    /// The log-likelihood plus the log priors, PEtab's unnormalized log posterior.
    #[must_use]
    pub fn unnorm_log_posterior(&self) -> f64 {
        self.llh + self.log_prior
    }
}

/// A problem translated into simulations of the engine.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The parameters of the parameter table, in its order.
    pub parameters: Vec<PlanParameter>,
    /// The models.
    pub models: Vec<PlannedModel>,
    /// The simulations.
    pub simulations: Vec<PlannedSimulation>,
    /// The measurements, in the order of the measurement table.
    pub measurements: Vec<PlannedMeasurement>,
}

/// What an id of a model is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EntityKind {
    /// A species, of an amount or not.
    Species { amount: bool },
    /// A compartment.
    Compartment,
    /// A parameter.
    Parameter,
    /// Something else which math reads: a reaction or a species reference.
    Other,
}

/// The ids of a model by what they are.
fn entities(document: &Document) -> HashMap<String, EntityKind> {
    let model = &document.model;
    let mut out = HashMap::new();
    for compartment in &model.compartments {
        out.insert(compartment.id.clone(), EntityKind::Compartment);
    }
    for species in &model.species {
        out.insert(
            species.id.clone(),
            EntityKind::Species {
                amount: species.has_only_substance_units,
            },
        );
    }
    for parameter in &model.parameters {
        out.insert(parameter.id.clone(), EntityKind::Parameter);
    }
    for reaction in &model.reactions {
        out.insert(reaction.id.clone(), EntityKind::Other);
        for reference in reaction.reactants.iter().chain(&reaction.products) {
            if let Some(id) = &reference.id {
                out.insert(id.clone(), EntityKind::Other);
            }
        }
    }
    out
}

/// The extension `sbmlsim` as far as the plan reads it.
#[derive(Debug, Default)]
struct SbmlsimExtension {
    data_at_change: Option<DataAtChange>,
    relative_tolerance: Option<f64>,
    absolute_tolerance: Option<f64>,
    /// The observables which are no data of the likelihood.
    excluded: HashSet<String>,
    /// The scale of every parameter, `settings.parameter_scale`.
    parameter_scale: Option<ParameterScale>,
    /// The scales of the parameters which have one of their own, `parameters.<id>.scale`.
    scales: HashMap<String, ParameterScale>,
}

fn read_extension(content: &serde_json::Value) -> Result<SbmlsimExtension, PetabError> {
    let mut extension = SbmlsimExtension::default();
    let scale = |value: &serde_json::Value, of: &str| {
        let name = value.as_str().unwrap_or_default();
        ParameterScale::from_name(name).ok_or_else(|| {
            PetabError::Unsupported(format!(
                "the extension 'sbmlsim' gives {of} the scale {value}, which is none of \
                 'LINEAR', 'LOG' and 'LOG10'"
            ))
        })
    };
    let settings = content.get("settings");
    let setting = |name: &str| settings.and_then(|settings| settings.get(name));
    extension.data_at_change = match setting("data_at_change").and_then(|v| v.as_str()) {
        Some(value) if value.eq_ignore_ascii_case("before") => Some(DataAtChange::Before),
        Some(value) if value.eq_ignore_ascii_case("after") => Some(DataAtChange::After),
        _ => None,
    };
    extension.relative_tolerance = setting("relative_tolerance").and_then(|v| v.as_f64());
    if let Some(value) = setting("parameter_scale") {
        extension.parameter_scale = Some(scale(value, "every parameter")?);
    }
    if let Some(serde_json::Value::Object(parameters)) = content.get("parameters") {
        for (id, parameter) in parameters {
            if let Some(value) = parameter.get("scale") {
                extension
                    .scales
                    .insert(id.clone(), scale(value, &format!("'{id}'"))?);
            }
        }
    }
    extension.absolute_tolerance = match setting("absolute_tolerance") {
        Some(serde_json::Value::Number(number)) => number.as_f64(),
        // the tolerance of every kind of entity: the smallest of them is the one tolerance
        // of the engine
        Some(serde_json::Value::Object(kinds)) => ["amount", "concentration", "other"]
            .iter()
            .filter_map(|kind| kinds.get(*kind).and_then(serde_json::Value::as_f64))
            .reduce(f64::min),
        _ => None,
    };
    // the validation data of the validation problem of a fit (`role: validation`) is the data
    // of its likelihood, which the problem evaluates the fit on, as sbmlsim reads it
    let validation = content.get("role").and_then(|role| role.as_str()) == Some("validation");
    if let Some(serde_json::Value::Object(observables)) = content.get("observables") {
        for (id, observable) in observables {
            let kind = observable.get("kind").and_then(|kind| kind.as_str());
            let data = |kind: &str| kind == "training" || (validation && kind == "validation");
            if kind.is_some_and(|kind| !data(kind)) {
                extension.excluded.insert(id.clone());
            }
        }
    }
    Ok(extension)
}

/// The math of a cell of a table, with the aliases of the mapping table resolved.
fn formula(text: &str, aliases: &HashMap<String, String>) -> Result<Node, PetabError> {
    let node = math::parse(text)?;
    Ok(math::substitute(&node, &|symbol| {
        aliases.get(symbol).map(|id| Node::Symbol(id.clone()))
    }))
}

/// The parameters and rules the plan adds to one model, each rule once.
struct Additions {
    /// The ids the model has, the added ones included.
    taken: HashSet<String>,
    parameters: Vec<Parameter>,
    rules: Vec<Rule>,
    /// The id of the rule of a math, by the text of its math.
    by_math: HashMap<String, String>,
    counter: usize,
}

impl Additions {
    /// The id of the parameter whose assignment rule is the math, added once per math.
    fn rule(&mut self, kind: &str, node: Node) -> String {
        let key = format!("{kind}:{node:?}");
        if let Some(id) = self.by_math.get(&key) {
            return id.clone();
        }
        let id = loop {
            let id = format!("{PREFIX}{kind}_{}", self.counter);
            self.counter += 1;
            if !self.taken.contains(&id) {
                break id;
            }
        };
        self.taken.insert(id.clone());
        self.parameters.push(Parameter {
            id: id.clone(),
            value: None,
            units: None,
            constant: false,
        });
        self.rules.push(Rule::Assignment {
            id: None,
            variable: id.clone(),
            math: Some(node),
        });
        self.by_math.insert(key, id.clone());
        id
    }
}

/// The value of a placeholder of a measurement: a number or a parameter.
fn placeholder_value(text: &str, aliases: &HashMap<String, String>) -> Node {
    match crate::tables::number(text) {
        Some(value) => Node::number(value),
        None => Node::Symbol(
            aliases
                .get(text)
                .cloned()
                .unwrap_or_else(|| text.to_string()),
        ),
    }
}

/// The selection a target of a condition is set by: the amount of a species of an amount, the
/// concentration of every other species, the id of any other entity.
fn selection(id: &str, kind: EntityKind) -> String {
    match kind {
        EntityKind::Species { amount: false } => format!("[{id}]"),
        _ => id.to_string(),
    }
}

impl Plan {
    /// Translates a problem.
    ///
    /// # Errors
    ///
    /// A part of the problem which does not fit another one (a measurement of an observable or
    /// an experiment the problem has not, a placeholder without a value, a condition of a
    /// target which is no entity of the model or a parameter of the table), a formula which is
    /// no math of PEtab, a prior of PEtab with parameters outside of its domain, a required
    /// extension the crate does not know and a scale of the extension `sbmlsim` which is none
    /// of its scales.
    #[allow(clippy::too_many_lines)]
    pub fn new(problem: &PetabProblem, options: &Options) -> Result<Plan, PetabError> {
        let invalid = |message: String| PetabError::Invalid(message);

        // the extensions
        let mut extension = SbmlsimExtension::default();
        for (name, block) in &problem.extensions {
            if name == "sbmlsim" {
                if options.extension == ExtensionPolicy::Read {
                    extension = read_extension(&block.content)?;
                }
            } else if block.required && !options.ignore_unknown_extensions {
                return Err(PetabError::Unsupported(format!(
                    "the problem requires the extension '{name}' {}",
                    block.version
                )));
            }
        }
        let data_at_change = options
            .data_at_change
            .or(extension.data_at_change)
            .unwrap_or_default();
        let integrator = options.integrator.clone().unwrap_or_else(|| {
            let mut settings = IntegratorSettings {
                relative_tolerance: 1e-8,
                ..IntegratorSettings::default()
            };
            if let Some(relative) = extension.relative_tolerance {
                settings.relative_tolerance = relative;
            }
            if let Some(absolute) = extension.absolute_tolerance {
                settings.absolute_tolerance = Tolerance::Scalar(absolute);
            }
            settings
        });
        let steady = options.steady_state.clone().unwrap_or_default();

        // the aliases of the mapping table
        let mut aliases: HashMap<String, String> = HashMap::new();
        for mapping in &problem.mappings {
            if let Some(model) = &mapping.model
                && *model != mapping.petab
            {
                aliases.insert(mapping.petab.clone(), model.clone());
            }
        }
        let resolve = |id: &str| aliases.get(id).cloned().unwrap_or_else(|| id.to_string());

        // the parameters
        let mut parameters = Vec::with_capacity(problem.parameters.len());
        let mut by_target: HashMap<String, usize> = HashMap::new();
        for row in &problem.parameters {
            let prior = match &row.prior {
                None => None,
                Some(spec) => {
                    let kind = PriorKind::from_name(&spec.distribution).ok_or_else(|| {
                        PetabError::Prior(format!(
                            "'{}' of '{}' is no distribution of PEtab v2",
                            spec.distribution, row.id
                        ))
                    })?;
                    Some(
                        Prior::new(kind, spec.parameters.clone(), row.lower, row.upper)
                            .map_err(|err| PetabError::Prior(format!("of '{}': {err}", row.id)))?,
                    )
                }
            };
            let target = resolve(&row.id);
            if by_target.insert(target.clone(), parameters.len()).is_some() {
                return Err(invalid(format!(
                    "the parameter '{}' is twice in the parameter table",
                    row.id
                )));
            }
            parameters.push(PlanParameter {
                id: row.id.clone(),
                target,
                lower: row.lower,
                upper: row.upper,
                nominal: row.nominal.unwrap_or(f64::NAN),
                estimate: row.estimate,
                prior,
                scale: extension
                    .scales
                    .get(&row.id)
                    .copied()
                    .or(extension.parameter_scale),
            });
        }

        // the models, which their measurements name
        let model_index: HashMap<&str, usize> = problem
            .models
            .iter()
            .enumerate()
            .map(|(k, model)| (model.id.as_str(), k))
            .collect();
        let mut documents = Vec::with_capacity(problem.models.len());
        let mut kinds = Vec::with_capacity(problem.models.len());
        let mut additions = Vec::with_capacity(problem.models.len());
        for model in &problem.models {
            let document = sbmlsim_sbml::read(&model.sbml).map_err(|err| PetabError::Compile {
                model: model.id.clone(),
                message: err.to_string(),
            })?;
            let mut known = entities(&document);
            let taken: HashSet<String> = known.keys().cloned().collect();
            if let Some(id) = taken.iter().find(|id| id.starts_with(PREFIX)) {
                return Err(PetabError::Unsupported(format!(
                    "the model '{}' has the id '{id}', which begins with the prefix of the ids the plan adds",
                    model.id
                )));
            }
            let mut added = Additions {
                taken,
                parameters: Vec::new(),
                rules: Vec::new(),
                by_math: HashMap::new(),
                counter: 0,
            };
            // a parameter of the table the model has not is a parameter of the model, so that
            // the math of the problem reads it as any other
            for parameter in &parameters {
                if !known.contains_key(&parameter.target) {
                    let value = if parameter.nominal.is_nan() {
                        0.5 * (parameter.lower + parameter.upper)
                    } else {
                        parameter.nominal
                    };
                    added.parameters.push(Parameter {
                        id: parameter.target.clone(),
                        value: Some(if value.is_finite() { value } else { 0.0 }),
                        units: None,
                        constant: true,
                    });
                    added.taken.insert(parameter.target.clone());
                    known.insert(parameter.target.clone(), EntityKind::Parameter);
                }
            }
            documents.push(document);
            kinds.push(known);
            additions.push(added);
        }

        // the conditions and the periods of the experiments
        let mut conditions: HashMap<&str, Vec<(String, &str)>> = HashMap::new();
        for change in &problem.conditions {
            conditions
                .entry(change.condition.as_str())
                .or_default()
                .push((resolve(&change.target), change.value.as_str()));
        }
        let mut experiments: BTreeMap<&str, BTreeMap<OrderedTime, Vec<&str>>> = BTreeMap::new();
        for period in &problem.periods {
            let periods = experiments.entry(period.experiment.as_str()).or_default();
            let conditions_at = periods.entry(OrderedTime(period.time)).or_default();
            if let Some(condition) = &period.condition {
                conditions_at.push(condition.as_str());
            }
        }
        let observables: HashMap<&str, &crate::problem::Observable> = problem
            .observables
            .iter()
            .map(|observable| (observable.id.as_str(), observable))
            .collect();

        // the simulations, one per model and experiment of a measurement, in the order of
        // their first measurement
        let mut simulation_of: HashMap<(usize, Option<&str>), usize> = HashMap::new();
        let mut keys: Vec<(usize, Option<&str>)> = Vec::new();
        let mut measurement_simulation = Vec::with_capacity(problem.measurements.len());
        for (row, measurement) in problem.measurements.iter().enumerate() {
            let model = match &measurement.model {
                Some(id) => *model_index.get(id.as_str()).ok_or_else(|| {
                    invalid(format!(
                        "the measurement {} names the model '{id}', which the problem has not",
                        row + 1
                    ))
                })?,
                None if problem.models.len() == 1 => 0,
                None => {
                    return Err(invalid(format!(
                        "the measurement {} names no model of a problem of several",
                        row + 1
                    )));
                }
            };
            let key = (model, measurement.experiment.as_deref());
            let next = keys.len();
            let simulation = *simulation_of.entry(key).or_insert_with(|| {
                keys.push(key);
                next
            });
            measurement_simulation.push(simulation);
        }

        // the measurements, whose observables and noise become rules of their model
        let mut measurements = Vec::with_capacity(problem.measurements.len());
        for (row, measurement) in problem.measurements.iter().enumerate() {
            let simulation = measurement_simulation[row];
            let model = keys[simulation].0;
            let observable = observables
                .get(measurement.observable.as_str())
                .ok_or_else(|| {
                    invalid(format!(
                        "the measurement {} is of the observable '{}', which the problem has not",
                        row + 1,
                        measurement.observable
                    ))
                })?;
            let fault = |message: String| {
                invalid(format!(
                    "the measurement {} of '{}': {message}",
                    row + 1,
                    observable.id
                ))
            };
            if observable.placeholders.len() != measurement.observable_parameters.len() {
                return Err(fault(format!(
                    "the observable has {} placeholders and the measurement {} observableParameters",
                    observable.placeholders.len(),
                    measurement.observable_parameters.len()
                )));
            }
            if observable.noise_placeholders.len() != measurement.noise_parameters.len() {
                return Err(fault(format!(
                    "the noise has {} placeholders and the measurement {} noiseParameters",
                    observable.noise_placeholders.len(),
                    measurement.noise_parameters.len()
                )));
            }
            let mut values: HashMap<&str, Node> = HashMap::new();
            for (name, value) in observable
                .placeholders
                .iter()
                .zip(&measurement.observable_parameters)
                .chain(
                    observable
                        .noise_placeholders
                        .iter()
                        .zip(&measurement.noise_parameters),
                )
            {
                values.insert(name.as_str(), placeholder_value(value, &aliases));
            }
            let known = &kinds[model];
            let check = |node: &Node, what: &str, extra: &str| -> Result<(), PetabError> {
                for symbol in math::symbols(node) {
                    if !known.contains_key(&symbol) && symbol != extra {
                        return Err(fault(format!(
                            "the {what} reads '{symbol}', which is no entity of the model '{}', \
                             no parameter of the table and no placeholder",
                            problem.models[model].id
                        )));
                    }
                }
                Ok(())
            };
            let placeholders =
                |node: &Node| math::substitute(node, &|symbol| values.get(symbol).cloned());
            let observed = placeholders(&formula(&observable.formula, &aliases)?);
            check(&observed, "observable", "")?;
            let added = &mut additions[model];
            let column = added.rule("observable", observed);
            let noise_node = math::substitute(
                &placeholders(&formula(&observable.noise_formula, &aliases)?),
                &|symbol| (symbol == observable.id).then(|| Node::Symbol(column.clone())),
            );
            check(&noise_node, "noise", &column)?;
            let noise = if math::symbols(&noise_node).is_empty() && !math::reads_time(&noise_node) {
                Noise::Number(
                    Formula::new(&noise_node)
                        .map_err(|err| fault(format!("the noise formula: {err}")))?
                        .evaluate(&[], 0.0),
                )
            } else {
                Noise::Column(added.rule("noise", noise_node))
            };
            let log_column = observable.distribution.logarithmic().then(|| {
                added.rule(
                    "log",
                    Node::Apply {
                        op: Op::Ln,
                        args: vec![Node::Symbol(column.clone())],
                    },
                )
            });
            measurements.push(PlannedMeasurement {
                simulation,
                observable: observable.id.clone(),
                experiment: measurement.experiment.clone(),
                column,
                log_column,
                noise,
                distribution: observable.distribution,
                time: measurement.time,
                value: measurement.value,
                after: true,
                enters: !extension.excluded.contains(&observable.id),
            });
        }
        if data_at_change == DataAtChange::Before {
            before_changes(&mut measurements);
        }

        // the requests
        let mut simulations = Vec::with_capacity(keys.len());
        for (simulation, &(model, experiment)) in keys.iter().enumerate() {
            let known = &kinds[model];
            let periods: Vec<(f64, Vec<(String, Node)>)> = match experiment {
                None => Vec::new(),
                Some(id) => experiments
                    .get(id)
                    .ok_or_else(|| {
                        invalid(format!(
                            "measurements name the experiment '{id}', which the problem has not"
                        ))
                    })?
                    .iter()
                    .map(|(time, ids)| {
                        let mut changes: Vec<(String, Node)> = Vec::new();
                        for condition in ids {
                            let rows = conditions.get(condition).ok_or_else(|| {
                                invalid(format!(
                                    "the experiment '{id}' applies the condition '{condition}', \
                                     which the problem has not"
                                ))
                            })?;
                            for (target, value) in rows {
                                if changes.iter().any(|(known, _)| known == target) {
                                    return Err(invalid(format!(
                                        "the experiment '{id}' changes '{target}' twice at the time {}",
                                        time.0
                                    )));
                                }
                                changes.push((target.clone(), formula(value, &aliases)?));
                            }
                        }
                        Ok((time.0, changes))
                    })
                    .collect::<Result<_, PetabError>>()?,
            };
            let name = experiment.unwrap_or("(the model as it is)");
            for (_, changes) in &periods {
                for (target, node) in changes {
                    match known.get(target) {
                        None => {
                            return Err(invalid(format!(
                                "the experiment '{name}' changes '{target}', which is no entity of \
                                 the model '{}'",
                                problem.models[model].id
                            )));
                        }
                        Some(_) if by_target.contains_key(target) => {
                            return Err(invalid(format!(
                                "the experiment '{name}' changes '{target}', which is a parameter of \
                                 the parameter table"
                            )));
                        }
                        Some(_) => {}
                    }
                    for symbol in math::symbols(node) {
                        if !known.contains_key(&symbol) {
                            return Err(invalid(format!(
                                "the value of '{target}' in the experiment '{name}' reads '{symbol}', \
                                 which is no entity of the model and no parameter of the table"
                            )));
                        }
                    }
                }
            }

            let steady_first = periods
                .first()
                .is_some_and(|(time, _)| *time == f64::NEG_INFINITY);
            let start = periods
                .iter()
                .map(|(time, _)| *time)
                .find(|time| time.is_finite())
                .unwrap_or(0.0);
            let own: Vec<usize> = measurements
                .iter()
                .enumerate()
                .filter(|(_, m)| m.simulation == simulation)
                .map(|(k, _)| k)
                .collect();
            let mut times: Vec<f64> = own
                .iter()
                .map(|&k| measurements[k].time)
                .filter(|time| time.is_finite())
                .collect();
            if let Some(early) = times.iter().find(|time| **time < start) {
                return Err(invalid(format!(
                    "the experiment '{name}' has a measurement at {early}, before its start at {start}"
                )));
            }
            times.sort_by(f64::total_cmp);
            times.dedup();
            let at_steady_state = own.iter().any(|&k| measurements[k].time.is_infinite());
            let mut end = times.last().copied().unwrap_or(start);
            if at_steady_state {
                // the steady state of `inf` is the one of the last period
                end = periods.iter().map(|(time, _)| *time).fold(end, f64::max);
            }
            if times.is_empty() {
                times.push(start);
            }

            // the first period: before the steady state of `-inf`, or at the start
            let mut initial = Vec::new();
            let mut changes: Vec<Change> = Vec::new();
            let mut first_at_start = None;
            let mut later = periods.iter();
            if let Some((time, first)) = periods.first()
                && (steady_first || *time == start)
            {
                later.next();
                for (target, node) in first {
                    let symbols = math::symbols(node);
                    let reads_parameters = symbols.iter().all(|s| by_target.contains_key(s))
                        && !math::reads_time(node);
                    if reads_parameters {
                        let formula = Formula::new(node).map_err(|err| {
                            invalid(format!("the value of '{target}' in '{name}': {err}"))
                        })?;
                        initial.push(InitialValue {
                            target: selection(target, known[target]),
                            node: node.clone(),
                            parameters: symbols.iter().map(|s| by_target[s]).collect(),
                            formula,
                        });
                    } else {
                        // a value which reads the model is set after its initialization,
                        // a change at the start
                        first_at_start
                            .get_or_insert_with(Vec::new)
                            .push((target.clone(), node.clone()));
                    }
                }
            }
            let mut apply = |time: f64, values: &[(String, Node)], added: &mut Additions| {
                let mut change = Change {
                    time,
                    ..Change::default()
                };
                for (target, node) in values {
                    let target_selection = selection(target, known[target]);
                    match math::number(node) {
                        Some(value) => change.values.push((target_selection, value)),
                        None => {
                            let rule = added.rule("value", node.clone());
                            change.forms.push(Form {
                                target: target_selection,
                                constant: 0.0,
                                terms: vec![(rule, 1.0)],
                            });
                        }
                    }
                }
                changes.push(change);
            };
            let added = &mut additions[model];
            if let Some(values) = &first_at_start {
                apply(start, values, added);
            }
            for (time, values) in later {
                if *time > end || values.is_empty() {
                    continue;
                }
                apply(*time, values, added);
            }
            // a change at the start after the values of a first period which read the model
            changes.sort_by(|a, b| a.time.total_cmp(&b.time));
            let mut merged: Vec<Change> = Vec::new();
            for change in changes {
                match merged.last_mut() {
                    Some(last) if last.time == change.time => {
                        last.values.extend(change.values);
                        last.forms.extend(change.forms);
                    }
                    _ => merged.push(change),
                }
            }

            let mut selections = vec!["time".to_string()];
            for &k in &own {
                let m = &measurements[k];
                for column in [Some(&m.column), m.log_column.as_ref()]
                    .into_iter()
                    .flatten()
                    .chain(match &m.noise {
                        Noise::Column(column) => Some(column),
                        Noise::Number(_) => None,
                    })
                {
                    if !selections.contains(column) {
                        selections.push(column.clone());
                    }
                }
            }
            let request = SimulationRequest {
                contract: sbmlsim_abi::SIMULATION_CONTRACT.to_string(),
                start,
                end,
                output: Output::Times(times),
                preinit: Vec::new(),
                changes: merged,
                steady_state: steady_first.then(|| SteadyState {
                    preinit: Vec::new(),
                    ..steady.clone()
                }),
                steady_state_after: at_steady_state.then_some(SteadyStateAfter {
                    from: end,
                    absolute_tolerance: steady.absolute_tolerance,
                    relative_tolerance: steady.relative_tolerance,
                    max_time: steady.max_time,
                }),
                before_events: false,
                selections,
                settings: integrator.clone(),
                seed: sbmlsim_abi::DEFAULT_SEED,
                sensitivities: Vec::new(),
                local_errors: false,
            };
            simulations.push(PlannedSimulation {
                model,
                experiment: experiment.map(str::to_string),
                request,
                initial,
                measurements: own,
            });
        }

        let models = problem
            .models
            .iter()
            .zip(documents)
            .zip(additions)
            .map(|((model, mut document), added)| {
                document.model.parameters.extend(added.parameters);
                document.model.rules.extend(added.rules);
                PlannedModel {
                    id: model.id.clone(),
                    document,
                }
            })
            .collect();
        Ok(Plan {
            parameters,
            models,
            simulations,
            measurements,
        })
    }

    /// Compiles a model of the plan, its index in [`Plan::models`], with the parameters and
    /// rules the plan added to it.
    ///
    /// # Errors
    ///
    /// The refusal of the compiler, e.g. an observable which reads the concentration of a
    /// species in a compartment of no size.
    pub fn compile(&self, model: usize) -> Result<sbmlsim_compiler::CompiledModel, PetabError> {
        let planned = &self.models[model];
        sbmlsim_compiler::compile_document(
            &planned.document,
            &sbmlsim_compiler::CompileOptions::default(),
        )
        .map_err(|err| PetabError::Compile {
            model: planned.id.clone(),
            message: err.to_string(),
        })
    }

    /// The indices of the estimated parameters in [`Plan::parameters`], the order of the
    /// vector of an evaluation and of a fit.
    #[must_use]
    pub fn estimated(&self) -> Vec<usize> {
        (0..self.parameters.len())
            .filter(|&k| self.parameters[k].estimate)
            .collect()
    }

    /// The nominal values of the estimated parameters, where PEtab evaluates a problem.
    #[must_use]
    pub fn nominal(&self) -> Vec<f64> {
        self.estimated()
            .into_iter()
            .map(|k| self.parameters[k].nominal)
            .collect()
    }

    /// The value of every parameter of the table: the estimated ones at `x`, in the order of
    /// [`Plan::estimated`], and the others at their nominal values.
    ///
    /// # Panics
    ///
    /// For an `x` which is not one value per estimated parameter.
    #[must_use]
    pub fn values(&self, x: &[f64]) -> Vec<f64> {
        let estimated = self.estimated();
        assert_eq!(
            x.len(),
            estimated.len(),
            "one value per estimated parameter"
        );
        let mut values: Vec<f64> = self.parameters.iter().map(|p| p.nominal).collect();
        for (value, k) in x.iter().zip(estimated) {
            values[k] = *value;
        }
        values
    }

    /// The request of a simulation at the values of the estimated parameters.
    ///
    /// # Panics
    ///
    /// For a simulation the plan has not or an `x` which is not one value per estimated
    /// parameter.
    #[must_use]
    pub fn request(&self, simulation: usize, x: &[f64]) -> SimulationRequest {
        let values = self.values(x);
        let planned = &self.simulations[simulation];
        let mut request = planned.request.clone();
        request.preinit = self
            .parameters
            .iter()
            .zip(&values)
            .map(|(parameter, value)| (parameter.target.clone(), *value))
            .collect();
        for value in &planned.initial {
            let read: Vec<f64> = value.parameters.iter().map(|&k| values[k]).collect();
            request.preinit.push((
                value.target.clone(),
                value.formula.evaluate(&read, request.start),
            ));
        }
        request
    }

    /// The requests of every simulation at the values of the estimated parameters.
    #[must_use]
    pub fn requests(&self, x: &[f64]) -> Vec<SimulationRequest> {
        (0..self.simulations.len())
            .map(|simulation| self.request(simulation, x))
            .collect()
    }

    /// Whether an estimated parameter has a prior, which makes the objective of PEtab the
    /// negative log posterior, with the uniform priors of the others, instead of the negative
    /// log-likelihood.
    #[must_use]
    pub fn has_priors(&self) -> bool {
        self.parameters
            .iter()
            .any(|parameter| parameter.estimate && parameter.prior.is_some())
    }

    /// The log prior of every estimated parameter at its value: the one of its prior truncated
    /// at its bounds, and the uniform distribution between the bounds for a parameter without
    /// one, as PEtab defaults it.
    #[must_use]
    pub fn log_priors(&self, x: &[f64]) -> Vec<(String, f64)> {
        self.estimated()
            .into_iter()
            .zip(x)
            .map(|(k, &value)| {
                let parameter = &self.parameters[k];
                let density = match &parameter.prior {
                    Some(prior) => prior.log_density(value),
                    None if value < parameter.lower || value > parameter.upper => f64::NEG_INFINITY,
                    None => -(parameter.upper - parameter.lower).ln(),
                };
                (parameter.id.clone(), density)
            })
            .collect()
    }

    /// The simulation table of PEtab of an evaluation: a row per measurement, in their order,
    /// with its simulated value in the column `simulation`, and the model of a problem of
    /// several models.
    #[must_use]
    pub fn simulation_table(&self, evaluation: &Evaluation) -> String {
        let models = self.models.len() > 1;
        let mut table = String::from(if models {
            "modelId\tobservableId\texperimentId\ttime\tsimulation\n"
        } else {
            "observableId\texperimentId\ttime\tsimulation\n"
        });
        for (m, simulated) in self.measurements.iter().zip(&evaluation.simulated) {
            if models {
                table.push_str(&self.models[self.simulations[m.simulation].model].id);
                table.push('\t');
            }
            table.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                m.observable,
                m.experiment.as_deref().unwrap_or(""),
                m.time,
                simulated
            ));
        }
        table
    }

    /// The evaluation of the problem from the results of its simulations at the values of the
    /// estimated parameters, the results in the order of [`Plan::simulations`].
    ///
    /// A failed simulation gives `NaN` for every measurement of it, and so a log-likelihood of
    /// `NaN`, which PEtab asks of a problem it cannot evaluate.
    ///
    /// # Panics
    ///
    /// For results which are not one per simulation.
    #[must_use]
    pub fn evaluate(&self, x: &[f64], results: &[Result<SimulationResult, String>]) -> Evaluation {
        assert_eq!(
            results.len(),
            self.simulations.len(),
            "one result per simulation"
        );
        let n = self.measurements.len();
        let mut simulated = vec![f64::NAN; n];
        let mut sigma = vec![f64::NAN; n];
        let mut failures = Vec::new();
        for (index, (planned, result)) in self.simulations.iter().zip(results).enumerate() {
            let table = match result {
                Ok(table) => table,
                Err(message) => {
                    failures.push((index, message.clone()));
                    continue;
                }
            };
            let width = table.columns.len();
            let times: Vec<f64> = (0..table.n_rows)
                .map(|row| table.values[row * width])
                .collect();
            let column = |name: &str| table.columns.iter().position(|c| c == name);
            for &k in &planned.measurements {
                let m = &self.measurements[k];
                let Some(row) = row_of(m.time, &times, m.after) else {
                    failures.push((
                        index,
                        format!(
                            "no row of the time {} of the observable '{}'",
                            m.time, m.observable
                        ),
                    ));
                    continue;
                };
                let read =
                    |name: &str| column(name).map_or(f64::NAN, |c| table.values[row * width + c]);
                simulated[k] = read(&m.column);
                sigma[k] = match &m.noise {
                    Noise::Number(value) => *value,
                    Noise::Column(name) => read(name),
                };
            }
        }
        let mut llh = 0.0;
        let mut chi2 = 0.0;
        for (k, m) in self.measurements.iter().enumerate() {
            if !m.enters {
                continue;
            }
            llh += m.distribution.log_density(m.value, simulated[k], sigma[k]);
            let r = m.distribution.residual(m.value, simulated[k], sigma[k]);
            chi2 += r * r;
        }
        let log_priors = self.log_priors(x);
        let log_prior = log_priors.iter().map(|(_, value)| value).sum();
        Evaluation {
            simulated,
            sigma,
            llh,
            chi2,
            log_priors,
            log_prior,
            failures,
        }
    }
}

/// The measurements of `before`: a measurement at a change after the start reads the state
/// before it, except one at the first time of the measurements of its observable in its
/// simulation which are measured at a later time as well, the start of a collection.
fn before_changes(measurements: &mut [PlannedMeasurement]) {
    let mut first: HashMap<(usize, String), (f64, bool)> = HashMap::new();
    for m in measurements.iter() {
        let entry = first
            .entry((m.simulation, m.observable.clone()))
            .or_insert((m.time, false));
        if m.time < entry.0 {
            *entry = (m.time, true);
        } else if m.time > entry.0 {
            entry.1 = true;
        }
    }
    for m in measurements.iter_mut() {
        let (time, later) = first[&(m.simulation, m.observable.clone())];
        m.after = later && m.time == time;
    }
}

/// The row of an output time, the nearest one within the resolution of the time; `after` reads
/// the last of several rows of that time, the state after a change.
pub(crate) fn row_of(time: f64, times: &[f64], after: bool) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (row, output) in times.iter().enumerate() {
        let distance = if *output == time {
            0.0
        } else {
            (output - time).abs()
        };
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
    let tolerance = 1e-9 * time.abs().max(1.0);
    (distance <= tolerance).then_some(row)
}

/// A time of a period, ordered.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedTime(f64);

impl Eq for OrderedTime {}

impl PartialOrd for OrderedTime {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedTime {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}
