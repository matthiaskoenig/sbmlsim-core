//! The ids of the model, the checks of the targets and the states and parameters.

use super::*;

impl<'m> Builder<'m> {
    pub(super) fn new(m: &'m Sbml) -> Result<Builder<'m>, CompileError> {
        let mut refs = Vec::new();
        for (j, r) in m.reactions.iter().enumerate() {
            for (list, product) in [(&r.reactants, false), (&r.products, true)] {
                for reference in list {
                    refs.push(Ref {
                        reaction: j,
                        reference,
                        product,
                    });
                }
            }
        }
        let mut items = HashMap::new();
        let mut add = |id: &'m str, item: Item| {
            if items.insert(id, item).is_some() {
                Err(invalid(format!("the id {id} is used more than once")))
            } else {
                Ok(())
            }
        };
        for (i, f) in m.function_definitions.iter().enumerate() {
            add(&f.id, Item::Function(i))?;
        }
        for (i, c) in m.compartments.iter().enumerate() {
            add(&c.id, Item::Compartment(i))?;
        }
        for (i, s) in m.species.iter().enumerate() {
            add(&s.id, Item::Species(i))?;
        }
        for (i, p) in m.parameters.iter().enumerate() {
            add(&p.id, Item::Parameter(i))?;
        }
        for (j, r) in m.reactions.iter().enumerate() {
            add(&r.id, Item::Reaction(j))?;
        }
        for (i, r) in refs.iter().enumerate() {
            if let Some(id) = &r.reference.id {
                add(id, Item::SpeciesReference(i))?;
            }
        }
        // the id of an event which is ignored (no trigger with math) is not registered, as
        // libroadrunner 2.11 does not: it may repeat another id
        for (_, e, _, _) in events_with_trigger(m) {
            if let Some(id) = &e.id {
                add(id, Item::Event)?;
            }
        }
        for r in &m.reactions {
            for s in r
                .reactants
                .iter()
                .chain(&r.products)
                .map(|x| &x.species)
                .chain(r.modifiers.iter().map(|x| &x.species))
            {
                if !matches!(items.get(s.as_str()), Some(Item::Species(_))) {
                    return Err(invalid(format!(
                        "the reaction {} refers to {s}, which is no species",
                        r.id
                    )));
                }
            }
        }
        Ok(Builder {
            m,
            items,
            refs,
            symbols: HashMap::new(),
            assignment_rule: HashMap::new(),
            rate_rule: HashMap::new(),
            species: Vec::new(),
            state_ids: Vec::new(),
            sources: Vec::new(),
            reaction_rates: HashMap::new(),
            state_rates: HashMap::new(),
            busy: Vec::new(),
        })
    }

    /// Whether `id` is constant by its SBML attribute.
    pub(super) fn is_constant(&self, id: &str) -> bool {
        let m = self.m;
        match self.items.get(id) {
            Some(Item::Compartment(i)) => m.compartments[*i].constant,
            Some(Item::Species(i)) => m.species[*i].constant,
            Some(Item::Parameter(i)) => m.parameters[*i].constant,
            Some(Item::SpeciesReference(i)) => self.refs[*i].reference.constant,
            _ => false,
        }
    }

    pub(super) fn is_entity(&self, id: &str) -> bool {
        matches!(
            self.items.get(id),
            Some(
                Item::Compartment(_)
                    | Item::Species(_)
                    | Item::Parameter(_)
                    | Item::SpeciesReference(_)
            )
        )
    }

    /// Checks the targets of the rules and initial assignments and splits the entities into the
    /// states and the parameters.
    pub(super) fn classify(&mut self) -> Result<(), CompileError> {
        let m = self.m;
        for (position, rule) in rules_with_math(m) {
            // the rules of the same kind, and of the other kind, before it in document order
            let (variable, map, other) = match rule {
                Rule::Assignment { variable, .. } => {
                    (variable, &mut self.assignment_rule, &self.rate_rule)
                }
                Rule::Rate { variable, .. } => {
                    (variable, &mut self.rate_rule, &self.assignment_rule)
                }
                Rule::Algebraic { .. } => unreachable!("refused before"),
            };
            if other.contains_key(variable.as_str())
                || map.insert(variable.as_str(), position).is_some()
            {
                return Err(invalid(format!("{variable} is the variable of two rules")));
            }
            let element = rule_name(position, rule);
            if !self.is_entity(variable) {
                return Err(invalid(format!(
                    "the variable {variable} of {element} is no compartment, species, parameter \
                     or species reference"
                )));
            }
            if self.is_constant(variable) {
                return Err(invalid(format!(
                    "the variable {variable} of {element} is constant"
                )));
            }
        }
        let mut assigned_initially = HashMap::new();
        for (position, ia) in initial_assignments_with_math(m) {
            let s = ia.symbol.as_str();
            let element = name(ia.id.as_ref(), "initialAssignment", position, Some(s));
            if !self.is_entity(s) {
                return Err(invalid(format!(
                    "the symbol {s} of {element} is no compartment, species, parameter or \
                     species reference"
                )));
            }
            if self.assignment_rule.contains_key(s) {
                return Err(invalid(format!(
                    "{s} has an initial assignment and an assignment rule"
                )));
            }
            if assigned_initially.insert(s, position).is_some() {
                return Err(invalid(format!("{s} has two initial assignments")));
            }
        }
        // libroadrunner refuses a parameter without a value unless an initial assignment or an
        // assignment rule gives it one ("Global parameter 'p' is missing a value"), also when a
        // rate rule or an event changes it
        if let Some(p) = m.parameters.iter().find(|p| {
            p.value.is_none()
                && !assigned_initially.contains_key(p.id.as_str())
                && !self.assignment_rule.contains_key(p.id.as_str())
        }) {
            return Err(unsupported("parameter without a value", &p.id));
        }
        // a species a rule or `constant` fixes cannot be changed by reactions as well
        for r in &self.refs {
            let s = &m.species[match self.items[r.reference.species.as_str()] {
                Item::Species(i) => i,
                _ => unreachable!("checked in new"),
            }];
            let ruled = self.assignment_rule.contains_key(s.id.as_str())
                || self.rate_rule.contains_key(s.id.as_str());
            if !s.boundary_condition && (ruled || s.constant) {
                return Err(invalid(format!(
                    "the species {} is a reactant or product of {} and {} without being a \
                     boundary condition",
                    s.id,
                    m.reactions[r.reaction].id,
                    if ruled {
                        "the variable of a rule"
                    } else {
                        "constant"
                    }
                )));
            }
        }

        // the states: the rate rules in their order, then the species of the reactions
        for (position, rule) in rules_with_math(m) {
            if let Rule::Rate { variable, .. } = rule {
                self.sources.push(Source::RateRule(position));
                self.state_ids.push(variable);
            }
        }
        for (i, s) in m.species.iter().enumerate() {
            let id = s.id.as_str();
            if !s.constant
                && !s.boundary_condition
                && !self.assignment_rule.contains_key(id)
                && !self.rate_rule.contains_key(id)
            {
                self.sources.push(Source::Reactions(i));
                self.state_ids.push(id);
            }
        }
        for (k, id) in self.state_ids.iter().enumerate() {
            self.symbols.insert(id, Symbol::State(index(k)));
        }
        // the parameters: everything else
        let mut params: Vec<&'m str> = Vec::new();
        params.extend(m.compartments.iter().map(|c| c.id.as_str()));
        params.extend(m.species.iter().map(|s| s.id.as_str()));
        params.extend(m.parameters.iter().map(|p| p.id.as_str()));
        params.extend(self.refs.iter().filter_map(|r| r.reference.id.as_deref()));
        let mut n = 0;
        for id in params {
            if !self.symbols.contains_key(id) {
                self.symbols.insert(id, Symbol::Param(index(n)));
                n += 1;
            }
        }

        for s in &m.species {
            let Some(&Item::Compartment(c)) = self.items.get(s.compartment.as_str()) else {
                return Err(invalid(format!(
                    "the compartment {} of the species {} is no compartment",
                    s.compartment, s.id
                )));
            };
            if s.initial_amount.is_some() && s.initial_concentration.is_some() {
                return Err(invalid(format!(
                    "the species {} has an initial amount and an initial concentration",
                    s.id
                )));
            }
            let ruled = self.assignment_rule.contains_key(s.id.as_str())
                || self.rate_rule.contains_key(s.id.as_str());
            self.species.push(SpeciesInfo {
                symbol: self.symbols[s.id.as_str()],
                compartment: self.symbols[s.compartment.as_str()],
                compartment_index: c,
                has_only_substance_units: s.has_only_substance_units,
                value: if ruled && !s.has_only_substance_units {
                    SpeciesValue::Concentration
                } else {
                    SpeciesValue::Amount
                },
            });
        }
        Ok(())
    }

    /// The states and the parameters.
    pub(super) fn entities(&self) -> (Vec<Entity>, Vec<Entity>) {
        let m = self.m;
        let units = Units::new(m);
        let mut states = Vec::new();
        let mut params = Vec::new();
        let mut push = |id: &str, kind, default: Option<f64>, unit| {
            let entity = Entity {
                id: id.to_string(),
                kind,
                default: default.unwrap_or(f64::NAN),
                settable: !self.assignment_rule.contains_key(id),
                unit,
            };
            match self.symbols[id] {
                Symbol::State(k) => states.push((k, entity)),
                Symbol::Param(k) => params.push((k, entity)),
                Symbol::Time => unreachable!("no entity is time"),
            }
        };
        // the missing values of libroadrunner: a compartment 1, a species 0, a stoichiometry 1;
        // a parameter without a value is refused unless it is assigned (then NaN until `initial`)
        for c in &m.compartments {
            let size = Some(c.size.unwrap_or(1.0));
            push(&c.id, EntityKind::Compartment, size, units.compartment(c));
        }
        for (s, info) in m.species.iter().zip(&self.species) {
            let (default, unit) = match info.value {
                SpeciesValue::Amount => (s.initial_amount, units.substance(s)),
                SpeciesValue::Concentration => (
                    s.initial_concentration,
                    units.concentration(s, Some(&m.compartments[info.compartment_index])),
                ),
            };
            // given in the other quantity: NaN, which `initial` completes
            let given = s.initial_amount.is_some() || s.initial_concentration.is_some();
            let default = if given { default } else { Some(0.0) };
            let kind = EntityKind::Species {
                compartment: info.compartment,
                has_only_substance_units: s.has_only_substance_units,
                boundary_condition: s.boundary_condition,
                constant: s.constant,
                value: info.value,
            };
            push(&s.id, kind, default, unit);
        }
        for p in &m.parameters {
            let unit = units.parameter(p.units.as_ref());
            push(&p.id, EntityKind::Parameter, p.value, unit);
        }
        for r in &self.refs {
            if let Some(id) = &r.reference.id {
                let default = Some(r.reference.stoichiometry.unwrap_or(1.0));
                push(id, EntityKind::SpeciesReference, default, None);
            }
        }
        states.sort_by_key(|(k, _)| *k);
        params.sort_by_key(|(k, _)| *k);
        (
            states.into_iter().map(|(_, e)| e).collect(),
            params.into_iter().map(|(_, e)| e).collect(),
        )
    }
}
