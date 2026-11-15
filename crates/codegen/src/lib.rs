//! The code generation of sbmlsim-core: `model.wasm` and its [`ModelDescription`] from the
//! model representation.
//!
//! [`emit`] translates an [`sbmlsim_ir::Model`] into a WebAssembly module of the contract
//! `model/1` (see `sbmlsim_abi`): it imports the memory of the engine as `env.memory` and the
//! functions of mathlib it calls as `math.*`, and exports the thirteen functions of the contract.
//! The expressions are emitted as written, without fast math, reassociation or folding of
//! constants (`q / 49` is one `f64.div` by `49`), with the meaning libroadrunner gives every
//! operator (see `math.rs`).
//!
//! The functions:
//!
//! - `model_defaults` stores the default of every parameter and state.
//! - `model_init` evaluates the initial evaluations of the model in their order, into the
//!   memory, each unless its target is fixed.
//! - `model_rhs` and `model_selections` evaluate the assignment rules into locals in their
//!   order, then the rate of every state or every selection; they read the targets of the
//!   assignment rules from these locals, so they depend on nothing but `t`, `y` and the other
//!   parameters.
//! - `model_assign` evaluates the assignment rules in their order into the memory.
//! - `model_jac` traps: the description gives a colored difference quotient over the dense
//!   pattern, and the engine uses the difference quotient of CVODE (an analytic Jacobian comes
//!   later).
//! - `model_roots` evaluates the assignment rules into locals, then the root functions of the
//!   triggers ([`sbmlsim_ir::Event::root_functions`]) in the order of the events.
//! - the functions of an event index (`model_trigger`, `model_priority`, `model_delay`,
//!   `model_event_values`, `model_event_apply`) evaluate the assignment rules into locals, then
//!   branch on the index: one `if` per event which returns; an index which is no event gives
//!   false or 0. `model_event_apply` stores the values, the amount of a species of a
//!   concentration from the compartment before the assignments of the event (read into a local
//!   first), and rescales a species which stores its concentration by its compartment before
//!   over after the assignments.
//! - `model_sizes` stores the six sizes of the model.
//!
//! The crate does no file system or OS access, so it builds for `wasm32-unknown-unknown`.

mod math;
mod wasm;

use std::collections::BTreeMap;

use sbmlsim_abi::{
    self as abi, Entity, EventInfo, JacobianKind, MODEL_CONTRACT, ModelDescription, ModelSizes,
    Selection, SparsityPattern,
};
use sbmlsim_ir::{CompileError, EntityKind, Event, Expr, Model, SpeciesValue, Symbol};
use wasm_encoder::ValType::{F64, I32};

use wasm::{Ctx, ModuleBuilder};
pub use wasm::{MAX_FUNCTION_BYTES, MAX_LOCALS};

/// Emits `model.wasm` and its description.
///
/// Refuses a model of which a function exceeds the limits of WebAssembly engines
/// ([`MAX_FUNCTION_BYTES`], [`MAX_LOCALS`]).
pub fn emit(model: &Model) -> Result<(Vec<u8>, ModelDescription), CompileError> {
    let wasm = module(model)?;
    Ok((wasm, description(model)))
}

/// The module of a model.
fn module(model: &Model) -> Result<Vec<u8>, CompileError> {
    let mut m = ModuleBuilder::default();
    // a function which reads no symbol
    let none = Ctx { t: 0, y: 0, p: 0 };

    // model_defaults(p, y)
    let ctx = Ctx { t: 0, y: 1, p: 0 };
    let mut f = m.function(abi::DEFAULTS, &[I32, I32], &[], ctx);
    for (i, e) in model.parameters.iter().enumerate() {
        f.store_constant(Symbol::Param(i as u32), e.default);
    }
    for (i, e) in model.states.iter().enumerate() {
        f.store_constant(Symbol::State(i as u32), e.default);
    }
    f.finish()?;

    // model_init(t, p, y, fixed): fixed has a byte per parameter, then per state
    let ctx = Ctx { t: 0, y: 2, p: 1 };
    let mut f = m.function(abi::INIT, &[F64, I32, I32, I32], &[], ctx);
    let n_params = model.parameters.len() as u32;
    for a in &model.initial {
        let k = match a.target {
            Symbol::Param(i) => i,
            Symbol::State(i) => n_params + i,
            Symbol::Time => unreachable!("time is no target"),
        };
        f.store_unless_fixed(3, k, a.target, &a.expr);
    }
    f.finish()?;

    // model_rhs(t, y, p, ydot)
    let ctx = Ctx { t: 0, y: 1, p: 2 };
    let mut f = m.function(abi::RHS, &[F64, I32, I32, I32], &[], ctx);
    for a in &model.assignment_rules {
        f.assign_local(a.target, &a.expr);
    }
    for (i, rate) in model.rates.iter().enumerate() {
        f.store_at(3, i as u32, rate);
    }
    f.finish()?;

    // model_jac(t, y, p, values)
    let mut f = m.function(abi::JAC, &[F64, I32, I32, I32], &[], none);
    f.unreachable();
    f.finish()?;

    // model_roots(t, y, p, g): the root functions of the triggers, in the order of the events
    let roots: Vec<Expr> = model
        .events
        .iter()
        .flat_map(|e| e.root_functions())
        .collect();
    let ctx = Ctx { t: 0, y: 1, p: 2 };
    let mut f = m.function(abi::ROOTS, &[F64, I32, I32, I32], &[], ctx);
    if !roots.is_empty() {
        f.assign_locals(&model.assignment_rules);
    }
    for (k, root) in roots.iter().enumerate() {
        f.store_at(3, k as u32, root);
    }
    f.finish()?;

    // the functions of an event index `i` (the local 0): a branch per event, which returns;
    // an index which is no event gives false, 0, or nothing
    let indexed = Ctx { t: 1, y: 2, p: 3 };
    let events = &model.events;
    let with_rules = |f: &mut wasm::Body<'_>| {
        if !events.is_empty() {
            f.assign_locals(&model.assignment_rules);
        }
    };

    // model_trigger(i, t, y, p) -> i32
    let mut f = m.function(abi::TRIGGER, &[I32, F64, I32, I32], &[I32], indexed);
    with_rules(&mut f);
    for (k, e) in events.iter().enumerate() {
        f.if_index(0, k as u32);
        f.return_bool(&e.trigger);
        f.end();
    }
    f.i32_const(0);
    f.finish()?;

    // model_priority(i, t, y, p) -> f64 and model_delay(i, t, y, p) -> f64, 0 without
    for (name, part) in [
        (
            abi::PRIORITY,
            (|e: &Event| e.priority.as_ref()) as fn(&Event) -> Option<&Expr>,
        ),
        (abi::DELAY, |e: &Event| e.delay.as_ref()),
    ] {
        let mut f = m.function(name, &[I32, F64, I32, I32], &[F64], indexed);
        if events.iter().any(|e| part(e).is_some()) {
            f.assign_locals(&model.assignment_rules);
        }
        for (k, e) in events.iter().enumerate() {
            if let Some(x) = part(e) {
                f.if_index(0, k as u32);
                f.return_f64(x);
                f.end();
            }
        }
        f.return_f64(&Expr::Number(0.0));
        f.finish()?;
    }

    // model_event_values(i, t, y, p, out)
    let mut f = m.function(abi::EVENT_VALUES, &[I32, F64, I32, I32, I32], &[], indexed);
    with_rules(&mut f);
    for (k, e) in events.iter().enumerate() {
        f.if_index(0, k as u32);
        for (j, a) in e.assignments.iter().enumerate() {
            f.store_at(4, j as u32, &a.value);
        }
        f.return_();
        f.end();
    }
    f.finish()?;

    // model_event_apply(i, t, values, y, p)
    let apply = Ctx { t: 1, y: 3, p: 4 };
    let mut f = m.function(abi::EVENT_APPLY, &[I32, F64, I32, I32, I32], &[], apply);
    for (k, e) in events.iter().enumerate() {
        f.if_index(0, k as u32);
        event_apply(model, e, &mut f);
        f.return_();
        f.end();
    }
    f.finish()?;

    // model_selections(t, y, p, out)
    let ctx = Ctx { t: 0, y: 1, p: 2 };
    let mut f = m.function(abi::SELECTIONS, &[F64, I32, I32, I32], &[], ctx);
    for a in &model.assignment_rules {
        f.assign_local(a.target, &a.expr);
    }
    for (k, s) in model.selections.iter().enumerate() {
        f.store_at(3, k as u32, &s.expr);
    }
    f.finish()?;

    // model_assign(t, y, p)
    let mut f = m.function(abi::ASSIGN, &[F64, I32, I32], &[], ctx);
    for a in &model.assignment_rules {
        f.store(a.target, &a.expr);
    }
    f.finish()?;

    // model_sizes(out)
    let mut f = m.function(abi::SIZES, &[I32], &[], none);
    for (k, x) in sizes(model).to_words().into_iter().enumerate() {
        f.store_i32_at(0, k as u32, x);
    }
    f.finish()?;

    Ok(m.finish())
}

/// The sizes of the module of a model.
fn sizes(model: &Model) -> ModelSizes {
    ModelSizes {
        states: model.states.len() as u32,
        parameters: model.parameters.len() as u32,
        selections: model.selections.len() as u32,
        roots: model
            .events
            .iter()
            .map(|e| e.root_functions().len() as u32)
            .sum(),
        events: model.events.len() as u32,
        event_values: model
            .events
            .iter()
            .map(|e| e.assignments.len() as u32)
            .sum(),
    }
}

/// The body of `model_event_apply` for the event `e`; the values are in the local 2.
///
/// Every target gets its value; a species of a concentration gets the amount of the value
/// times its compartment before the assignments of the event, and a species which stores its
/// concentration keeps its amount when the event assigns its compartment (it is rescaled by the
/// compartment before over the compartment after).
fn event_apply(model: &Model, e: &Event, f: &mut wasm::Body<'_>) {
    const VALUES: u32 = 2;
    let assigned: Vec<Symbol> = e.assignments.iter().map(|a| a.target).collect();
    // a species which stores its concentration in a compartment the event assigns
    let stores_concentration = |s: Symbol| match &model.entity(s).kind {
        EntityKind::Species {
            compartment,
            value: SpeciesValue::Concentration,
            ..
        } => Some(*compartment),
        _ => None,
    };
    let rescaled: Vec<(Symbol, Symbol)> = model
        .states
        .iter()
        .enumerate()
        .map(|(i, _)| Symbol::State(i as u32))
        .chain((0..model.parameters.len()).map(|i| Symbol::Param(i as u32)))
        .filter_map(|s| {
            let c = stores_concentration(s)?;
            (assigned.contains(&c) && !assigned.contains(&s)).then_some((s, c))
        })
        .collect();
    // the compartments before the assignments
    let mut compartments: Vec<Symbol> = Vec::new();
    for c in e
        .assignments
        .iter()
        .filter_map(|a| a.compartment)
        .chain(rescaled.iter().map(|(_, c)| *c))
    {
        if !compartments.contains(&c) {
            compartments.push(c);
        }
    }
    if compartments.is_empty() {
        for (j, a) in e.assignments.iter().enumerate() {
            f.store_scaled(a.target, VALUES, Some(j as u32), None, None);
        }
        return;
    }
    f.assign_locals(&model.assignment_rules);
    let before: Vec<(Symbol, u32)> = compartments
        .iter()
        .map(|c| (*c, f.keep(&Expr::Symbol(*c))))
        .collect();
    let local = |list: &[(Symbol, u32)], c: Symbol| {
        list.iter()
            .find(|(s, _)| *s == c)
            .map(|(_, l)| *l)
            .expect("a compartment of the list")
    };
    f.forget_rules();
    // the targets which store what is assigned: the value, or the amount of a concentration
    let mut later = Vec::new();
    for (j, a) in e.assignments.iter().enumerate() {
        match (a.compartment, stores_concentration(a.target)) {
            (Some(c), None) => f.store_scaled(
                a.target,
                VALUES,
                Some(j as u32),
                Some(local(&before, c)),
                None,
            ),
            (Some(c), Some(_)) => later.push((a.target, Some(j as u32), c)),
            (None, _) => f.store_scaled(a.target, VALUES, Some(j as u32), None, None),
        }
    }
    later.extend(rescaled.iter().map(|(s, c)| (*s, None, *c)));
    if later.is_empty() {
        return;
    }
    // the species which store their concentration: the amount over the compartment after
    f.assign_locals(&model.assignment_rules);
    let after: Vec<(Symbol, u32)> = compartments
        .iter()
        .map(|c| (*c, f.keep(&Expr::Symbol(*c))))
        .collect();
    f.forget_rules();
    for (s, j, c) in later {
        f.store_scaled(
            s,
            VALUES,
            j,
            Some(local(&before, c)),
            Some(local(&after, c)),
        );
    }
}

/// The values of the parameters and states as loaded, `(p, y)`: the defaults, then the initial
/// evaluations at the time 0, evaluated natively with the operations of `model_init`, so they
/// are its bits.
pub fn initial_values(model: &Model) -> (Vec<f64>, Vec<f64>) {
    let mut p: Vec<f64> = model.parameters.iter().map(|e| e.default).collect();
    let mut y: Vec<f64> = model.states.iter().map(|e| e.default).collect();
    for a in &model.initial {
        let value = math::eval(&a.expr, &|s| match s {
            Symbol::State(i) => y[i as usize],
            Symbol::Param(i) => p[i as usize],
            Symbol::Time => 0.0,
        });
        match a.target {
            Symbol::State(i) => y[i as usize] = value,
            Symbol::Param(i) => p[i as usize] = value,
            Symbol::Time => unreachable!("time is no target"),
        }
    }
    (p, y)
}

/// The description of a model.
fn description(model: &Model) -> ModelDescription {
    let (p, y) = initial_values(model);
    let entity = |e: &sbmlsim_ir::Entity, initial: f64| Entity {
        id: e.id.clone(),
        kind: match &e.kind {
            EntityKind::Species {
                compartment,
                has_only_substance_units,
                ..
            } => abi::EntityKind::Species {
                compartment: model.entity(*compartment).id.clone(),
                has_only_substance_units: *has_only_substance_units,
            },
            EntityKind::Compartment => abi::EntityKind::Compartment,
            EntityKind::Parameter => abi::EntityKind::Parameter,
            EntityKind::SpeciesReference => abi::EntityKind::SpeciesReference,
        },
        initial,
        settable: e.settable,
    };
    let n = model.states.len() as u32;
    let pattern = SparsityPattern {
        n,
        col_ptr: (0..=n).map(|j| j * n).collect(),
        row_idx: (0..n).flat_map(|_| 0..n).collect(),
    };
    ModelDescription {
        contract: MODEL_CONTRACT.to_string(),
        id: model.id.clone().unwrap_or_default(),
        states: model
            .states
            .iter()
            .zip(y)
            .map(|(e, v)| entity(e, v))
            .collect(),
        parameters: model
            .parameters
            .iter()
            .zip(p)
            .map(|(e, v)| entity(e, v))
            .collect(),
        selections: model
            .selections
            .iter()
            .enumerate()
            .map(|(index, s)| Selection {
                id: s.id.clone(),
                index: index as u32,
            })
            .collect(),
        jacobian: JacobianKind::ColoredDifference {
            pattern,
            colors: (0..n).collect(),
        },
        events: model
            .events
            .iter()
            .map(|e| EventInfo {
                id: e.id.clone().unwrap_or_default(),
                has_delay: e.delay.is_some(),
                has_priority: e.priority.is_some(),
                persistent: e.persistent,
                initial_value: e.initial_value,
                use_values_from_trigger_time: e.use_values_from_trigger_time,
                n_assignments: e.assignments.len() as u32,
            })
            .collect(),
        n_roots: sizes(model).roots,
        units: model
            .selections
            .iter()
            .filter_map(|s| Some((s.id.clone(), s.unit.clone()?)))
            .collect::<BTreeMap<_, _>>(),
    }
}
