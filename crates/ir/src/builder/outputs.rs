//! The initial evaluations, the events and the selections.

use super::*;

/// The order of evaluations as assignments, or the error naming a cycle.
pub(super) fn ordered(
    entries: Vec<(Symbol, Expr)>,
    ids: impl Fn(Symbol) -> String,
    what: &str,
) -> Result<Vec<Assignment>, CompileError> {
    let target: HashMap<Symbol, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, (s, _))| (*s, i))
        .collect();
    let deps: Vec<Vec<usize>> = entries
        .iter()
        .map(|(_, e)| {
            e.symbols()
                .into_iter()
                .filter_map(|s| target.get(&s).copied())
                .collect()
        })
        .collect();
    let positions = order(&deps).map_err(|cycle| {
        let names: Vec<String> = cycle.iter().map(|&i| ids(entries[i].0)).collect();
        invalid(format!("a cycle of {what}: {}", names.join(", ")))
    })?;
    let mut entries: Vec<Option<(Symbol, Expr)>> = entries.into_iter().map(Some).collect();
    Ok(positions
        .into_iter()
        .map(|i| {
            let (target, expr) = entries[i].take().expect("each position once");
            Assignment { target, expr }
        })
        .collect())
}

impl<'m> Builder<'m> {
    /// The amounts of the species given in the other quantity, and the concentrations of the
    /// species of a rule given as an amount, unless an initial assignment or an assignment rule
    /// sets them.
    pub(super) fn initial_values(&self) -> Result<Vec<(Symbol, Expr)>, CompileError> {
        let m = self.m;
        let assigned: Vec<&str> = initial_assignments_with_math(m)
            .map(|(_, a)| a.symbol.as_str())
            .collect();
        let mut out = Vec::new();
        for (s, info) in m.species.iter().zip(&self.species) {
            if assigned.contains(&s.id.as_str()) || self.assignment_rule.contains_key(s.id.as_str())
            {
                continue;
            }
            let c = Expr::Symbol(info.compartment);
            let e = match (info.value, s.initial_amount, s.initial_concentration) {
                (SpeciesValue::Amount, None, Some(x)) => {
                    Expr::apply(Op::Times, vec![Expr::Number(x), c])
                }
                (SpeciesValue::Concentration, Some(x), None) => {
                    Expr::apply(Op::Divide, vec![Expr::Number(x), c])
                }
                _ => continue,
            };
            out.push((info.symbol, e));
        }
        Ok(out)
    }

    /// The initial assignments, in the quantity their targets store.
    pub(super) fn initial_assignments(&mut self) -> Result<Vec<(Symbol, Expr)>, CompileError> {
        let m = self.m;
        let mut out = Vec::new();
        for (position, ia) in initial_assignments_with_math(m) {
            let s = ia.symbol.as_str();
            let element = name(ia.id.as_ref(), "initialAssignment", position, Some(s));
            let math = ia.math.as_ref().expect("an initial assignment with math");
            let value = expr::build(math, self, &Scope::global(&element))?;
            let e = match self.items[s] {
                Item::Species(i) => {
                    let info = self.species[i];
                    if !info.has_only_substance_units && info.value == SpeciesValue::Amount {
                        Expr::apply(Op::Times, vec![value, Expr::Symbol(info.compartment)])
                    } else {
                        value
                    }
                }
                _ => value,
            };
            out.push((self.symbols[s], e));
        }
        Ok(out)
    }

    /// The event `e` of its trigger `t` with the math `math`; a delay or a priority without math
    /// is none and an event assignment without math is ignored, as libroadrunner does.
    pub(super) fn event(
        &mut self,
        position: usize,
        e: &'m SbmlEvent,
        t: &'m Trigger,
        math: &'m Node,
    ) -> Result<Event, CompileError> {
        let event = event_name(position, e);
        let part = |id: &Option<String>, kind: &str| {
            id.clone().unwrap_or_else(|| format!("{kind} of {event}"))
        };
        let trigger = expr::build(math, self, &Scope::global(&part(&t.id, "trigger")))?;
        let mut optional = |math: &Option<Node>, id: &Option<String>, kind: &str| {
            math.as_ref()
                .map(|n| expr::build(n, self, &Scope::global(&part(id, kind))))
                .transpose()
        };
        let delay = optional(&e.delay, &e.delay_id, "delay")?;
        let priority = optional(&e.priority, &e.priority_id, "priority")?;
        let mut assignments = Vec::new();
        for (i, a) in e.assignments.iter().enumerate() {
            let Some(math) = &a.math else {
                continue;
            };
            let v = a.variable.as_str();
            let element =
                a.id.clone()
                    .unwrap_or_else(|| format!("eventAssignment #{i} ({v}) of {event}"));
            if !self.is_entity(v) {
                return Err(invalid(format!(
                    "the variable {v} of {element} is no compartment, species, parameter or \
                     species reference"
                )));
            }
            if self.is_constant(v) || self.assignment_rule.contains_key(v) {
                return Err(invalid(format!(
                    "the variable {v} of {element} is constant or the variable of an \
                     assignment rule"
                )));
            }
            let value = expr::build(math, self, &Scope::global(&element))?;
            let compartment = match self.items[v] {
                Item::Species(s) => {
                    let info = self.species[s];
                    (!info.has_only_substance_units && info.value == SpeciesValue::Amount)
                        .then_some(info.compartment)
                }
                _ => None,
            };
            assignments.push(EventAssignment {
                target: self.symbols[v],
                value,
                compartment,
            });
        }
        Ok(Event {
            id: e.id.clone(),
            trigger,
            initial_value: t.initial_value,
            persistent: t.persistent,
            use_values_from_trigger_time: e.use_values_from_trigger_time,
            delay,
            priority,
            assignments,
        })
    }

    pub(super) fn selections(&mut self) -> Result<Vec<SelectionDef>, CompileError> {
        let m = self.m;
        let units = Units::new(m);
        let mut out = Vec::new();
        let entity = |id: &str, kind, unit| SelectionDef {
            id: id.to_string(),
            kind,
            expr: Expr::Symbol(self.symbols[id]),
            unit,
        };
        for c in &m.compartments {
            out.push(entity(
                &c.id,
                SelectionKind::Compartment,
                units.compartment(c),
            ));
        }
        let mut amounts = Vec::new();
        let mut concentrations = Vec::new();
        for (s, info) in m.species.iter().zip(&self.species) {
            let (x, c) = (Expr::Symbol(info.symbol), Expr::Symbol(info.compartment));
            let (amount, concentration) = match info.value {
                SpeciesValue::Amount => (x.clone(), Expr::apply(Op::Divide, vec![x, c])),
                SpeciesValue::Concentration => (Expr::apply(Op::Times, vec![x.clone(), c]), x),
            };
            amounts.push(SelectionDef {
                id: s.id.clone(),
                kind: SelectionKind::Amount,
                expr: amount,
                unit: units.substance(s),
            });
            concentrations.push(SelectionDef {
                id: format!("[{}]", s.id),
                kind: SelectionKind::Concentration,
                expr: concentration,
                unit: units.concentration(s, Some(&m.compartments[info.compartment_index])),
            });
        }
        out.extend(amounts);
        out.extend(concentrations);
        for p in &m.parameters {
            out.push(entity(
                &p.id,
                SelectionKind::Parameter,
                units.parameter(p.units.as_ref()),
            ));
        }
        for r in &self.refs {
            if let Some(id) = &r.reference.id {
                out.push(entity(id, SelectionKind::SpeciesReference, None));
            }
        }
        for (j, r) in m.reactions.iter().enumerate() {
            out.push(SelectionDef {
                id: r.id.clone(),
                kind: SelectionKind::ReactionRate,
                expr: self.reaction_rate(j)?,
                unit: units.reaction(),
            });
        }
        Ok(out)
    }
}
