//! Structural matching of algebraic rules to unknowns, before choosing storage for species.

use super::*;
use std::collections::{HashSet, VecDeque};

/// Resolve the *SBML symbols* in a formula, before amount/concentration conversion introduces
/// implicit compartment reads. The normal math builder resolves function arguments and locals.
struct Dependencies<'a, 'm> {
    builder: &'a Builder<'m>,
    ids: Vec<&'m str>,
    indices: HashMap<&'m str, usize>,
    derivatives: HashSet<&'m str>,
}

impl Context for Dependencies<'_, '_> {
    fn identifier(&mut self, id: &str, element: &str) -> Result<Expr, CompileError> {
        let &i = self
            .indices
            .get(id)
            .ok_or_else(|| invalid(format!("unknown identifier {id} in {element}")))?;
        Ok(Expr::Symbol(Symbol::Param(index(i))))
    }

    fn rate_of(&mut self, id: &str, element: &str) -> Result<Expr, CompileError> {
        let &i = self
            .indices
            .get(id)
            .ok_or_else(|| invalid(format!("unknown identifier {id} of rateOf in {element}")))?;
        self.derivatives.insert(self.ids[i]);
        if let Item::Species(s) = self.builder.items[id] {
            let species = &self.builder.m.species[s];
            if !species.has_only_substance_units {
                self.derivatives.insert(&species.compartment);
            }
        }
        // A derivative is not a free algebraic variable.
        Ok(Expr::Number(0.0))
    }

    fn function(&self, id: &str) -> Option<Option<Node>> {
        self.builder.function(id)
    }
}

impl Dependencies<'_, '_> {
    fn reads(&mut self, math: &Node, scope: &Scope) -> Result<Vec<usize>, CompileError> {
        let e = expr::build(math, self, scope)?;
        let mut reads = Vec::new();
        let mut seen = HashSet::new();
        e.visit_symbols(&mut |s| {
            if let Symbol::Param(i) = s
                && seen.insert(i)
            {
                reads.push(i as usize);
            }
        });
        Ok(reads)
    }
}

impl<'m> Builder<'m> {
    /// The unknowns for the nonempty algebraic rules, in rule order.
    pub(super) fn algebraic_unknowns(
        &self,
        initials: &HashMap<&str, usize>,
    ) -> Result<Vec<&'m str>, CompileError> {
        let m = self.m;
        if !rules_with_math(m).any(|(_, r)| matches!(r, Rule::Algebraic { .. })) {
            return Ok(Vec::new());
        }
        let mut ids: Vec<_> = self.items.keys().copied().collect();
        ids.sort_unstable();
        let indices = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        let mut deps = Dependencies {
            builder: self,
            ids,
            indices,
            derivatives: HashSet::new(),
        };
        let mut definitions = vec![Vec::new(); deps.ids.len()];
        let mut equations = Vec::new();
        for (pos, rule) in rules_with_math(m) {
            let name = rule_name(pos, rule);
            let reads = deps.reads(rule.math().unwrap(), &Scope::global(&name))?;
            match rule {
                Rule::Assignment { variable, .. } => {
                    definitions[deps.indices[variable.as_str()]] = reads
                }
                Rule::Algebraic { .. } => equations.push((name, reads)),
                Rule::Rate { .. } => {}
            }
        }
        for r in &m.reactions {
            if let Some(law) = &r.kinetic_law
                && let Some(math) = &law.math
            {
                let locals = law
                    .local_parameters
                    .iter()
                    .map(|p| (p.id.clone(), p.value.unwrap_or(f64::NAN)))
                    .collect();
                let scope = Scope {
                    locals: Some(&locals),
                    ..Scope::global(&r.id)
                };
                definitions[deps.indices[r.id.as_str()]] = deps.reads(math, &scope)?;
            }
        }
        // SBML rateOf cannot refer to a variable determined by an algebraic rule, including
        // from initial assignments and event math. Event targets are less suitable as unknowns.
        let mut event_targets = HashSet::new();
        for (_, a) in initial_assignments_with_math(m) {
            deps.reads(a.math.as_ref().unwrap(), &Scope::global(&a.symbol))?;
        }
        for (pos, e, _, trigger) in events_with_trigger(m) {
            let name = event_name(pos, e);
            for math in [Some(trigger), e.delay.as_ref(), e.priority.as_ref()]
                .into_iter()
                .flatten()
            {
                deps.reads(math, &Scope::global(&name))?;
            }
            for a in &e.assignments {
                if let Some(math) = &a.math {
                    event_targets.insert(a.variable.as_str());
                    deps.reads(math, &Scope::global(&name))?;
                }
            }
        }
        let is_eligible = |id: &str| {
            self.is_entity(id)
                && !self.is_constant(id)
                && !self.rate_rule.contains_key(id)
                && !self.assignment_rule.contains_key(id)
                && !deps.derivatives.contains(id)
                && !matches!(self.items[id], Item::Species(i) if !m.species[i].boundary_condition && self.refs_of.contains_key(id))
        };
        let initialized = |id: &str| {
            initials.contains_key(id)
                || match self.items[id] {
                    Item::Parameter(i) => m.parameters[i].value.is_some(),
                    Item::Compartment(i) => m.compartments[i].size.is_some(),
                    Item::Species(i) => {
                        m.species[i].initial_amount.is_some()
                            || m.species[i].initial_concentration.is_some()
                    }
                    Item::SpeciesReference(i) => self.refs[i].reference.stoichiometry.is_some(),
                    _ => false,
                }
        };

        let mut candidates = Vec::new();
        for (_, reads) in &equations {
            let mut seen = HashSet::new();
            let mut queue = VecDeque::from(reads.clone());
            let mut row = Vec::new();
            while let Some(i) = queue.pop_front() {
                if seen.insert(i) {
                    if is_eligible(deps.ids[i]) {
                        row.push(i);
                    } else {
                        queue.extend(&definitions[i]);
                    }
                }
            }
            candidates.push(row);
        }
        // A parameter with no initialization needs an equation. Among other choices prefer
        // independent event targets and existing initial values to remain free.
        let priority: Vec<_> = deps
            .ids
            .iter()
            .map(|&id| {
                let given = initialized(id);
                let required = matches!(self.items[id], Item::Parameter(_)) && !given;
                (!required, event_targets.contains(id), given)
            })
            .collect();
        let matched = match_rows(&candidates, &priority).map_err(|r| {
            invalid(format!(
                "overdetermined algebraic system: no independent variable for {}",
                equations[r].0
            ))
        })?;
        Ok(matched.into_iter().map(|i| deps.ids[i]).collect())
    }
}

/// Augment one rule at a time. A path may reassign earlier rules to different variables.
/// An iterative search keeps the stack bounded for large coupled systems.
fn match_rows(rows: &[Vec<usize>], priority: &[(bool, bool, bool)]) -> Result<Vec<usize>, usize> {
    let variables = priority.len();
    let mut owners = vec![None; variables];
    let mut matched = vec![None; rows.len()];
    let mut visited = vec![None; variables];
    let mut parent = vec![0; variables];
    for start in 0..rows.len() {
        let mut queue = VecDeque::from([start]);
        let mut best = None;
        // Every augmenting path adds exactly its free endpoint to the set of unknowns.
        // Choosing the preferred reachable endpoint preserves previous choices collectively,
        // even when achieving it requires reassigning several equations.
        while let Some(row) = queue.pop_front() {
            for &v in &rows[row] {
                if visited[v] == Some(start) {
                    continue;
                }
                visited[v] = Some(start);
                parent[v] = row;
                match owners[v] {
                    Some(previous) => queue.push_back(previous),
                    None if best.is_none_or(|b| priority[v] < priority[b]) => best = Some(v),
                    None => {}
                }
            }
        }
        let mut free = best.ok_or(start)?;
        loop {
            let row = parent[free];
            owners[free] = Some(row);
            match matched[row].replace(free) {
                Some(previous) => free = previous,
                None => break,
            }
        }
    }
    Ok(matched.into_iter().map(Option::unwrap).collect())
}
