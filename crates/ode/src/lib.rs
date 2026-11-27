//! Pure Rust equation analysis and linked LaTeX for the existing report contract.
mod ast;
pub mod code;
mod flatten;
pub use flatten::flatten;
mod context;
mod derivative;
mod documents;
#[cfg(feature = "filesystem")]
mod filesystem;
mod metadata;
mod sources;
mod templates;
use document_math::Dialect;
mod document_math;
mod latex;
use ast::*;
use indexmap::IndexMap;
use sbmlsim_document::{Ast, AstKind as K, Document, Element, Model, RuleKind};
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};

type SymbolMap = HashMap<String, String>;

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Constant,
    Assigned,
    State,
}
#[derive(Clone)]
struct Quantity {
    sid: String,
    pk: Option<String>,
    role: Role,
    constant: bool,
    value: Option<f64>,
    species: bool,
    compartment: Option<String>,
    amount: bool,
}
struct Reaction {
    sid: String,
    rate: Ast,
}
struct Ode {
    sid: String,
    rhs: Ast,
    origin: &'static str,
    terms: Option<Ast>,
    volume: Option<String>,
    size_rate: Option<String>,
}
#[derive(Clone)]
struct Assignment {
    sid: String,
    math: Ast,
    origin: &'static str,
}
struct PreparedEvent {
    index: usize,
    id: String,
    trigger: Ast,
    optional: [Option<Ast>; 2],
    assigned: IndexMap<String, Ast>,
}
struct PreparedSystem {
    state_rhs: IndexMap<String, Ast>,
    events: Vec<PreparedEvent>,
    assignments: Vec<Assignment>,
    initial: Vec<Assignment>,
    size_maths: IndexMap<String, Option<Ast>>,
}
struct Analysis<'a> {
    model: &'a Model,
    links: &'a Links,
    metadata: HashMap<String, metadata::Symbol>,
    taken: HashSet<String>,
    quantities: IndexMap<String, Quantity>,
    rates: IndexMap<String, Ast>,
    assignments: IndexMap<String, Ast>,
    initial: IndexMap<String, Ast>,
    reactions: Vec<Reaction>,
    continuous: HashSet<String>,
    size_rates: IndexMap<String, String>,
    rhs: IndexMap<String, Ast>,
    unsupported: Vec<(String, String)>,
    pks: HashMap<String, String>,
    odes: Vec<Ode>,
}
fn id(e: &Element) -> String {
    e.id.clone().unwrap_or_default()
}
fn list_text(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|v| format!("'{v}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn equation(pk: Option<&String>, lhs: &str, lines: Vec<String>, origin: &str) -> Value {
    json!({"variable":pk,"lhs":lhs,"lines":lines,"origin":origin})
}
impl<'a> Analysis<'a> {
    fn new(model: &'a Model, links: &'a Links) -> Self {
        let pks = links.symbols.clone();
        let mut taken: HashSet<_> = pks.keys().cloned().collect();
        taken.extend(model.element.id.iter().cloned());
        macro_rules! reserve {
            ($items:expr) => {
                taken.extend($items.iter().filter_map(|e| e.element.id.clone()));
            };
        }
        reserve!(model.function_definitions);
        reserve!(model.compartments);
        reserve!(model.species);
        reserve!(model.parameters);
        reserve!(model.reactions);
        reserve!(model.events);
        for reaction in &model.reactions {
            reserve!(reaction.reactants);
            reserve!(reaction.products);
            reserve!(reaction.modifiers);
        }

        Self {
            model,
            links,
            metadata: metadata::symbols(model),
            taken,
            quantities: IndexMap::new(),
            rates: IndexMap::new(),
            assignments: IndexMap::new(),
            initial: IndexMap::new(),
            reactions: vec![],
            continuous: HashSet::new(),
            size_rates: IndexMap::new(),
            rhs: IndexMap::new(),
            unsupported: vec![],
            pks,
            odes: vec![],
        }
    }
    fn unique(&mut self, base: &str) -> String {
        let mut sid = base.to_owned();
        let mut i = 0;
        while self.taken.contains(&sid) {
            i += 1;
            sid = format!("{base}_{i}");
        }
        self.taken.insert(sid.clone());
        sid
    }
    fn unsupported(&mut self, kind: &str, element: &str) {
        let pair = (kind.to_owned(), element.to_owned());
        if !self.unsupported.contains(&pair) {
            self.unsupported.push(pair);
        }
    }
    fn math(&mut self, element: &str, math: &Ast) -> Ast {
        let mut stack = vec![math];
        while let Some(a) = stack.pop() {
            if a.kind == K::FunctionDelay {
                self.unsupported("delay", element);
            } else if format!("{:?}", a.kind).starts_with("Distrib") {
                self.unsupported("distrib function", element);
            }
            stack.extend(&a.children);
        }
        math.clone()
    }
    fn role(&self, sid: &str) -> Role {
        if self.rates.contains_key(sid) {
            Role::State
        } else if self.assignments.contains_key(sid) {
            Role::Assigned
        } else {
            Role::Constant
        }
    }
    fn quantity(&self, sid: String, constant: bool, value: Option<f64>) -> Quantity {
        Quantity {
            pk: self.pks.get(&sid).cloned(),
            role: self.role(&sid),
            sid,
            constant,
            value,
            species: false,
            compartment: None,
            amount: false,
        }
    }
    fn read(&mut self) -> Result<(), String> {
        for (i, r) in self.model.rules.iter().enumerate() {
            if r.kind == RuleKind::Algebraic {
                self.unsupported(
                    "algebraic rule",
                    r.element
                        .id
                        .as_deref()
                        .or(r.element.meta_id.as_deref())
                        .unwrap_or(&format!("rule{i}")),
                );
                continue;
            }
            let sid = r.variable.clone().unwrap_or_default();
            if self.rates.contains_key(&sid) || self.assignments.contains_key(&sid) {
                return Err(format!("The variable '{sid}' has more than one rule."));
            }
            if let Some(math) = &r.math {
                let math = self.math(&sid, math);
                if r.kind == RuleKind::Rate {
                    self.rates.insert(sid, math);
                } else {
                    self.assignments.insert(sid, math);
                }
            }
        }
        for initial in &self.model.initial_assignments {
            let sid = &initial.symbol;
            if self.assignments.contains_key(sid) || self.initial.contains_key(sid) {
                return Err(format!(
                    "The variable '{sid}' has an initial assignment and an assignment rule or a second initial assignment."
                ));
            }
            if let Some(math) = &initial.math {
                let math = self.math(sid, math);
                self.initial.insert(sid.clone(), math);
            }
        }
        for function in &self.model.function_definitions {
            if let Some(lambda) = &function.math
                && let Some(body) = lambda.children.last()
            {
                self.math(&id(&function.element), body);
                let mut stack = vec![body];
                while let Some(a) = stack.pop() {
                    if a.kind == K::FunctionRateOf {
                        self.unsupported("rateOf in a function definition", &id(&function.element));
                    }
                    stack.extend(&a.children);
                }
            }
        }
        let mut locals = Vec::new();
        let mut refs = Vec::new();
        for input in &self.model.reactions {
            let rid = id(&input.element);
            if input.fast == Some(true) {
                self.unsupported("fast reaction", &rid);
            }
            let mut renamed = HashMap::new();
            if let Some(law) = &input.kinetic_law {
                for (i, p) in law.local_parameters.iter().enumerate() {
                    let pid = id(&p.element);
                    let sid = self.unique(&format!("{rid}_{pid}"));
                    renamed.insert(pid.clone(), sid.clone());
                    let mut info = metadata::symbol(
                        &p.element,
                        metadata::unit(p.units.as_deref(), self.model),
                        "parameter",
                    );
                    info.source = vec![rid.clone(), pid.clone()];
                    self.metadata.insert(sid.clone(), info);
                    let mut q = self.quantity(sid.clone(), true, p.value);
                    q.role = Role::Constant;
                    q.pk = self.links.locals.get(&(rid.clone(), i)).cloned();
                    locals.push(q);
                }
            }
            let mut rate = input
                .kinetic_law
                .as_ref()
                .and_then(|law| law.math.as_ref())
                .map_or_else(|| number(0.0), |math| self.math(&rid, math));
            rename(&mut rate, &renamed);
            self.reactions.push(Reaction { sid: rid, rate });
            for reference in input.reactants.iter().chain(&input.products) {
                if reference.element.id.is_some() {
                    refs.push(self.quantity(
                        id(&reference.element),
                        reference.constant.unwrap_or(false),
                        Some(if reference.stoichiometry_set {
                            reference.stoichiometry
                        } else {
                            1.0
                        }),
                    ));
                }
            }
        }
        for c in &self.model.compartments {
            let q = self.quantity(
                id(&c.element),
                c.constant.unwrap_or(true),
                Some(c.size.unwrap_or(1.0)),
            );
            self.quantities.insert(q.sid.clone(), q);
        }
        let reacting: HashSet<_> = self
            .model
            .reactions
            .iter()
            .flat_map(|r| r.reactants.iter().chain(&r.products))
            .map(|s| s.species.clone())
            .collect();
        self.continuity(&reacting);
        for s in &self.model.species {
            let sid = id(&s.element);
            let amount = s.has_only_substance_units.unwrap_or(false);
            if s.compartment.is_empty() && (!amount || s.initial_concentration.is_some()) {
                return Err(format!(
                    "The species '{sid}' has no compartment, which converts between its amount and its concentration."
                ));
            }
            let constant = s.constant.unwrap_or(false);
            let boundary = s.boundary_condition.unwrap_or(false);
            let role = if self.rates.contains_key(&sid) || self.assignments.contains_key(&sid) {
                self.role(&sid)
            } else if (reacting.contains(&sid) && !constant && !boundary)
                || (!amount && self.continuous.contains(&s.compartment))
            {
                Role::State
            } else {
                Role::Constant
            };
            let value = if amount {
                s.initial_amount
            } else {
                s.initial_concentration
            };
            let q = Quantity {
                pk: self.pks.get(&sid).cloned(),
                sid: sid.clone(),
                role,
                constant,
                value,
                species: true,
                compartment: (!s.compartment.is_empty()).then(|| s.compartment.clone()),
                amount,
            };
            self.quantities.insert(sid, q);
        }
        for p in &self.model.parameters {
            let q = self.quantity(id(&p.element), p.constant.unwrap_or(true), p.value);
            self.quantities.insert(q.sid.clone(), q);
        }
        for q in locals.into_iter().chain(refs) {
            self.quantities.insert(q.sid.clone(), q);
        }
        self.build_odes(&reacting)?;
        Ok(())
    }
    fn continuity(&mut self, reacting: &HashSet<String>) {
        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        let mut seeds: Vec<_> = self.rates.keys().cloned().collect();
        for (sid, math) in self
            .assignments
            .iter()
            .map(|(s, a)| (s.as_str(), a))
            .chain(self.reactions.iter().map(|r| (r.sid.as_str(), &r.rate)))
        {
            let mut stack = vec![math];
            while let Some(a) = stack.pop() {
                if a.kind == K::NameTime {
                    seeds.push(sid.to_owned());
                }
                stack.extend(&a.children);
            }
            for dep in names(math) {
                edges.entry(dep).or_default().push(sid.to_owned());
            }
        }
        for s in &self.model.species {
            let sid = id(&s.element);
            if self.rates.contains_key(&sid) || self.assignments.contains_key(&sid) {
                continue;
            }
            if reacting.contains(&sid)
                && s.constant != Some(true)
                && s.boundary_condition != Some(true)
            {
                seeds.push(sid.clone());
            } else if s.has_only_substance_units != Some(true) {
                edges.entry(s.compartment.clone()).or_default().push(sid);
            }
        }
        while let Some(sid) = seeds.pop() {
            if self.continuous.insert(sid.clone())
                && let Some(next) = edges.get(&sid)
            {
                seeds.extend(next.iter().cloned());
            }
        }
    }
    fn size_rate(&mut self, cid: &str) -> Option<String> {
        if !self.continuous.contains(cid) {
            return None;
        }
        if let Some(sid) = self.size_rates.get(cid) {
            return Some(sid.clone());
        }
        let sid = self.unique(&format!("d{cid}_dt"));
        let label = self
            .metadata
            .get(cid)
            .and_then(|s| s.name.as_deref())
            .unwrap_or(cid);
        self.metadata.insert(
            sid.clone(),
            metadata::Symbol {
                name: Some(format!("rate of {label}")),
                unit: metadata::quotient(
                    metadata::compartment(cid, self.model),
                    metadata::unit(self.model.units.time.as_deref(), self.model),
                ),
                kind: "rate",
                source: vec![cid.to_owned()],
            },
        );
        self.size_rates.insert(cid.to_owned(), sid.clone());
        Some(sid)
    }
    fn build_odes(&mut self, reacting: &HashSet<String>) -> Result<(), String> {
        let changed: HashSet<_> = self
            .rates
            .keys()
            .chain(self.assignments.keys())
            .chain(self.initial.keys())
            .cloned()
            .chain(
                self.model
                    .events
                    .iter()
                    .flat_map(|e| e.event_assignments.iter().map(|a| a.variable.clone())),
            )
            .collect();
        let factors: HashMap<_, _> = self
            .model
            .species
            .iter()
            .map(|s| {
                (
                    id(&s.element),
                    s.conversion_factor
                        .as_ref()
                        .or(self.model.conversion_factor.as_ref()),
                )
            })
            .collect();
        let mut terms: HashMap<String, Vec<(i8, Ast)>> = HashMap::new();
        for r in &self.model.reactions {
            let rid = id(&r.element);
            for (sign, side) in [(-1, &r.reactants), (1, &r.products)] {
                for reference in side {
                    let mut sign = sign;
                    let mut factors_ast = Vec::new();
                    if let Some(sid) = reference
                        .element
                        .id
                        .as_deref()
                        .filter(|s| changed.contains(*s))
                    {
                        factors_ast.push(name(sid));
                    } else {
                        let n = if reference.stoichiometry_set {
                            reference.stoichiometry
                        } else {
                            1.0
                        };
                        if n != 1.0 {
                            if n < 0.0 {
                                sign = -sign;
                            }
                            factors_ast.push(number(n.abs()));
                        }
                    }
                    if let Some(Some(f)) = factors.get(&reference.species) {
                        factors_ast.push(name(f));
                    }
                    factors_ast.push(name(&rid));
                    terms
                        .entry(reference.species.clone())
                        .or_default()
                        .push((sign, product(factors_ast)));
                }
            }
        }
        let quantities: Vec<_> = self.quantities.values().cloned().collect();
        for q in quantities {
            if q.role != Role::State {
                continue;
            }
            let reaction_terms = sum(terms.remove(&q.sid).unwrap_or_default());
            let mut saved_terms = None;
            let mut origin = "rate_rule";
            let rhs = if let Some(rate) = self.rates.get(&q.sid) {
                rate.clone()
            } else {
                origin = "reactions";
                saved_terms = Some(reaction_terms.clone());
                if q.amount {
                    reaction_terms
                } else {
                    let cid = q.compartment.as_deref().unwrap_or("None");
                    let dilution = self.size_rate(cid).map(|rate| {
                        node(
                            K::Times,
                            vec![node(K::Divide, vec![name(&q.sid), name(cid)]), name(&rate)],
                        )
                    });
                    let reacting_state = reacting.contains(&q.sid)
                        && !q.constant
                        && self
                            .model
                            .species
                            .iter()
                            .find(|s| s.element.id.as_deref() == Some(&q.sid))
                            .is_some_and(|s| s.boundary_condition != Some(true));
                    if reacting_state {
                        let divided = node(K::Divide, vec![reaction_terms, name(cid)]);
                        if let Some(dilution) = dilution {
                            sum(vec![(1, divided), (-1, dilution)])
                        } else {
                            divided
                        }
                    } else {
                        origin = "dilution";
                        saved_terms = None;
                        dilution.map_or_else(|| number(0.0), negate)
                    }
                }
            };
            let volume = if origin == "rate_rule" || q.amount {
                None
            } else {
                q.compartment.clone()
            };
            let size_rate = volume
                .as_ref()
                .and_then(|c| self.size_rates.get(c))
                .cloned();
            self.odes.push(Ode {
                sid: q.sid.clone(),
                rhs: rhs.clone(),
                origin,
                terms: saved_terms,
                volume,
                size_rate,
            });
            self.rhs.insert(q.sid, rhs);
        }
        Ok(())
    }
    fn resolve(
        &mut self,
        math: &Ast,
        element: &str,
        stack: &mut BTreeSet<String>,
    ) -> Result<Ast, String> {
        if stack.len() > 256 {
            return Err("The rates exceed the equation dependency limit.".into());
        }
        let mut out = math.clone();
        let mut pending = vec![&mut out];
        while let Some(a) = pending.pop() {
            if a.kind == K::FunctionRateOf {
                if a.children.len() != 1 || a.children[0].kind != K::Name {
                    self.unsupported("rateOf of an expression", element);
                } else {
                    let sid = a.children[0].name.clone().unwrap_or_default();
                    let replacement = if let Some(rhs) = self.rhs.get(&sid).cloned() {
                        if !stack.insert(sid.clone()) {
                            return Err(format!(
                                "The rates of {} depend on each other in a cycle.",
                                list_text(&stack.iter().cloned().collect::<Vec<_>>())
                            ));
                        }
                        let result = self.resolve(&rhs, element, stack)?;
                        stack.remove(&sid);
                        Some(result)
                    } else if let Some(q) = self.quantities.get(&sid) {
                        if q.role == Role::Constant {
                            Some(number(0.0))
                        } else if !q.species
                            && self
                                .model
                                .compartments
                                .iter()
                                .any(|c| c.element.id.as_deref() == Some(&sid))
                        {
                            Some(
                                self.size_rate(&sid)
                                    .map_or_else(|| number(0.0), |s| name(&s)),
                            )
                        } else {
                            self.unsupported("rateOf of an assigned variable", element);
                            None
                        }
                    } else if self.reactions.iter().any(|r| r.sid == sid) {
                        self.unsupported("rateOf of an assigned variable", element);
                        None
                    } else {
                        return Err(format!(
                            "The rateOf in '{element}' refers to the unknown id '{sid}'."
                        ));
                    };
                    if let Some(replacement) = replacement {
                        *a = replacement;
                        continue;
                    }
                }
            }
            pending.extend(&mut a.children);
        }
        Ok(out)
    }
    fn total_rate(
        &mut self,
        sid: &str,
        element: &str,
        stack: &mut BTreeSet<String>,
    ) -> Result<Option<Ast>, String> {
        if !stack.insert(sid.to_owned()) {
            return Err(format!(
                "The rates of {} depend on each other in a cycle.",
                list_text(&stack.iter().cloned().collect::<Vec<_>>())
            ));
        }
        let result = if let Some(rhs) = self.rhs.get(sid).cloned() {
            Some(self.resolve(&rhs, element, &mut BTreeSet::new())?)
        } else if self.size_rates.values().any(|s| s == sid) {
            None
        } else if let Some(math) = self
            .assignments
            .get(sid)
            .or_else(|| {
                self.reactions
                    .iter()
                    .find(|r| r.sid == sid)
                    .map(|r| &r.rate)
            })
            .cloned()
        {
            let math = self.resolve(&math, element, &mut BTreeSet::new())?;
            let math = self.expand_functions(&math, 0)?;
            check_depth(&math, element)?;
            if let Some(by_time) = derivative::derivative(&math, "@time", 0) {
                let mut terms = vec![(1, by_time)];
                let mut valid = true;
                for dep in names(&math) {
                    let Some(partial) = derivative::derivative(&math, &dep, 0) else {
                        valid = false;
                        break;
                    };
                    if is(&partial, 0.0) {
                        continue;
                    }
                    let Some(rate) = self.total_rate(&dep, element, stack)? else {
                        valid = false;
                        break;
                    };
                    terms.push((1, derivative::multiply(vec![partial, rate])));
                }
                valid.then(|| sum(terms))
            } else {
                None
            }
        } else if self.quantities.contains_key(sid) {
            Some(number(0.0))
        } else {
            return Err(format!(
                "The rate of '{element}' refers to the unknown id '{sid}'."
            ));
        };
        stack.remove(sid);
        Ok(result)
    }
    fn expand_functions(&self, math: &Ast, depth: usize) -> Result<Ast, String> {
        if depth > 100 {
            return Err("Function definitions exceed the equation expansion limit.".into());
        }
        let mut out = math.clone();
        let mut stack = vec![&mut out];
        while let Some(a) = stack.pop() {
            if a.kind == K::Function
                && let Some(f) = self
                    .model
                    .function_definitions
                    .iter()
                    .find(|f| f.element.id == a.name)
                && let Some(lambda) = &f.math
                && let Some(body) = lambda.children.last()
            {
                let bindings: HashMap<_, _> = lambda
                    .children
                    .iter()
                    .take(lambda.children.len() - 1)
                    .filter_map(|p| p.name.as_deref())
                    .zip(&a.children)
                    .collect();
                let mut body = body.clone();
                let mut work = vec![&mut body];
                while let Some(b) = work.pop() {
                    if b.kind == K::Name
                        && let Some(value) = b.name.as_deref().and_then(|s| bindings.get(s))
                    {
                        *b = (*value).clone();
                    } else {
                        work.extend(&mut b.children);
                    }
                }
                *a = self.expand_functions(&body, depth + 1)?;
            } else {
                stack.extend(&mut a.children);
            }
        }
        Ok(out)
    }
}

fn ordered(items: Vec<Assignment>) -> Result<Vec<Assignment>, String> {
    let indices: HashMap<_, _> = items
        .iter()
        .enumerate()
        .map(|(i, a)| (a.sid.as_str(), i))
        .collect();
    let mut remaining = vec![0; items.len()];
    let mut users = vec![vec![]; items.len()];
    for (i, a) in items.iter().enumerate() {
        for name in names(&a.math) {
            if let Some(dep) = indices.get(name.as_str()) {
                remaining[i] += 1;
                users[*dep].push(i);
            }
        }
    }
    let mut ready = BinaryHeap::new();
    for (i, count) in remaining.iter().enumerate() {
        if *count == 0 {
            ready.push(Reverse(i));
        }
    }
    let mut order = Vec::new();
    while let Some(Reverse(i)) = ready.pop() {
        order.push(i);
        for u in &users[i] {
            remaining[*u] -= 1;
            if remaining[*u] == 0 {
                ready.push(Reverse(*u));
            }
        }
    }
    if order.len() != items.len() {
        let cycle = items
            .iter()
            .enumerate()
            .filter(|(i, _)| remaining[*i] > 0)
            .map(|(_, a)| a.sid.clone())
            .collect::<Vec<_>>();
        return Err(format!(
            "The assignments of {} depend on each other in a cycle.",
            list_text(&cycle)
        ));
    }
    let mut slots: Vec<_> = items.into_iter().map(Some).collect();
    Ok(order.into_iter().filter_map(|i| slots[i].take()).collect())
}
impl Analysis<'_> {
    fn prepare(&mut self) -> Result<PreparedSystem, String> {
        let mut state_rhs = IndexMap::new();
        for (sid, rhs) in self.rhs.clone() {
            let resolved = self.resolve(&rhs, &sid, &mut BTreeSet::from([sid.clone()]))?;
            state_rhs.insert(sid, resolved);
        }
        for i in 0..self.reactions.len() {
            let sid = self.reactions[i].sid.clone();
            let rate = self.resolve(&self.reactions[i].rate.clone(), &sid, &mut BTreeSet::new())?;
            self.reactions[i].rate = rate;
        }
        let mut assignments = Vec::new();
        for (sid, math) in self.assignments.clone() {
            let math = self.resolve(&math, &sid, &mut BTreeSet::new())?;
            assignments.push(Assignment {
                sid,
                math,
                origin: "assignment_rule",
            });
        }
        let events = self.analyze_events()?;
        let mut initial = Vec::new();
        for q in self.quantities.values().cloned().collect::<Vec<_>>() {
            let origin = if self.assignments.contains_key(&q.sid) {
                "assignment_rule"
            } else if self.initial.contains_key(&q.sid) {
                "initial_assignment"
            } else {
                "initial_value"
            };
            let math = if let Some(a) = self
                .assignments
                .get(&q.sid)
                .or_else(|| self.initial.get(&q.sid))
                .cloned()
            {
                Some(self.resolve(&a, &q.sid, &mut BTreeSet::new())?)
            } else if q.species {
                let s = self
                    .model
                    .species
                    .iter()
                    .find(|s| s.element.id.as_deref() == Some(&q.sid))
                    .unwrap();
                if !q.amount
                    && let Some(amount) = s.initial_amount
                {
                    Some(node(
                        K::Divide,
                        vec![
                            number(amount),
                            name(q.compartment.as_deref().unwrap_or("None")),
                        ],
                    ))
                } else if q.amount
                    && let Some(concentration) = s.initial_concentration
                {
                    Some(node(
                        K::Times,
                        vec![
                            number(concentration),
                            name(q.compartment.as_deref().unwrap_or("None")),
                        ],
                    ))
                } else {
                    (q.role == Role::State).then(|| number(q.value.unwrap_or(0.0)))
                }
            } else {
                (q.role == Role::State).then(|| number(q.value.unwrap_or(0.0)))
            };
            if let Some(math) = math {
                initial.push(Assignment {
                    sid: q.sid,
                    math,
                    origin,
                });
            }
        }
        let mut size_maths = IndexMap::new();
        loop {
            let pending = self
                .size_rates
                .keys()
                .filter(|c| !size_maths.contains_key(*c))
                .cloned()
                .collect::<Vec<_>>();
            if pending.is_empty() {
                break;
            }
            for cid in pending {
                let math = if let Some(rhs) = state_rhs.get(&cid) {
                    Some(rhs.clone())
                } else {
                    let math = self.total_rate(&cid, &cid, &mut BTreeSet::new())?;
                    let math = math.filter(|a| !names(a).contains(&self.size_rates[&cid]));
                    if math.is_none() {
                        self.unsupported("rate of an assigned size", &cid);
                    }
                    math
                };
                size_maths.insert(cid, math);
            }
        }
        let mut size_assignments = Vec::new();
        for c in &self.model.compartments {
            let cid = id(&c.element);
            if let Some(Some(math)) = size_maths.get(&cid) {
                size_assignments.push(Assignment {
                    sid: self.size_rates[&cid].clone(),
                    math: math.clone(),
                    origin: "size_rate",
                });
            }
        }
        let mut all_assignments = size_assignments.clone();
        all_assignments.extend(assignments);
        all_assignments.extend(self.reactions.iter().map(|r| Assignment {
            sid: r.sid.clone(),
            math: r.rate.clone(),
            origin: "reaction",
        }));
        let assignments = ordered(all_assignments)?;
        let mut rates: IndexMap<_, _> = self
            .reactions
            .iter()
            .map(|r| {
                (
                    r.sid.clone(),
                    Assignment {
                        sid: r.sid.clone(),
                        math: r.rate.clone(),
                        origin: "reaction",
                    },
                )
            })
            .collect();
        rates.extend(size_assignments.iter().map(|a| (a.sid.clone(), a.clone())));
        let mut needed: BTreeSet<_> = initial.iter().flat_map(|a| names(&a.math)).collect();
        let mut added = HashSet::new();
        loop {
            let pending = rates
                .keys()
                .filter(|s| needed.contains(*s) && !added.contains(*s))
                .cloned()
                .collect::<Vec<_>>();
            if pending.is_empty() {
                break;
            }
            for sid in pending {
                needed.extend(names(&rates[&sid].math));
                added.insert(sid);
            }
        }
        initial.extend(
            rates
                .into_iter()
                .filter(|(sid, _)| added.contains(sid))
                .map(|(_, a)| a),
        );
        let initial = ordered(initial)?;
        Ok(PreparedSystem {
            state_rhs,
            events,
            assignments,
            initial,
            size_maths,
        })
    }
    fn output(&mut self) -> Result<Value, String> {
        let prepared = self.prepare()?;
        self.typeset_prepared(&prepared, Dialect::Latex, false)
    }
    fn named_symbols(
        &self,
        ids: &[String],
        dialect: Dialect,
        by_name: bool,
    ) -> Result<HashMap<String, String>, String> {
        if !by_name {
            return dialect.symbols(ids);
        }
        let mut candidates = ids
            .iter()
            .map(|sid| {
                (
                    sid.clone(),
                    self.metadata
                        .get(sid)
                        .and_then(|m| m.name.clone())
                        .filter(|n| {
                            n.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
                                && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        })
                        .unwrap_or_else(|| sid.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        loop {
            let names = ids
                .iter()
                .map(|id| candidates[id].clone())
                .collect::<Vec<_>>();
            let rendered = dialect.symbols(&names)?;
            let mut counts = HashMap::new();
            for sid in ids {
                *counts
                    .entry(rendered[&candidates[sid]].clone())
                    .or_insert(0) += 1;
            }
            let mut changed = false;
            for sid in ids {
                if counts[&rendered[&candidates[sid]]] > 1 && candidates[sid] != *sid {
                    candidates.insert(sid.clone(), sid.clone());
                    changed = true;
                }
            }
            if !changed {
                return Ok(ids
                    .iter()
                    .map(|sid| (sid.clone(), rendered[&candidates[sid]].clone()))
                    .collect());
            }
        }
    }
    fn presentation_symbols(
        &self,
        prepared: &PreparedSystem,
        dialect: Dialect,
        by_name: bool,
    ) -> Result<(SymbolMap, SymbolMap), String> {
        let mut ids = self.quantities.keys().cloned().collect::<Vec<_>>();
        ids.extend(self.reactions.iter().map(|r| r.sid.clone()));
        ids.extend(
            self.model
                .function_definitions
                .iter()
                .filter(|f| f.math.is_some())
                .map(|f| id(&f.element)),
        );
        let mut plain = self.named_symbols(&ids, dialect, by_name)?;
        let mut taken: HashSet<_> = plain.values().cloned().collect();
        for r in &self.reactions {
            let label = self
                .metadata
                .get(&r.sid)
                .and_then(|m| m.name.as_deref())
                .filter(|n| by_name && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                .unwrap_or(&r.sid);
            let preferred = dialect.symbol(&format!("v_{label}"))?;
            if !taken.contains(&preferred) {
                if let Some(old) = plain.insert(r.sid.clone(), preferred.clone()) {
                    taken.remove(&old);
                }
                taken.insert(preferred);
            }
        }
        for (cid, rate) in &self.size_rates {
            let Some(symbol) = plain.get(cid) else {
                return Err(format!("'{cid}'"));
            };
            plain.insert(rate.clone(), dialect.derivative(symbol, symbol));
        }
        for q in self.quantities.values() {
            if q.species
                && let Some(cid) = &q.compartment
                && !plain.contains_key(cid)
            {
                return Err(format!("'{cid}'"));
            }
        }
        if let Some(sid) = self.model.element.id.as_deref() {
            dialect.symbol(sid)?;
        }
        for assignment in &prepared.assignments {
            if !plain.contains_key(&assignment.sid) {
                return Err(format!("'{}'", assignment.sid));
            }
        }
        for reaction in &self.model.reactions {
            for sid in reaction
                .reactants
                .iter()
                .chain(&reaction.products)
                .map(|r| r.species.as_str())
                .chain(reaction.modifiers.iter().map(|r| r.species.as_str()))
            {
                if !plain.contains_key(sid) {
                    return Err(format!("'{sid}'"));
                }
            }
        }
        let mut symbols = plain.clone();
        for (sid, symbol) in &mut symbols {
            let pk = self
                .quantities
                .get(sid)
                .and_then(|q| q.pk.as_ref())
                .or_else(|| self.pks.get(sid))
                .or_else(|| {
                    self.size_rates
                        .iter()
                        .find(|(_, s)| *s == sid)
                        .and_then(|(cid, _)| self.pks.get(cid))
                });
            if let Some(pk) = pk {
                *symbol = format!(r"\htmlData{{pk={pk}}}{{{symbol}}}");
            }
        }
        Ok((plain, symbols))
    }
    fn typeset_prepared(
        &self,
        prepared: &PreparedSystem,
        dialect: Dialect,
        by_name: bool,
    ) -> Result<Value, String> {
        let PreparedSystem {
            state_rhs,
            events,
            assignments,
            initial,
            size_maths,
        } = prepared;
        let (plain, symbols) = self.presentation_symbols(prepared, dialect, by_name)?;
        let mut ode_rows = Vec::new();
        for ode in &self.odes {
            if let Some(rate) = &ode.size_rate
                && self
                    .size_rates
                    .iter()
                    .any(|(cid, s)| s == rate && size_maths.get(cid).is_some_and(Option::is_none))
            {
                continue;
            }
            let rhs = state_rhs.get(&ode.sid).unwrap_or(&ode.rhs);
            let lines = if let (Some(terms), Some(volume)) = (&ode.terms, &ode.volume) {
                let mut lines = dialect.lines(terms, &symbols)?;
                let dilution = ode.size_rate.as_ref().map(|rate| {
                    node(
                        K::Times,
                        vec![
                            node(K::Divide, vec![name(&ode.sid), name(volume)]),
                            name(rate),
                        ],
                    )
                });
                if lines.len() == 1 {
                    let divided = node(K::Divide, vec![terms.clone(), name(volume)]);
                    let rhs =
                        dilution.map_or(divided.clone(), |d| sum(vec![(1, divided), (-1, d)]));
                    lines = vec![dialect.print(&rhs, &symbols, false)?];
                } else {
                    let factor = dialect.print(
                        &node(K::Divide, vec![number(1.0), name(volume)]),
                        &symbols,
                        false,
                    )?;
                    lines[0] = format!(
                        "{factor} {} {}",
                        if dialect == Dialect::Typst {
                            r"lr(size: #150%, \()"
                        } else {
                            r"\Bigl("
                        },
                        lines[0]
                    );
                    let last = lines.len() - 1;
                    lines[last].push_str(if dialect == Dialect::Typst {
                        r" lr(size: #150%, \))"
                    } else {
                        r" \Bigr)"
                    });
                    if let Some(d) = dilution {
                        lines.push(format!("- {}", dialect.print(&d, &symbols, false)?));
                    }
                }
                lines
            } else {
                dialect.lines(rhs, &symbols)?
            };
            let lhs = dialect.derivative(&plain[&ode.sid], &symbols[&ode.sid]);
            ode_rows.push(equation(self.pks.get(&ode.sid), &lhs, lines, ode.origin));
        }
        let reactions = self
            .reactions
            .iter()
            .map(|r| {
                Ok(equation(
                    self.pks.get(&r.sid),
                    &symbols[&r.sid],
                    dialect.lines(&r.rate, &symbols)?,
                    "reaction",
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let assignments = assignments
            .iter()
            .filter(|a| {
                a.origin == "assignment_rule"
                    || (a.origin == "size_rate"
                        && !self
                            .size_rates
                            .iter()
                            .any(|(cid, rate)| rate == &a.sid && self.rates.contains_key(cid)))
            })
            .map(|a| {
                Ok(equation(
                    self.pks.get(&a.sid).or_else(|| {
                        self.size_rates
                            .iter()
                            .find(|(_, rate)| *rate == &a.sid)
                            .and_then(|(cid, _)| self.pks.get(cid))
                    }),
                    &symbols[&a.sid],
                    dialect.lines(&a.math, &symbols)?,
                    a.origin,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let initial = initial
            .iter()
            .filter(|a| {
                a.origin == "initial_assignment"
                    || (a.origin == "initial_value" && value(&a.math).is_none())
            })
            .map(|a| {
                Ok(equation(
                    self.pks.get(&a.sid),
                    &symbols[&a.sid],
                    dialect.lines(&a.math, &symbols)?,
                    a.origin,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut functions = Vec::new();
        for f in &self.model.function_definitions {
            if let Some(lambda) = &f.math
                && let Some(body) = lambda.children.last()
            {
                let sid = id(&f.element);
                let mut local = HashMap::new();
                for function in &self.model.function_definitions {
                    let id = id(&function.element);
                    if let Some(symbol) = symbols.get(&id) {
                        local.insert(id, symbol.clone());
                    }
                }
                let mut args = Vec::new();
                for arg in lambda.children.iter().take(lambda.children.len() - 1) {
                    let arg = arg.name.as_deref().unwrap_or("");
                    let symbol = dialect.symbol(arg)?;
                    local.insert(arg.to_owned(), symbol.clone());
                    args.push(symbol);
                }
                let lhs = dialect.call_symbol(&symbols[&sid], &args);
                functions.push(equation(
                    self.pks.get(&sid),
                    &lhs,
                    vec![dialect.print(body, &local, false)?],
                    "function",
                ));
            }
        }
        let events = self.events(events, &symbols, dialect)?;
        let unsupported = self
            .unsupported
            .iter()
            .map(|(kind, label)| {
                let expected = match kind.as_str() {
                    "algebraic rule" => Some("AlgebraicRule"),
                    "fast reaction" => Some("Reaction"),
                    _ => None,
                };
                let by_meta = self
                    .links
                    .metadata
                    .get(&(expected.unwrap_or("").to_owned(), label.clone()))
                    .map(String::as_str);
                let by_id = self.pks.get(label).map(String::as_str).filter(|pk| {
                    expected.is_none_or(|t| {
                        pk.split('/')
                            .nth(1)
                            .is_some_and(|s| s.starts_with(&format!("{t}:")))
                    })
                });
                json!({"kind":kind,"element":by_meta.or(by_id)})
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"odes":ode_rows,"reactions":reactions,"assignments":assignments,"functions":functions,"initial":initial,"events":events,"unsupported":unsupported}),
        )
    }
    fn analyze_events(&mut self) -> Result<Vec<PreparedEvent>, String> {
        let mut events = Vec::new();
        for (i, event) in self.model.events.iter().enumerate() {
            let eid = event
                .element
                .id
                .clone()
                .unwrap_or_else(|| self.unique(&format!("event{i}")));
            let trigger = if let Some(math) = event.trigger.as_ref().and_then(|t| t.math.as_ref()) {
                let math = self.math(&eid, math);
                self.resolve(&math, &eid, &mut BTreeSet::new())?
            } else {
                node(K::ConstantFalse, vec![])
            };
            let expanded_trigger = self.expand_functions(&trigger, 0)?;
            check_depth(&expanded_trigger, "")?;
            if !continuous_trigger(&expanded_trigger) {
                self.unsupported("event trigger", &eid);
            }
            let mut optional = Vec::new();
            for math in [
                event.delay.as_ref().and_then(|d| d.math.as_ref()),
                event.priority.as_ref().and_then(|p| p.math.as_ref()),
            ] {
                optional.push(if let Some(math) = math {
                    let math = self.math(&eid, math);
                    let math = self.resolve(&math, &eid, &mut BTreeSet::new())?;
                    Some(math)
                } else {
                    None
                });
            }
            let mut assigned = IndexMap::new();
            for a in &event.event_assignments {
                let Some(q) = self.quantities.get(&a.variable) else {
                    return Err(format!(
                        "The event '{eid}' assigns '{}', which is no quantity.",
                        a.variable
                    ));
                };
                if let Some(math) = &a.math {
                    if q.constant {
                        self.unsupported("event assignment to a constant", &eid);
                    } else if q.role == Role::Assigned {
                        self.unsupported("event assignment to an assigned variable", &eid);
                    } else {
                        let math = self.math(&eid, math);
                        let math = self.resolve(&math, &eid, &mut BTreeSet::new())?;
                        assigned.insert(a.variable.clone(), math);
                    }
                }
            }
            events.push(PreparedEvent {
                index: i,
                id: eid,
                trigger,
                optional: optional.try_into().unwrap(),
                assigned,
            });
        }
        Ok(events)
    }
    fn events(
        &self,
        prepared: &[PreparedEvent],
        symbols: &HashMap<String, String>,
        dialect: Dialect,
    ) -> Result<Vec<Value>, String> {
        let mut events = vec![];
        for prepared in prepared {
            let event = &self.model.events[prepared.index];
            let eid = &prepared.id;
            let trigger = dialect.print(&prepared.trigger, symbols, true)?;
            let optional = prepared
                .optional
                .iter()
                .map(|m| {
                    m.as_ref()
                        .map(|m| dialect.print(m, symbols, false))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut rows = vec![];
            for a in self.event_assignments(prepared) {
                let mut factors = vec![];
                if let Some(math) = &a.math
                    && (a.scale.is_none() || !is(math, 1.0))
                {
                    factors.push(math.clone());
                }
                if let Some(scale) = a.scale {
                    factors.push(scale);
                }
                let mut value = if factors.is_empty() {
                    number(1.0)
                } else {
                    product(factors)
                };
                let mut symbols = symbols.clone();
                if let Some(cid) = a.divisor {
                    let new = format!("@new:{cid}");
                    symbols.insert(new.clone(), dialect.new_volume(&symbols[&cid]));
                    value = node(K::Divide, vec![value, name(&new)]);
                }
                let pk = self
                    .quantities
                    .get(&a.sid)
                    .and_then(|q| q.pk.as_ref())
                    .or_else(|| self.pks.get(&a.sid));
                rows.push(equation(
                    pk,
                    &symbols[&a.sid],
                    vec![dialect.print(&value, &symbols, false)?],
                    "event",
                ));
            }
            events.push(json!({"event":self.pks.get(eid),"label":eid,"trigger":trigger,"delay":optional[0],"priority":optional[1],"initialValue":event.trigger.as_ref().and_then(|t|t.initial_value).unwrap_or(true),"persistent":event.trigger.as_ref().and_then(|t|t.persistent).unwrap_or(true),"useValuesFromTriggerTime":event.use_values_from_trigger_time.unwrap_or(true),"assignments":rows}));
        }
        Ok(events)
    }
}

/// Caller-owned links. Equation analysis knows source identities, never report object types.
#[derive(Default, Clone)]
pub struct Links {
    /// The opaque link of a globally scoped source identifier.
    pub symbols: HashMap<String, String>,
    /// The opaque link of a local parameter, keyed by reaction identifier and source position.
    pub locals: HashMap<(String, usize), String>,
    /// Links by SBML element kind and meta identifier (empty kind means any supported kind).
    pub metadata: HashMap<(String, String), String>,
}

/// Typeset a normalized model, optionally decorating its symbols with caller-supplied links.
pub fn typeset(model: &Model, links: &Links) -> Result<Value, String> {
    let mut analysis = Analysis::new(model, links);
    analysis.read()?;
    analysis.output()
}

/// Caller-provided resolution of comp model references within a closed set of documents.
pub trait ModelResolver<'a> {
    /// Return the target document position and model of a model reference.
    fn model(&self, entry: usize, model_ref: &str) -> Option<(usize, &'a Model)>;
    /// Maximum number of expanded SBML elements.
    fn budget(&self) -> usize {
        10_000
    }
}

/// Normalize older SBML encodings while retaining authored quantity identities.
pub fn normalize<'a>(document: &Document, model: &'a Model) -> std::borrow::Cow<'a, Model> {
    if document.level >= 3 {
        return std::borrow::Cow::Borrowed(model);
    }
    let mut converted = model.clone();
    for (sid, kind, exponent) in [
        ("substance", "mole", 1.0),
        ("time", "second", 1.0),
        ("volume", "litre", 1.0),
        ("area", "metre", 2.0),
        ("length", "metre", 1.0),
    ] {
        if !converted
            .unit_definitions
            .iter()
            .any(|u| u.element.id.as_deref() == Some(sid))
        {
            converted
                .unit_definitions
                .push(sbmlsim_document::UnitDefinition {
                    element: Element {
                        id: Some(sid.into()),
                        ..Default::default()
                    },
                    units: vec![sbmlsim_document::Unit {
                        kind: Some(kind.into()),
                        exponent: Some(exponent),
                        scale: Some(0),
                        multiplier: Some(1.0),
                    }],
                });
        }
    }
    converted.units.time.get_or_insert_with(|| "time".into());
    converted
        .units
        .substance
        .get_or_insert_with(|| "substance".into());
    converted
        .units
        .extent
        .get_or_insert_with(|| "substance".into());
    converted
        .units
        .volume
        .get_or_insert_with(|| "volume".into());
    converted.units.area.get_or_insert_with(|| "area".into());
    converted
        .units
        .length
        .get_or_insert_with(|| "length".into());
    let mut taken: HashSet<_> = converted
        .parameters
        .iter()
        .map(|p| id(&p.element))
        .chain(converted.species.iter().map(|s| id(&s.element)))
        .collect();
    let mut generated = 0;
    for reaction in &mut converted.reactions {
        reaction.fast = None;
        for reference in reaction.reactants.iter_mut().chain(&mut reaction.products) {
            if let Some(math) = &reference.stoichiometry_math_ast {
                let sid = reference.element.id.clone().unwrap_or_else(|| {
                    loop {
                        let sid = format!("generatedId_{generated}");
                        generated += 1;
                        if taken.insert(sid.clone()) {
                            break sid;
                        }
                    }
                });
                reference.element.id = Some(sid.clone());
                converted.rules.push(sbmlsim_document::Rule {
                    variable: Some(sid),
                    kind: RuleKind::Assignment,
                    math: Some(math.clone()),
                    ..Default::default()
                });
            }
        }
    }
    std::borrow::Cow::Owned(converted)
}

/// Version of the structured equation and source-export contract.
pub const ODE_CONTRACT: &str = "ode/1";

/// Options of the standalone code or document target.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RenderOptions {
    /// Include the standalone integration driver in executable targets.
    pub simulator: bool,
    /// Include the document preamble and top-level title.
    pub standalone: bool,
    /// Mathematical symbols use source identifiers or unambiguous names.
    pub symbols: String,
    /// Source filename to describe in the generated header.
    pub source: Option<String>,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            simulator: true,
            standalone: true,
            symbols: "id".into(),
            source: None,
        }
    }
}

/// A generated file, with its canonical suffix and model identifier.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Export {
    /// Download filename (Diffrax and Python both use `.py`).
    pub filename: String,
    /// UTF-8 generated content.
    pub content: String,
}

/// Render an owned, resolved model through the shared equation plan.
pub fn render_model(
    document: &Document,
    model: &Model,
    fmt: &str,
    options: &RenderOptions,
) -> Result<Export, String> {
    render_model_template(document, model, fmt, options, None)
}
fn render_model_template(
    document: &Document,
    model: &Model,
    fmt: &str,
    options: &RenderOptions,
    template: Option<&str>,
) -> Result<Export, String> {
    let target = match fmt {
        "python" => Some(code::Target::Python),
        "diffrax" => Some(code::Target::Diffrax),
        "julia" => Some(code::Target::Julia),
        "r" => Some(code::Target::R),
        "markdown" | "latex" | "typst" => None,
        _ => {
            return Err(format!(
                "The format '{fmt}' is none of diffrax, julia, latex, markdown, python, r, typst."
            ));
        }
    };
    let model = normalize(document, model);
    let links = Links::default();
    let mut analysis = Analysis::new(&model, &links);
    analysis.read()?;
    let prepared = analysis.prepare()?;
    let context = if let Some(target) = target {
        context::build(
            document,
            &analysis,
            &prepared,
            target,
            options.simulator,
            options.source.as_deref(),
        )?
    } else {
        documents::build(document, &analysis, &prepared, fmt, options)?
    };
    let content = if template.is_some() {
        templates::render_custom(fmt, context, template)?
    } else {
        templates::render(fmt, context)?
    };
    let suffix = match fmt {
        "python" | "diffrax" => ".py",
        "julia" => ".jl",
        "r" => ".R",
        "markdown" => ".md",
        "latex" => ".tex",
        _ => ".typ",
    };
    Ok(Export {
        filename: format!("{}{suffix}", model.element.id.as_deref().unwrap_or("model")),
        content,
    })
}

/// Read and export SBML entirely in Rust. External models must be supplied by a source adapter.
pub fn export(source: &[u8], fmt: &str, options: &RenderOptions) -> Result<Export, String> {
    OdeSystem::read(source)?.render(fmt, options)
}

/// An owned SBML equation source. Every target uses the same normalized model.
#[derive(Clone, Debug)]
pub struct OdeSystem {
    document: Document,
    model: Model,
}
impl OdeSystem {
    /// Read SBML without filesystem or network access; expand in-document comp definitions.
    pub fn read(source: &[u8]) -> Result<Self, String> {
        Self::read_with_sources(source, "./model.xml", &[], 10_000)
    }
    /// Source identifier of the main model.
    pub fn model_id(&self) -> Option<&str> {
        self.model.element.id.as_deref()
    }
    /// Typeset equations with caller-supplied opaque source links.
    pub fn typeset(&self, links: &Links) -> Result<Value, String> {
        typeset(&self.model, links)
    }
    /// Render a caller-supplied template with the same typed target context.
    pub fn render_template(
        &self,
        fmt: &str,
        options: &RenderOptions,
        template: &str,
    ) -> Result<Export, String> {
        render_model_template(&self.document, &self.model, fmt, options, Some(template))
    }
    /// Render code or a document.
    pub fn render(&self, fmt: &str, options: &RenderOptions) -> Result<Export, String> {
        render_model(&self.document, &self.model, fmt, options)
    }
}
