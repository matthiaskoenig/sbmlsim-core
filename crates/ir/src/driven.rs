//! Driven species: species whose value follows a prescribed path instead of their rate.
//!
//! [`apply`] is a pass on a built [`Model`], before [`optimize`](crate::optimize) and
//! [`derive`](mod@crate::derive), so every target of the model representation shares it. For
//! each driven species `X` it
//!
//! - removes `X` from the states (and its rate) and adds the parameters `X__value` and
//!   `X__slope`, and once for all driven species `driven__t_ref`;
//! - reads, wherever the math read `X`, the path `X__value + X__slope * (time - driven__t_ref)`:
//!   a concentration in the compartment of `X`, so the amount of `X` is the path times the
//!   compartment and its concentration the path itself, or the amount of a species with
//!   `hasOnlySubstanceUnits`, so its concentration is the path over the compartment;
//! - adds the state `X__exchange` (an amount, initially 0) behind the differential states, whose
//!   rate is the amount rate `X` had (stoichiometry times rate, times the conversion factor),
//!   with the path substituted: over an interval its change is the amount the model gave to
//!   `X` (positive) or took from it (negative).
//!
//! The initial value of `X__value` is the initial value of `X` in the quantity of the path
//! (an initial assignment of `X` or an initial amount of a species of a concentration becomes
//! an initial evaluation of `X__value`); `X__slope` and `driven__t_ref` are 0. Every symbol the
//! pass introduces is a [`Symbol::Param`] or a [`Symbol::State`], so the optimizer, the
//! derivatives and the emitters see an ordinary model, whose rates read the time.
//!
//! Refused (`Unsupported`, the element the id): an unknown id, an id which is no species, a
//! species which is a boundary condition or constant, the target of a rate, assignment or
//! algebraic rule or of an event assignment, a species in a compartment whose size a rule or
//! an event changes, a species whose rate `rateOf` reads or which a `delay` reads (the path is
//! known for the current interval only), a species given twice and a species whose new ids are
//! taken.

use crate::expr::MAX_EXPR_DEPTH;
use crate::units::quotient;
use crate::{
    Assignment, CompileError, DrivenSpecies, Entity, EntityKind, Expr, Model, Op, SelectionDef,
    SelectionKind, SpeciesValue, Symbol, index,
};

/// The id of the parameter of the reference time of the paths.
pub const T_REF: &str = "driven__t_ref";

/// The id of the parameter of the value of the path of the species `id`.
pub fn value_id(id: &str) -> String {
    format!("{id}__value")
}

/// The id of the parameter of the slope of the path of the species `id`.
pub fn slope_id(id: &str) -> String {
    format!("{id}__slope")
}

/// The id of the accumulator state of the species `id`.
pub fn exchange_id(id: &str) -> String {
    format!("{id}__exchange")
}

fn refuse(feature: &str, id: &str) -> CompileError {
    CompileError::Unsupported {
        feature: format!("driving {feature}"),
        element: id.to_string(),
    }
}

/// A species to drive, as the checks found it.
struct Target {
    /// Its index in the states.
    state: usize,
    /// Its compartment, a parameter; `None` for a species with `hasOnlySubstanceUnits`, whose
    /// path is an amount.
    compartment: Option<Symbol>,
}

/// Whether an expression reads `symbol` within the operand of a `delay`.
fn delayed(e: &Expr, symbol: Symbol, inside: bool) -> bool {
    match e {
        Expr::Number(_) | Expr::Bool(_) => false,
        Expr::Symbol(s) => inside && *s == symbol,
        Expr::Apply { op, args } => {
            let inside = inside || matches!(op, Op::Delay | Op::HistoryQuery);
            args.iter().any(|a| delayed(a, symbol, inside))
        }
        Expr::Piecewise { pieces, otherwise } => {
            pieces
                .iter()
                .any(|(v, c)| delayed(v, symbol, inside) || delayed(c, symbol, inside))
                || otherwise
                    .as_deref()
                    .is_some_and(|o| delayed(o, symbol, inside))
        }
    }
}

/// Every expression of a model.
fn expressions(m: &Model) -> impl Iterator<Item = &Expr> {
    m.rates
        .iter()
        .chain(m.assignment_rules.iter().map(|a| &a.expr))
        .chain(m.initial.iter().map(|a| &a.expr))
        .chain(m.events.iter().flat_map(|e| {
            std::iter::once(&e.trigger)
                .chain(&e.delay)
                .chain(&e.priority)
                .chain(e.assignments.iter().map(|a| &a.value))
        }))
        .chain(m.selections.iter().map(|s| &s.expr))
}

/// Whether a rule or an event changes `symbol` (a compartment) in time.
fn changes(m: &Model, symbol: Symbol) -> bool {
    matches!(symbol, Symbol::State(_))
        || m.assignment_rules.iter().any(|a| a.target == symbol)
        || m.events
            .iter()
            .any(|e| e.assignments.iter().any(|a| a.target == symbol))
}

/// The checks of driving `id`, the `position`-th of `driven`.
fn check(m: &Model, driven: &[String], position: usize) -> Result<Target, CompileError> {
    let id = driven[position].as_str();
    if driven[..position].iter().any(|d| d == id) {
        return Err(refuse("a species twice", id));
    }
    let Some(symbol) = m.symbol(id) else {
        return Err(refuse("an unknown id", id));
    };
    let EntityKind::Species {
        compartment,
        has_only_substance_units,
        boundary_condition,
        constant,
        value,
    } = m.entity(symbol).kind
    else {
        return Err(refuse("an id which is no species", id));
    };
    if boundary_condition {
        return Err(refuse("a boundary species", id));
    }
    if constant {
        return Err(refuse("a constant species", id));
    }
    if m.rate_rules.iter().any(|r| r == id) {
        return Err(refuse("the target of a rate rule", id));
    }
    if m.assignment_rules.iter().any(|a| a.target == symbol) {
        return Err(refuse("the target of an assignment rule", id));
    }
    let n_differential = m.states.len() - m.n_algebraic as usize;
    if matches!(symbol, Symbol::State(k) if k as usize >= n_differential) {
        return Err(refuse("the unknown of an algebraic rule", id));
    }
    if m.events
        .iter()
        .any(|e| e.assignments.iter().any(|a| a.target == symbol))
    {
        return Err(refuse("the target of an event assignment", id));
    }
    if changes(m, compartment) {
        return Err(refuse(
            &format!(
                "a species in the compartment {}, whose size a rule or an event changes",
                m.entity(compartment).id
            ),
            id,
        ));
    }
    if m.rate_of.iter().any(|r| r == id) {
        return Err(refuse("a species whose rate rateOf reads", id));
    }
    if expressions(m).any(|e| delayed(e, symbol, false)) {
        return Err(refuse("a species which a delay reads", id));
    }
    for taken in [
        value_id(id),
        slope_id(id),
        exchange_id(id),
        T_REF.to_string(),
    ] {
        // an entity, or a reaction (whose rate is a selection)
        if m.symbol(&taken).is_some() || m.selection(&taken).is_some() || driven.contains(&taken) {
            return Err(refuse(
                &format!("a species whose id {taken} is taken by the model"),
                id,
            ));
        }
    }
    let Symbol::State(k) = symbol else {
        unreachable!(
            "a species which is neither constant nor a boundary condition nor the target of a rule is a state"
        );
    };
    debug_assert_eq!(
        value,
        SpeciesValue::Amount,
        "a species of no rule stores its amount"
    );
    Ok(Target {
        state: k as usize,
        compartment: (!has_only_substance_units).then_some(compartment),
    })
}

/// What a state of the model before the pass becomes.
#[derive(Clone)]
enum Replacement {
    /// The state of this index after the pass.
    State(u32),
    /// The path of a driven species, in the compartment (a concentration) or an amount.
    Path {
        path: Expr,
        compartment: Option<Symbol>,
    },
}

struct Substitution {
    states: Vec<Replacement>,
}

impl Substitution {
    /// The symbol after the pass of a symbol which is no driven species (a target).
    fn symbol(&self, s: Symbol) -> Symbol {
        match s {
            Symbol::State(i) => match &self.states[i as usize] {
                Replacement::State(j) => Symbol::State(*j),
                Replacement::Path { .. } => unreachable!("a driven species is no target"),
            },
            s => s,
        }
    }

    /// The expression after the pass: the paths of the driven species in place of their
    /// amounts, a concentration `amount / compartment` the path itself.
    fn expr(&self, e: &Expr) -> Expr {
        match e {
            Expr::Number(_) | Expr::Bool(_) => e.clone(),
            Expr::Symbol(Symbol::State(i)) => match &self.states[*i as usize] {
                Replacement::State(j) => Expr::Symbol(Symbol::State(*j)),
                Replacement::Path {
                    path,
                    compartment: None,
                } => path.clone(),
                Replacement::Path {
                    path,
                    compartment: Some(c),
                } => Expr::apply(Op::Times, vec![path.clone(), Expr::Symbol(*c)]),
            },
            Expr::Symbol(_) => e.clone(),
            Expr::Apply { op, args } => {
                if let (Op::Divide, [Expr::Symbol(Symbol::State(i)), Expr::Symbol(c)]) =
                    (op, args.as_slice())
                    && let Replacement::Path {
                        path,
                        compartment: Some(own),
                    } = &self.states[*i as usize]
                    && own == c
                {
                    return path.clone();
                }
                Expr::apply(*op, args.iter().map(|a| self.expr(a)).collect())
            }
            Expr::Piecewise { pieces, otherwise } => Expr::Piecewise {
                pieces: pieces
                    .iter()
                    .map(|(v, c)| (self.expr(v), self.expr(c)))
                    .collect(),
                otherwise: otherwise.as_deref().map(|o| Box::new(self.expr(o))),
            },
        }
    }
}

/// An amount of a driven species in the quantity of its path: the amount itself, or over the
/// compartment (`value * compartment` the value itself).
fn in_path_quantity(amount: Expr, compartment: Option<Symbol>) -> Expr {
    let Some(c) = compartment else {
        return amount;
    };
    match amount {
        Expr::Apply {
            op: Op::Times,
            mut args,
        } if args.len() == 2 && args[1] == Expr::Symbol(c) => {
            args.truncate(1);
            args.pop().expect("one argument")
        }
        amount => Expr::apply(Op::Divide, vec![amount, Expr::Symbol(c)]),
    }
}

fn parameter(id: String, default: f64, unit: Option<String>) -> Entity {
    Entity {
        id,
        kind: EntityKind::Parameter,
        default,
        settable: true,
        unit,
    }
}

/// The model with the species `driven` (SBML ids, in this order) driven (see the module
/// documentation); without driven species the model as it is.
pub fn apply(model: Model, driven: &[String]) -> Result<Model, CompileError> {
    if driven.is_empty() {
        return Ok(model);
    }
    let targets = (0..driven.len())
        .map(|position| check(&model, driven, position))
        .collect::<Result<Vec<_>, _>>()?;
    let mut m = model;

    let n_states = m.states.len();
    let n_differential = n_states - m.n_algebraic as usize;
    let n_driven = targets.len();
    let first_parameter = m.parameters.len();
    let value = |j: usize| index(first_parameter + 2 * j);
    let slope = |j: usize| index(first_parameter + 2 * j + 1);
    let t_ref = index(first_parameter + 2 * n_driven);
    // the states after the pass: the differential states but the driven species, the
    // accumulators, the algebraic states
    let mut replacements = vec![Replacement::State(0); n_states];
    for (j, t) in targets.iter().enumerate() {
        let time = Expr::apply(
            Op::Minus,
            vec![
                Expr::Symbol(Symbol::Time),
                Expr::Symbol(Symbol::Param(t_ref)),
            ],
        );
        let path = Expr::apply(
            Op::Plus,
            vec![
                Expr::Symbol(Symbol::Param(value(j))),
                Expr::apply(Op::Times, vec![Expr::Symbol(Symbol::Param(slope(j))), time]),
            ],
        );
        replacements[t.state] = Replacement::Path {
            path,
            compartment: t.compartment,
        };
    }
    let is_driven = |k: usize| targets.iter().any(|t| t.state == k);
    let mut kept = Vec::new();
    for k in (0..n_differential).filter(|&k| !is_driven(k)) {
        replacements[k] = Replacement::State(index(kept.len()));
        kept.push(k);
    }
    let first_exchange = kept.len();
    // as many accumulators as driven species: the algebraic states keep their indices
    for (k, r) in replacements.iter_mut().enumerate().skip(n_differential) {
        *r = Replacement::State(index(k));
    }
    let sub = Substitution {
        states: replacements,
    };

    // the entities: the kinds keep their compartments, which may be states
    let remap = |e: &Entity| {
        let mut e = e.clone();
        if let EntityKind::Species { compartment, .. } = &mut e.kind {
            *compartment = sub.symbol(*compartment);
        }
        e
    };
    let old_states = std::mem::take(&mut m.states);
    let old_rates = std::mem::take(&mut m.rates);
    let mut states: Vec<Entity> = kept.iter().map(|&k| remap(&old_states[k])).collect();
    let mut rates: Vec<Expr> = kept.iter().map(|&k| sub.expr(&old_rates[k])).collect();
    for (j, t) in targets.iter().enumerate() {
        let x = &old_states[t.state];
        states.push(parameter(exchange_id(&driven[j]), 0.0, x.unit.clone()));
        rates.push(sub.expr(&old_rates[t.state]));
    }
    for k in n_differential..n_states {
        states.push(remap(&old_states[k]));
        rates.push(sub.expr(&old_rates[k]));
    }
    m.states = states;
    m.rates = rates;
    m.parameters = m.parameters.iter().map(remap).collect();

    m.assignment_rules = m
        .assignment_rules
        .iter()
        .map(|a| Assignment {
            target: sub.symbol(a.target),
            expr: sub.expr(&a.expr),
        })
        .collect();

    // the initial evaluations: those of a driven species give the value of its path, an
    // evaluation of a number its default
    let mut defaults: Vec<Option<f64>> = vec![None; n_driven];
    let mut initial = Vec::with_capacity(m.initial.len() + n_driven);
    for a in &m.initial {
        let expr = sub.expr(&a.expr);
        match targets
            .iter()
            .position(|t| a.target == Symbol::State(index(t.state)))
        {
            Some(j) => match in_path_quantity(expr, targets[j].compartment) {
                Expr::Number(x) => defaults[j] = Some(x),
                expr => {
                    defaults[j] = Some(f64::NAN);
                    initial.push(Assignment {
                        target: Symbol::Param(value(j)),
                        expr,
                    });
                }
            },
            None => initial.push(Assignment {
                target: sub.symbol(a.target),
                expr,
            }),
        }
    }
    for (j, t) in targets.iter().enumerate() {
        if defaults[j].is_some() {
            continue;
        }
        let amount = old_states[t.state].default;
        defaults[j] = Some(match t.compartment {
            None => amount,
            // an initial amount of a species of a concentration: its concentration once the
            // compartment has its initial value, right after its evaluation if it has one
            Some(c) if amount != 0.0 && !amount.is_nan() => {
                let at = initial
                    .iter()
                    .position(|a| a.target == c)
                    .map_or(0, |i| i + 1);
                initial.insert(
                    at,
                    Assignment {
                        target: Symbol::Param(value(j)),
                        expr: Expr::apply(Op::Divide, vec![Expr::Number(amount), Expr::Symbol(c)]),
                    },
                );
                f64::NAN
            }
            // 0 or NaN in either quantity
            Some(_) => amount,
        });
    }
    m.initial = initial;

    for e in &mut m.events {
        e.trigger = sub.expr(&e.trigger);
        e.delay = e.delay.as_ref().map(|d| sub.expr(d));
        e.priority = e.priority.as_ref().map(|p| sub.expr(p));
        for a in &mut e.assignments {
            a.target = sub.symbol(a.target);
            a.value = sub.expr(&a.value);
            a.compartment = a.compartment.map(|c| sub.symbol(c));
        }
    }
    for s in &mut m.selections {
        s.expr = sub.expr(&s.expr);
    }

    // the new parameters, and the selections of the new entities behind the parameters
    let mut selected = Vec::new();
    for (j, t) in targets.iter().enumerate() {
        let id = &driven[j];
        let x = &old_states[t.state];
        let unit = match t.compartment {
            None => x.unit.clone(),
            Some(_) => m.selection(&format!("[{id}]")).and_then(|s| s.unit.clone()),
        };
        // the slope in the unit of the value per time
        let slope_unit = unit
            .as_deref()
            .zip(m.time_unit.as_deref())
            .map(|(value, time)| quotient(value, time));
        let default = defaults[j].expect("a default of every driven species");
        m.parameters
            .push(parameter(value_id(id), default, unit.clone()));
        m.parameters
            .push(parameter(slope_id(id), 0.0, slope_unit.clone()));
        selected.push((value_id(id), Symbol::Param(value(j)), unit));
        selected.push((slope_id(id), Symbol::Param(slope(j)), slope_unit));
    }
    m.parameters
        .push(parameter(T_REF.to_string(), 0.0, m.time_unit.clone()));
    selected.push((T_REF.to_string(), Symbol::Param(t_ref), m.time_unit.clone()));
    for (j, t) in targets.iter().enumerate() {
        selected.push((
            exchange_id(&driven[j]),
            Symbol::State(index(first_exchange + j)),
            old_states[t.state].unit.clone(),
        ));
    }
    let at = m
        .selections
        .iter()
        .position(|s| {
            matches!(
                s.kind,
                SelectionKind::SpeciesReference | SelectionKind::ReactionRate
            )
        })
        .unwrap_or(m.selections.len());
    m.selections.splice(
        at..at,
        selected.into_iter().map(|(id, symbol, unit)| SelectionDef {
            id,
            kind: SelectionKind::Parameter,
            expr: Expr::Symbol(symbol),
            unit,
        }),
    );

    m.driven = targets
        .iter()
        .enumerate()
        .map(|(j, t)| DrivenSpecies {
            id: driven[j].clone(),
            value: value(j),
            slope: slope(j),
            exchange: index(first_exchange + j),
            amount: t.compartment.is_none(),
        })
        .collect();
    m.t_ref = Some(t_ref);

    if expressions(&m).any(|e| e.depth() > MAX_EXPR_DEPTH) {
        return Err(CompileError::Invalid(format!(
            "a math which reads a driven species of {} is nested deeper than {MAX_EXPR_DEPTH} \
             levels with its path",
            driven.join(", ")
        )));
    }
    Ok(m)
}
