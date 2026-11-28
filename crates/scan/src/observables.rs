//! The observables of a run, compiled against the model and evaluated on the native output of
//! every point, sbmlsim's `simulator/observables.py`.
//!
//! [`compile`] orders the observables by what they read, checks every symbol against the
//! model, derives the kind (a timecourse or a value per simulation) and the unit of every
//! observable, and prunes them to what `keep` needs: a [`Graph`]. A run without observables
//! has the graph of the selections of the model, each a timecourse of its own name.
//!
//! An [`Observation`] evaluates the graph on the points of a chunk which ran, point by point:
//! a `Formula` with the operations of the engine (`sbmlsim_petab::math`) and its reductions
//! over the time of the point, a `PK` with the kernel of pkpdutils on the points of the chunk,
//! a `Custom` by the host in a round of its own ([`crate::Ask::Observe`]). A point fails when
//! its own evaluation fails, as in sbmlsim, which evaluates a chunk point by point after a
//! failure.

use std::collections::{BTreeMap, HashMap};

use sbmlsim_units::Registry;

use crate::derive::{Units, at_times, quantity_of, value_at_ones};
use crate::error::ScanError;
use crate::host_observables::{HostObservables, HostObserved, HostPoint};
use crate::model::ModelInfo;
use crate::pk::{self, DoseSpec, PkNode, PkPoint};
use crate::plan::{Plan, time_tolerance};
use crate::reduce::{self, Reduced, Times, Val};
use crate::request::{Dose, Observable};
use crate::text;

/// The time.
const TIME: &str = "time";

/// What an observable is per simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A value per simulation.
    Scalar,
    /// A value per output point.
    Timecourse,
}

/// An observable compiled against the model.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    /// A formula, its values multiplied by the factor from its derived unit into its declared
    /// one.
    Formula {
        id: String,
        reduced: Reduced,
        kind: Kind,
        factor: f64,
    },
    /// A function of the host.
    Custom {
        id: String,
        kind: Kind,
        symbols: Vec<String>,
    },
    /// The non-compartmental analysis of a timecourse.
    Pk(Box<PkNode>),
}

/// The observables of a run, see the module.
#[derive(Debug, Clone)]
pub(crate) struct Graph {
    /// The observables the kept ones need, in the order of what they read.
    pub nodes: Vec<Node>,
    /// The selections the requests select, without `time`.
    pub selections: Vec<String>,
    /// The kind of every symbol and output.
    pub kinds: HashMap<String, Kind>,
    /// The unit of every symbol and output.
    pub units: HashMap<String, String>,
    /// The outputs of the result, in order.
    pub keep: Vec<String>,
    /// Every output.
    pub outputs: Vec<String>,
}

impl Graph {
    /// The kept timecourses.
    pub fn timecourses(&self) -> Vec<String> {
        self.keep
            .iter()
            .filter(|k| self.kinds[*k] == Kind::Timecourse)
            .cloned()
            .collect()
    }

    /// The kept values per simulation.
    pub fn scalars(&self) -> Vec<String> {
        self.keep
            .iter()
            .filter(|k| self.kinds[*k] == Kind::Scalar)
            .cloned()
            .collect()
    }

    /// Whether the run evaluates observables.
    pub fn observes(&self) -> bool {
        !self.nodes.is_empty()
    }

    /// The dose targets of the PK observables the run evaluates, each once, sbmlsim's
    /// `ObservableGraph.doses`.
    pub fn doses(&self) -> Vec<String> {
        let mut doses: Vec<String> = Vec::new();
        for node in &self.nodes {
            if let Node::Pk(pk) = node
                && let Some(target) = pk.dose.as_ref().and_then(|d| d.target.as_ref())
                && !doses.contains(target)
            {
                doses.push(target.clone());
            }
        }
        doses
    }
}

/// The unit of a symbol of the model, `""` without one.
fn model_unit(model: &ModelInfo, symbol: &str) -> String {
    if symbol == TIME {
        return model.time_unit_text().to_string();
    }
    model.unit(symbol).unwrap_or("").to_string()
}

/// The graph of a run without observables: the kept selections, each a timecourse of its own
/// name, sbmlsim's `identity_graph`.
fn identity(selections: &[String], model: &ModelInfo) -> Graph {
    let names: Vec<String> = selections.iter().filter(|s| *s != TIME).cloned().collect();
    let mut kinds = HashMap::from([(TIME.to_string(), Kind::Timecourse)]);
    let mut units = HashMap::from([(TIME.to_string(), model_unit(model, TIME))]);
    for name in &names {
        kinds.insert(name.clone(), Kind::Timecourse);
        units.insert(name.clone(), model_unit(model, name));
    }
    Graph {
        nodes: Vec::new(),
        selections: names.clone(),
        kinds,
        units,
        keep: names.clone(),
        outputs: names,
    }
}

/// The symbols an observable reads.
fn reads(observable: &Observable) -> Result<Vec<String>, ScanError> {
    Ok(match observable {
        Observable::Formula { formula, .. } => {
            Reduced::new(formula).map_err(ScanError::request)?.symbols()
        }
        Observable::Pk { selection, .. } => vec![selection.clone()],
        Observable::Custom { symbols, .. } => symbols.clone(),
    })
}

/// The observable a symbol is an output of, `None` for a selection.
fn owner<'a>(symbol: &str, definitions: &'a [(String, Observable)]) -> Option<&'a str> {
    if let Some((id, _)) = definitions.iter().find(|(id, _)| id == symbol) {
        return Some(id);
    }
    let (head, _) = symbol.split_once('.')?;
    definitions
        .iter()
        .find(|(id, o)| id == head && matches!(o, Observable::Pk { .. }))
        .map(|(id, _)| id.as_str())
}

/// The observables in an order in which every one comes after what it reads.
fn order(
    definitions: &[(String, Observable)],
    read: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, ScanError> {
    fn visit(
        name: &str,
        path: &mut Vec<String>,
        state: &mut HashMap<String, bool>,
        order: &mut Vec<String>,
        definitions: &[(String, Observable)],
        read: &HashMap<String, Vec<String>>,
    ) -> Result<(), ScanError> {
        match state.get(name) {
            Some(true) => return Ok(()),
            Some(false) => {
                let at = path.iter().position(|p| p == name).unwrap_or(0);
                let mut cycle: Vec<String> = path[at..].to_vec();
                cycle.push(name.to_string());
                return Err(ScanError::request(format!(
                    "The observables read each other in a cycle: {}.",
                    cycle.join(" -> ")
                )));
            }
            None => {}
        }
        state.insert(name.to_string(), false);
        path.push(name.to_string());
        for symbol in &read[name] {
            if let Some(owner) = owner(symbol, definitions) {
                visit(owner, path, state, order, definitions, read)?;
            }
        }
        path.pop();
        state.insert(name.to_string(), true);
        order.push(name.to_string());
        Ok(())
    }
    let mut out = Vec::new();
    let mut state = HashMap::new();
    for (name, _) in definitions {
        visit(
            name,
            &mut Vec::new(),
            &mut state,
            &mut out,
            definitions,
            read,
        )?;
    }
    Ok(out)
}

/// Compiles the observables of a run, see the module.
///
/// `plans` are the plans of the first point of every simulation, whose dosing decides the
/// parameters of the PK observables.
///
/// # Errors
///
/// The errors of sbmlsim's `compile_observables`: two observables of one id, an id which is a
/// selection, a symbol which is neither, a cycle, the time of `at` of a timecourse or another
/// unit, a formula which mixes scales, a unit which cannot be derived or converted, a PK
/// observable which does not fit the model, a `keep` which names no output.
pub(crate) fn compile(
    observables: &[Observable],
    selections: &[String],
    keep: Option<&[String]>,
    model: &ModelInfo,
    plans: &[Plan],
) -> Result<Graph, ScanError> {
    if observables.is_empty() {
        let kept = crate::requests::run_selections(selections, keep)?;
        return Ok(identity(&kept, model));
    }
    let mut definitions: Vec<(String, Observable)> = Vec::new();
    for observable in observables {
        let id = observable.id().to_string();
        if definitions.iter().any(|(d, _)| *d == id) {
            return Err(ScanError::request(format!(
                "Two observables have the id '{id}'."
            )));
        }
        definitions.push((id, observable.clone()));
    }
    let mut clash: Vec<&str> = definitions
        .iter()
        .map(|(id, _)| id.as_str())
        .filter(|id| model.has_selection(id))
        .collect();
    clash.sort_unstable();
    if !clash.is_empty() {
        return Err(ScanError::request(format!(
            "The observable ids {} are selections of the model: an observable and a selection \
             share no name, choose other ids.",
            text::list(&clash)
        )));
    }
    let mut read: HashMap<String, Vec<String>> = HashMap::new();
    for (id, observable) in &definitions {
        read.insert(id.clone(), reads(observable)?);
    }
    let registry = model.registry();
    let mut kinds: HashMap<String, Kind> = HashMap::from([(TIME.to_string(), Kind::Timecourse)]);
    let mut units: HashMap<String, String> =
        HashMap::from([(TIME.to_string(), model_unit(model, TIME))]);
    let mut resolved: Vec<String> = Vec::new();
    let mut compiled: Vec<(String, Node)> = Vec::new();
    let mut outputs: HashMap<String, Vec<String>> = HashMap::new();
    for name in order(&definitions, &read)? {
        let observable = &definitions
            .iter()
            .find(|(id, _)| *id == name)
            .expect("an observable of the run")
            .1;
        for symbol in &read[&name] {
            if kinds.contains_key(symbol) {
                continue;
            }
            if let Some(owner) = owner(symbol, &definitions) {
                return Err(ScanError::request(format!(
                    "The observable '{name}' reads '{symbol}', which the PK observable \
                     '{owner}' does not give: {}.",
                    text::list(outputs.get(owner).map_or(&[][..], Vec::as_slice))
                )));
            }
            if !model.has_selection(symbol) {
                return Err(ScanError::request(format!(
                    "The observable '{name}' reads '{symbol}', which is neither an observable \
                     nor a selection of the model."
                )));
            }
            kinds.insert(symbol.clone(), Kind::Timecourse);
            units.insert(symbol.clone(), model_unit(model, symbol));
            resolved.push(symbol.clone());
        }
        let node = match observable {
            Observable::Formula { formula, unit, .. } => {
                let (node, unit) =
                    compile_formula(&name, formula, unit.as_deref(), &kinds, &units, registry)?;
                let Node::Formula { kind, .. } = &node else {
                    unreachable!()
                };
                kinds.insert(name.clone(), *kind);
                units.insert(name.clone(), unit);
                outputs.insert(name.clone(), vec![name.clone()]);
                node
            }
            Observable::Custom {
                unit,
                symbols,
                kind,
                ..
            } => {
                let kind = match kind.as_str() {
                    "scalar" => Kind::Scalar,
                    "timecourse" => Kind::Timecourse,
                    other => {
                        return Err(ScanError::request(format!(
                            "The kind of the observable '{name}' is 'scalar' or 'timecourse', \
                             not '{other}'."
                        )));
                    }
                };
                let unit = registry
                    .named(unit)
                    .map_err(|err| {
                        ScanError::units(format!("The unit of the observable '{name}': {err}"))
                    })?
                    .to_string();
                kinds.insert(name.clone(), kind);
                units.insert(name.clone(), unit);
                outputs.insert(name.clone(), vec![name.clone()]);
                Node::Custom {
                    id: name.clone(),
                    kind,
                    symbols: symbols.clone(),
                }
            }
            Observable::Pk {
                selection,
                dose,
                route,
                options,
                parameters,
                ..
            } => {
                if kinds[selection] != Kind::Timecourse {
                    return Err(ScanError::request(format!(
                        "The PK observable '{name}' analyses '{selection}', which is a value per \
                         simulation, not a timecourse."
                    )));
                }
                let node = compile_pk(
                    &name,
                    selection,
                    dose.as_ref(),
                    route.as_deref(),
                    options.as_ref(),
                    parameters.as_deref(),
                    &units,
                    model,
                    plans,
                )?;
                let (node, parameter_units) = node;
                let mut names = Vec::new();
                for (parameter, unit) in parameter_units {
                    let output = node.output(&parameter);
                    kinds.insert(output.clone(), Kind::Scalar);
                    units.insert(output.clone(), unit);
                    names.push(output);
                }
                outputs.insert(name.clone(), names);
                Node::Pk(Box::new(node))
            }
        };
        compiled.push((name, node));
    }
    let every_observable: Vec<String> = definitions
        .iter()
        .flat_map(|(id, _)| outputs[id].iter().cloned())
        .collect();
    let mut kept_selections: Vec<String> = Vec::new();
    if let Some(keep) = keep {
        for k in keep {
            if !every_observable.contains(k)
                && k != TIME
                && model.has_selection(k)
                && !kept_selections.contains(k)
            {
                kept_selections.push(k.clone());
            }
        }
    }
    for selection in &kept_selections {
        kinds.insert(selection.clone(), Kind::Timecourse);
        units.insert(selection.clone(), model_unit(model, selection));
        if !resolved.contains(selection) {
            resolved.push(selection.clone());
        }
    }
    let mut every = every_observable;
    every.extend(kept_selections.iter().cloned());
    let kept = kept_outputs(keep, &every, &outputs)?;
    // the observables the kept outputs need
    let mut needed: Vec<String> = Vec::new();
    let mut stack: Vec<String> = kept
        .iter()
        .filter_map(|k| owner(k, &definitions).map(str::to_string))
        .collect();
    while let Some(name) = stack.pop() {
        if needed.contains(&name) {
            continue;
        }
        stack.extend(
            read[&name]
                .iter()
                .filter_map(|s| owner(s, &definitions).map(str::to_string)),
        );
        needed.push(name);
    }
    let nodes: Vec<Node> = compiled
        .into_iter()
        .filter(|(name, _)| needed.contains(name))
        .map(|(_, node)| node)
        .collect();
    let mut used: Vec<&String> = needed.iter().flat_map(|n| read[n].iter()).collect();
    used.extend(kept_selections.iter());
    let selections = resolved.into_iter().filter(|s| used.contains(&s)).collect();
    Ok(Graph {
        nodes,
        selections,
        kinds,
        units,
        keep: kept,
        outputs: every,
    })
}

/// The kept outputs; the id of a PK observable keeps all of its, sbmlsim's `_keep`.
fn kept_outputs(
    keep: Option<&[String]>,
    every: &[String],
    groups: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, ScanError> {
    let Some(keep) = keep else {
        return Ok(every.to_vec());
    };
    let mut kept: Vec<String> = Vec::new();
    for key in keep {
        let names = match groups.get(key) {
            Some(names) if !names.is_empty() => names.clone(),
            _ if every.contains(key) => vec![key.clone()],
            _ => {
                return Err(ScanError::request(format!(
                    "'keep' names '{key}', which is neither an observable of the run nor a \
                     selection of the model: {}.",
                    text::list(every)
                )));
            }
        };
        for name in names {
            if !kept.contains(&name) {
                kept.push(name);
            }
        }
    }
    if kept.is_empty() {
        return Err(ScanError::request(
            "'keep' keeps nothing; name at least one observable.",
        ));
    }
    Ok(kept)
}

/// Compiles a formula: its kind, its unit and the factor into it, sbmlsim's `_compile_formula`.
fn compile_formula(
    id: &str,
    formula: &str,
    declared: Option<&str>,
    kinds: &HashMap<String, Kind>,
    units: &HashMap<String, String>,
    registry: &Registry,
) -> Result<(Node, String), ScanError> {
    let reduced = Reduced::new(formula).map_err(ScanError::request)?;
    for symbol in reduced.time_symbols() {
        if kinds.get(&symbol) == Some(&Kind::Timecourse) {
            return Err(ScanError::request(format!(
                "The time of 'at' in the formula of the observable '{id}' reads the timecourse \
                 '{symbol}'; it is a number or a value per simulation."
            )));
        }
    }
    let unit_of = |s: &str| units.get(s).cloned().unwrap_or_default();
    let algebra = Units {
        registry,
        unit_of: &unit_of,
    };
    let time_unit = units.get(TIME).cloned().unwrap_or_default();
    let time_unit = if time_unit.is_empty() {
        "dimensionless".to_string()
    } else {
        time_unit
    };
    for when in at_times(&reduced, &algebra) {
        if when.units.is_none() {
            continue;
        }
        let text = when.text();
        let factor = registry.factor(&text, &time_unit).ok();
        if factor.is_none_or(|f| (f - 1.0).abs() > 1e-9) {
            return Err(ScanError::request(format!(
                "The time of 'at' in the formula of the observable '{id}' has the unit \
                 '{text}', not the time unit of the model '{}'; give a number or a time in the \
                 time unit of the model.",
                units.get(TIME).cloned().unwrap_or_default()
            )));
        }
    }
    let derived = quantity_of(&reduced, &algebra);
    if let Some(q) = &derived {
        let plain = value_at_ones(&reduced);
        let close = (q.magnitude - plain).abs() <= 1e-12 + 1e-9 * plain.abs();
        if !close {
            return Err(ScanError::request(format!(
                "The formula '{formula}' of the observable '{id}' mixes units of different \
                 scale (e.g. ng/ml and mg/l); the values are in the units of their symbols: \
                 declare one of them as a Formula with unit=..., a factor in the formula does \
                 not convert it."
            )));
        }
    }
    let kind = if reduced
        .outer_symbols()
        .iter()
        .all(|s| kinds.get(s) == Some(&Kind::Scalar))
    {
        Kind::Scalar
    } else {
        Kind::Timecourse
    };
    let derived = derived.map(|q| q.text());
    let node = |factor: f64| Node::Formula {
        id: id.to_string(),
        reduced: reduced.clone(),
        kind,
        factor,
    };
    let Some(declared) = declared else {
        let Some(derived) = derived else {
            return Err(ScanError::request(format!(
                "The unit of the formula '{formula}' of the observable '{id}' cannot be derived, \
                 e.g. of a comparison or of piecewise; give it as unit=."
            )));
        };
        return Ok((node(1.0), derived));
    };
    let declared = registry
        .named(declared)
        .map_err(|err| ScanError::units(format!("The unit of the observable '{id}': {err}")))?
        .to_string();
    let unitless = reduced
        .symbols()
        .iter()
        .all(|s| units.get(s).is_none_or(String::is_empty));
    let derived = match derived {
        None => return Ok((node(1.0), declared)),
        Some(d) if d == "dimensionless" && unitless => return Ok((node(1.0), declared)),
        Some(d) => d,
    };
    // the reason of the registry follows sbmlsim's message, e.g. a power which is no integer
    // (`sqrt` of a unit), which pint reads and the registry refuses
    let factor = registry.factor(&derived, &declared).map_err(|err| {
        ScanError::units(format!(
            "The formula of the observable '{id}' has the unit '{derived}', which cannot be \
             converted into its unit '{declared}': {err}."
        ))
    })?;
    Ok((node(factor), declared))
}

/// Compiles a PK observable, sbmlsim's `_compile_pk` and `compile_pk`: the node and the unit
/// of every parameter it gives.
#[allow(clippy::too_many_arguments)]
fn compile_pk(
    id: &str,
    selection: &str,
    dose: Option<&Dose>,
    route: Option<&str>,
    options: Option<&serde_json::Value>,
    parameters: Option<&[String]>,
    units: &HashMap<String, String>,
    model: &ModelInfo,
    plans: &[Plan],
) -> Result<(PkNode, Vec<(String, String)>), ScanError> {
    let unit = units.get(selection).cloned().unwrap_or_default();
    if unit.is_empty() {
        return Err(ScanError::request(format!(
            "The timecourse '{selection}' of the observable '{id}' has no unit, which the \
             analysis needs; give the model units or analyse a Formula observable with a unit."
        )));
    }
    if plans.is_empty() {
        return Err(ScanError::request(format!(
            "The observable '{id}' has no simulation to analyse."
        )));
    }
    let route = route.map(|r| pk::route_of(id, r)).transpose()?;
    let dose = match dose {
        None => None,
        Some(Dose::Target(target)) => {
            if !model.has_selection(target) {
                return Err(ScanError::request(format!(
                    "The dose '{target}' of the observable '{id}' is no target of the model."
                )));
            }
            let unit = model.unit(target).unwrap_or("").to_string();
            if unit.is_empty() {
                return Err(ScanError::request(format!(
                    "The dose '{target}' of the observable '{id}' has no unit in the model."
                )));
            }
            Some(DoseSpec {
                target: Some(target.clone()),
                amount: 0.0,
                unit,
            })
        }
        Some(Dose::Quantity { value, unit }) => Some(DoseSpec {
            target: None,
            amount: *value,
            unit: model
                .registry()
                .named(unit)
                .map_err(|err| {
                    ScanError::units(format!("The dose of the observable '{id}': {err}"))
                })?
                .to_string(),
        }),
    };
    let mut node = PkNode {
        id: id.to_string(),
        selection: selection.to_string(),
        time_unit: units.get(TIME).cloned().unwrap_or_default(),
        unit,
        dose,
        route,
        options: pk::options_of(id, options)?,
        parameters: Vec::new(),
    };
    let found = pk::probe_parameters(&node, plans)?;
    let kept: Vec<String> = match parameters {
        None => found.iter().map(|(p, _)| p.clone()).collect(),
        Some(parameters) => {
            let unknown: Vec<&String> = parameters
                .iter()
                .filter(|p| !found.iter().any(|(f, _)| f == *p))
                .collect();
            if !unknown.is_empty() {
                let mut all: Vec<&String> = found.iter().map(|(p, _)| p).collect();
                all.sort();
                return Err(ScanError::request(format!(
                    "The observable '{id}' keeps the parameters {}, which pkpdutils does not \
                     derive for its dosing: {}.",
                    text::list(&unknown),
                    text::list(&all)
                )));
            }
            let mut kept: Vec<String> = Vec::new();
            for p in parameters
                .iter()
                .chain(std::iter::once(&pk::FLAGS.to_string()))
            {
                if !kept.contains(p) {
                    kept.push(p.clone());
                }
            }
            kept
        }
    };
    let units = kept
        .iter()
        .map(|p| {
            let unit = found
                .iter()
                .find(|(f, _)| f == p)
                .map(|(_, u)| u.clone())
                .unwrap_or_default();
            (p.clone(), unit)
        })
        .collect();
    node.parameters = kept;
    Ok((node, units))
}

/// One point of a chunk whose simulation ran.
#[derive(Debug, Clone)]
pub(crate) struct PointRun {
    /// The flat index of the point in the scan.
    pub index: usize,
    /// Its plan.
    pub plan: Plan,
    /// The time of every output row, in the time of the result.
    pub time: Vec<f64>,
    /// The selections and the outputs evaluated so far.
    pub values: HashMap<String, Val>,
    /// The failure of the point, which ends its evaluation.
    pub failed: Option<String>,
}

impl PointRun {
    /// The times of the point for the reductions.
    fn times<'a>(&'a self, changes: &'a [f64]) -> Times<'a> {
        Times {
            time: &self.time,
            tolerance: time_tolerance(self.plan.start, self.plan.end),
            changes,
        }
    }

    /// The times of the changes after the start, in the time of the result.
    pub fn changes(&self) -> Vec<f64> {
        self.plan
            .change_times()
            .iter()
            .map(|t| t + self.plan.time_shift)
            .collect()
    }
}

/// The evaluation of the observables on the points of a chunk, see the module.
#[derive(Debug, Clone)]
pub(crate) struct Observation {
    /// The points which ran.
    pub points: Vec<PointRun>,
    /// The next node to evaluate.
    next: usize,
    /// The nodes of the host the round asked for and its question.
    pending: Option<(Vec<usize>, HostObservables)>,
}

impl Observation {
    pub fn new(points: Vec<PointRun>) -> Observation {
        Observation {
            points,
            next: 0,
            pending: None,
        }
    }

    /// The question of the round of the host the evaluation waits for.
    pub fn question(&self) -> Option<&HostObservables> {
        self.pending.as_ref().map(|(_, q)| q)
    }

    /// Evaluates the nodes until the host is asked (the question then waits in
    /// [`Observation::question`]) or every node is evaluated; whether it is done.
    pub fn advance(&mut self, graph: &Graph) -> bool {
        while self.next < graph.nodes.len() {
            match &graph.nodes[self.next] {
                Node::Formula {
                    id,
                    reduced,
                    kind,
                    factor,
                } => {
                    for point in self.points.iter_mut().filter(|p| p.failed.is_none()) {
                        match formula(point, reduced, *kind, *factor) {
                            Ok(value) => {
                                point.values.insert(id.clone(), value);
                            }
                            Err(err) => {
                                point.failed = Some(format!(
                                    "The formula of the observable '{id}' failed: {err}"
                                ));
                            }
                        }
                    }
                    self.next += 1;
                }
                Node::Pk(node) => {
                    self.analyse(node);
                    self.next += 1;
                }
                Node::Custom { .. } => {
                    let mut batch: Vec<usize> = Vec::new();
                    let mut ids: Vec<&str> = Vec::new();
                    while let Some(Node::Custom { id, symbols, .. }) = graph.nodes.get(self.next) {
                        if symbols.iter().any(|s| ids.contains(&s.as_str())) {
                            break;
                        }
                        ids.push(id);
                        batch.push(self.next);
                        self.next += 1;
                    }
                    if !self.points.iter().any(|p| p.failed.is_none()) {
                        continue;
                    }
                    let question = self.question_of(graph, &batch);
                    self.pending = Some((batch, question));
                    return false;
                }
            }
        }
        true
    }

    /// The question of the host for custom nodes.
    fn question_of(&self, graph: &Graph, batch: &[usize]) -> HostObservables {
        let mut observables = Vec::new();
        let mut symbols: Vec<&String> = Vec::new();
        for &k in batch {
            if let Node::Custom { id, symbols: s, .. } = &graph.nodes[k] {
                observables.push(id.clone());
                symbols.extend(s.iter());
            }
        }
        let points = self
            .points
            .iter()
            .filter(|p| p.failed.is_none())
            .map(|p| {
                let rows: Vec<usize> = (0..p.time.len())
                    .filter(|&r| p.time[r].is_finite())
                    .collect();
                let columns: BTreeMap<String, Vec<f64>> = symbols
                    .iter()
                    .map(|s| {
                        let values = match p.values.get(*s) {
                            Some(Val::Scalar(x)) => vec![*x],
                            Some(Val::Series(xs)) => rows.iter().map(|&r| xs[r]).collect(),
                            None => Vec::new(),
                        };
                        ((*s).clone(), values)
                    })
                    .collect();
                HostPoint {
                    index: p.index,
                    time: rows.iter().map(|&r| p.time[r]).collect(),
                    columns,
                }
            })
            .collect();
        let mut scalars: Vec<String> = symbols
            .iter()
            .filter(|s| graph.kinds.get(**s) == Some(&Kind::Scalar))
            .map(|s| (*s).clone())
            .collect();
        scalars.sort();
        scalars.dedup();
        HostObservables {
            observables,
            points,
            scalars,
        }
    }

    /// Takes the answer of the host to its question.
    ///
    /// # Errors
    ///
    /// No question, or an answer which does not fit it.
    pub fn take(&mut self, graph: &Graph, observed: HostObserved) -> Result<(), ScanError> {
        let Some((batch, question)) = self.pending.take() else {
            return Err(ScanError::request(
                "The scan asked for no observables: ask() before tell_observed().",
            ));
        };
        let n = question.points.len();
        for id in &question.observables {
            let ok = observed.values.get(id).is_some_and(|v| v.len() == n);
            if !ok {
                let error = ScanError::request(format!(
                    "The scan asked for the observable '{id}' of {n} points, the answer has not \
                     their values."
                ));
                self.pending = Some((batch, question));
                return Err(error);
            }
        }
        let alive: Vec<usize> = (0..self.points.len())
            .filter(|&k| self.points[k].failed.is_none())
            .collect();
        for &node in &batch {
            let Node::Custom { id, kind, .. } = &graph.nodes[node] else {
                unreachable!("a batch of the host has its observables only")
            };
            for (i, &k) in alive.iter().enumerate() {
                let point = &mut self.points[k];
                if point.failed.is_some() {
                    continue;
                }
                let fail = |message: &str| {
                    format!("The function of the observable '{id}' failed: {message}")
                };
                if let Some(error) = observed
                    .errors
                    .iter()
                    .find(|e| e.observable == *id && e.index == point.index)
                {
                    point.failed = Some(fail(&error.message));
                    continue;
                }
                let values = &observed.values[id][i];
                let rows: Vec<usize> = (0..point.time.len())
                    .filter(|&r| point.time[r].is_finite())
                    .collect();
                match kind {
                    Kind::Scalar => {
                        if values.len() == 1 {
                            point.values.insert(id.clone(), Val::Scalar(values[0]));
                        } else {
                            point.failed = Some(fail(
                                "TypeError: only length-1 arrays can be converted to Python \
                                 scalars",
                            ));
                        }
                    }
                    Kind::Timecourse => {
                        if values.len() == rows.len() {
                            let mut series = vec![f64::NAN; point.time.len()];
                            for (&r, x) in rows.iter().zip(values) {
                                series[r] = *x;
                            }
                            point.values.insert(id.clone(), Val::Series(series));
                        } else {
                            point.failed = Some(fail(&format!(
                                "ValueError: the function returned the shape ({},) for {} time \
                                 points",
                                values.len(),
                                rows.len()
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Analyses the points of the chunk with a PK observable.
    fn analyse(&mut self, node: &PkNode) {
        let alive: Vec<usize> = (0..self.points.len())
            .filter(|&k| self.points[k].failed.is_none())
            .collect();
        let series: Vec<Vec<f64>> = alive
            .iter()
            .map(|&k| {
                let p = &self.points[k];
                match p.values.get(&node.selection) {
                    Some(Val::Series(xs)) => xs.clone(),
                    Some(Val::Scalar(x)) => vec![*x; p.time.len()],
                    None => vec![f64::NAN; p.time.len()],
                }
            })
            .collect();
        let points: Vec<PkPoint<'_>> = alive
            .iter()
            .zip(&series)
            .map(|(&k, values)| PkPoint {
                time: &self.points[k].time,
                values,
                plan: &self.points[k].plan,
            })
            .collect();
        let results = pk::evaluate(node, &points);
        for (&k, result) in alive.iter().zip(results) {
            let point = &mut self.points[k];
            match result {
                Ok(values) => {
                    for (parameter, value) in node.parameters.iter().zip(values) {
                        point
                            .values
                            .insert(node.output(parameter), Val::Scalar(value));
                    }
                }
                Err(err) => point.failed = Some(err),
            }
        }
    }
}

/// Evaluates a formula at one point, sbmlsim's `_formula`.
fn formula(point: &PointRun, reduced: &Reduced, kind: Kind, factor: f64) -> Result<Val, String> {
    let changes = point.changes();
    let times = point.times(&changes);
    let scope = |name: &str| point.values.get(name).cloned();
    let value = reduce::evaluate(reduced, &scope, &times)?;
    let n = point.time.len();
    let value = match (kind, value) {
        (Kind::Scalar, Val::Scalar(x)) => Val::Scalar(x),
        (Kind::Scalar, Val::Series(_)) => {
            return Err(format!(
                "ValueError: the formula '{}' is a timecourse, not a value per simulation",
                reduced.formula
            ));
        }
        (Kind::Timecourse, Val::Scalar(x)) => Val::Series(vec![x; n]),
        (Kind::Timecourse, Val::Series(xs)) => Val::Series(xs),
    };
    Ok(match value {
        Val::Scalar(x) => Val::Scalar(if factor == 1.0 { x } else { x * factor }),
        Val::Series(xs) => Val::Series(
            xs.into_iter()
                .zip(&point.time)
                .map(|(x, t)| {
                    let x = if factor == 1.0 { x } else { x * factor };
                    if t.is_nan() { f64::NAN } else { x }
                })
                .collect(),
        ),
    })
}
