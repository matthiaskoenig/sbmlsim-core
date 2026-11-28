//! The rates of the reactions and the states, and the resolution of identifiers.

use super::*;

impl Builder<'_> {
    /// The value of a species as the math sees it.
    pub(super) fn species_value(&self, i: usize) -> Expr {
        let s = self.species[i];
        if s.has_only_substance_units || s.value == SpeciesValue::Concentration {
            Expr::Symbol(s.symbol)
        } else {
            Expr::apply(
                Op::Divide,
                vec![Expr::Symbol(s.symbol), Expr::Symbol(s.compartment)],
            )
        }
    }

    /// Marks the rate of `id` as being built, an error naming the cycle when it is already.
    pub(super) fn enter(&mut self, id: &str) -> Result<(), CompileError> {
        if let Some(start) = self.busy.iter().position(|b| b == id) {
            return Err(invalid(format!(
                "a cycle of rates through rateOf: {}",
                self.busy[start..].join(", ")
            )));
        }
        self.busy.push(id.to_string());
        Ok(())
    }

    /// The rate of a reaction, the math of its kinetic law with its local parameters.
    pub(super) fn reaction_rate(&mut self, j: usize) -> Result<Expr, CompileError> {
        if let Some(e) = self.reaction_rates.get(&j) {
            return Ok(e.clone());
        }
        let r = &self.m.reactions[j];
        let Some(law) = &r.kinetic_law else {
            // libroadrunner: a reaction without a kinetic law has the rate 0
            return Ok(Expr::Number(0.0));
        };
        let Some(math) = &law.math else {
            return Err(unsupported("kinetic law without math", &r.id));
        };
        self.enter(&r.id)?;
        let locals: HashMap<String, f64> = law
            .local_parameters
            .iter()
            // libroadrunner: a local parameter without a value is NaN
            .map(|p| (p.id.clone(), p.value.unwrap_or(f64::NAN)))
            .collect();
        let element = law
            .id
            .clone()
            .unwrap_or_else(|| format!("kineticLaw of {}", r.id));
        let scope = Scope {
            element: &element,
            locals: Some(&locals),
            bvars: None,
            inlining: &[],
        };
        let e = expr::build(math, self, &scope)?;
        self.busy.pop();
        self.reaction_rates.insert(j, e.clone());
        Ok(e)
    }

    /// The rate of the state `k`: its rate rule, or the amount rate of its species.
    pub(super) fn state_rate(&mut self, k: usize) -> Result<Expr, CompileError> {
        if let Some(e) = self.state_rates.get(&k) {
            return Ok(e.clone());
        }
        let m = self.m;
        let id = self.state_ids[k];
        self.enter(id)?;
        let e = match self.sources[k] {
            Source::RateRule(position) => {
                let rule = &m.rules[position];
                let Rule::Rate {
                    math: Some(math), ..
                } = rule
                else {
                    unreachable!("a rate rule with math")
                };
                let element = rule_name(position, rule);
                expr::build(math, self, &Scope::global(&element))?
            }
            Source::Reactions(i) => {
                let mut terms = Vec::new();
                // the references of the species, in their order (an index, not a scan of every
                // reference per state, which is quadratic in the size of the model)
                let positions = self.refs_of.get(id).cloned().unwrap_or_default();
                for r in positions {
                    let (reaction, reference, product) = {
                        let r = &self.refs[r];
                        (r.reaction, r.reference, r.product)
                    };
                    let stoichiometry = match &reference.id {
                        Some(sid) => Expr::Symbol(self.symbols[sid.as_str()]),
                        // libroadrunner: a missing stoichiometry is 1
                        None => Expr::Number(reference.stoichiometry.unwrap_or(1.0)),
                    };
                    let v = self.reaction_rate(reaction)?;
                    let term = Expr::apply(Op::Times, vec![stoichiometry, v]);
                    terms.push(if product {
                        term
                    } else {
                        Expr::apply(Op::Minus, vec![term])
                    });
                }
                let rate = sum(terms);
                let s = &m.species[i];
                match s
                    .conversion_factor
                    .as_ref()
                    .or(m.conversion_factor.as_ref())
                {
                    Some(cf) => {
                        let factor = self.identifier(cf, &s.id)?;
                        Expr::apply(Op::Times, vec![factor, rate])
                    }
                    None => rate,
                }
            }
        };
        self.busy.pop();
        self.state_rates.insert(k, e.clone());
        Ok(e)
    }

    /// Whether `id` changes in time other than by events: the target of a rule.
    pub(super) fn changes_continuously(&self, id: &str) -> bool {
        self.assignment_rule.contains_key(id) || self.rate_rule.contains_key(id)
    }
}

impl Context for Builder<'_> {
    fn identifier(&mut self, id: &str, element: &str) -> Result<Expr, CompileError> {
        match self.items.get(id).copied() {
            Some(Item::Species(i)) => Ok(self.species_value(i)),
            Some(Item::Compartment(_) | Item::Parameter(_) | Item::SpeciesReference(_)) => {
                Ok(Expr::Symbol(self.symbols[id]))
            }
            Some(Item::Reaction(j)) => self.reaction_rate(j),
            Some(Item::Function(_)) => Err(invalid(format!(
                "the function definition {id} is used as a value in {element}"
            ))),
            Some(Item::Event) | None => {
                Err(invalid(format!("unknown identifier {id} in {element}")))
            }
        }
    }

    fn note_delay(&mut self) {
        self.delay_seen = true;
    }

    fn has_prehistory(&self) -> bool {
        self.prehistory
    }

    fn rate_of(&mut self, id: &str, element: &str) -> Result<Expr, CompileError> {
        self.rate_of_seen = true;
        match self.items.get(id).copied() {
            Some(Item::Reaction(_)) => return Err(unsupported("rateOf of a reaction", id)),
            Some(Item::Function(_) | Item::Event) | None => {
                return Err(invalid(format!(
                    "unknown identifier {id} of rateOf in {element}"
                )));
            }
            _ => {}
        }
        if self.assignment_rule.contains_key(id) {
            // libroadrunner: "Unable to define the rateOf for symbol ... as it is changed by an
            // assignment rule"
            return Err(unsupported("rateOf of an assignment rule target", id));
        }
        if self.algebraic.contains(id) {
            return Err(unsupported("rateOf of an algebraic variable", id));
        }
        let species = match self.items[id] {
            Item::Species(i) => Some(self.species[i]),
            _ => None,
        };
        let concentration = species.filter(|s| !s.has_only_substance_units);
        if let Some(s) = concentration
            && self
                .algebraic
                .contains(self.m.compartments[s.compartment_index].id.as_str())
        {
            return Err(unsupported(
                "rateOf of a concentration in an algebraic compartment",
                id,
            ));
        }
        if !self.rate_of.iter().any(|r| r == id) {
            self.rate_of.push(id.to_string());
        }
        match self.symbols[id] {
            Symbol::State(k) => {
                let k = k as usize;
                let rate = self.state_rate(k)?;
                let Some(s) =
                    concentration.filter(|_| matches!(self.sources[k], Source::Reactions(_)))
                else {
                    // a rate rule gives the rate of the value it stores, the value the math
                    // sees; an amount is its amount
                    return Ok(rate);
                };
                let c = Expr::Symbol(s.compartment);
                let c_id = self.m.compartments[s.compartment_index].id.as_str();
                match s.compartment {
                    Symbol::State(kc) => {
                        // the quotient rule as libroadrunner 2.11 computes it (probed bitwise):
                        // n' / V - (n / V) * V' / V
                        let dv = self.state_rate(kc as usize)?;
                        let value =
                            Expr::apply(Op::Divide, vec![Expr::Symbol(s.symbol), c.clone()]);
                        Ok(Expr::apply(
                            Op::Minus,
                            vec![
                                Expr::apply(Op::Divide, vec![rate, c.clone()]),
                                Expr::apply(
                                    Op::Divide,
                                    vec![Expr::apply(Op::Times, vec![value, dv]), c],
                                ),
                            ],
                        ))
                    }
                    _ if self.assignment_rule.contains_key(c_id) => Err(unsupported(
                        "rateOf of a concentration species in a compartment of an assignment \
                         rule",
                        id,
                    )),
                    _ => Ok(Expr::apply(Op::Divide, vec![rate, c])),
                }
            }
            Symbol::Param(_) => {
                if let Some(s) = concentration {
                    let c_id = self.m.compartments[s.compartment_index].id.as_str();
                    if self.changes_continuously(c_id) {
                        // libroadrunner gives 0 although the concentration changes with the
                        // compartment
                        return Err(unsupported(
                            "rateOf of a concentration species which reactions do not change \
                             in a variable compartment",
                            id,
                        ));
                    }
                }
                Ok(Expr::Number(0.0))
            }
            Symbol::Time => unreachable!("no entity is time"),
            Symbol::Derivative(_) => unreachable!("no entity is a derivative of a Jacobian"),
            Symbol::Common(_) | Symbol::Constant(_) => {
                unreachable!("no entity is a symbol of the optimizer")
            }
        }
    }

    fn function(&self, id: &str) -> Option<Option<Node>> {
        match self.items.get(id) {
            Some(Item::Function(i)) => Some(self.m.function_definitions[*i].math.clone()),
            _ => None,
        }
    }
}
