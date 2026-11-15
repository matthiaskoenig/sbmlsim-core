//! The code generation of sbmlsim-core: `model.wasm` and its [`ModelDescription`] from the
//! model representation.
//!
//! [`emit`] translates an [`sbmlsim_ir::Model`] into a WebAssembly module of the contract
//! `model/1` (see `sbmlsim_abi`): it imports the memory of the engine as `env.memory` and the
//! functions of mathlib it calls as `math.*`, and exports the fourteen functions of the contract.
//! The expressions are emitted as written, without fast math, reassociation or folding of
//! constants (`q / 49` is one `f64.div` by `49`), with the meaning libroadrunner gives every
//! operator (see `math.rs`). [`emit_optimized`] emits the programs of the optimizer of
//! `sbmlsim_ir::optimize` instead (constant folding and the rewrites which are exact in binary64,
//! only the assignment rules a function reads, its common subexpressions once in locals), which
//! give the bits of the code as written; the functions below are those of [`emit_with`].
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
//! - `model_jac` evaluates the assignment rules into locals, then the derivatives of the rules
//!   ([`JacobianIr::Analytic`]) into locals in their order, then the entries of the Jacobian in
//!   the CSC order of its pattern. For a model without an analytic Jacobian (the description
//!   gives a colored difference quotient, which the engine computes) it traps.
//! - `model_roots` evaluates the assignment rules into locals, then the root functions of the
//!   triggers ([`sbmlsim_ir::Event::root_functions`]) in the order of the events.
//! - the functions of an event index (`model_trigger`, `model_priority`, `model_delay`,
//!   `model_event_values`, `model_event_apply`) evaluate the assignment rules into locals, then
//!   branch on the index: one `if` per event which returns; an index which is no event gives
//!   false or 0. `model_event_apply` stores the values, the amount of a species of a
//!   concentration from the compartment before the assignments of the event (read into a local
//!   first), and rescales a species which stores its concentration by its compartment before
//!   over after the assignments.
//! - `model_sizes` and `model_event_sizes` store the sizes of the model.
//!
//! The crate does no file system or OS access, so it builds for `wasm32-unknown-unknown`.

mod math;
mod wasm;

use std::collections::BTreeMap;

use sbmlsim_abi::{
    self as abi, Entity, EventInfo, JacobianChoice, JacobianKind, MODEL_CONTRACT, ModelDescription,
    ModelSizes, Selection,
};
use sbmlsim_ir::derive::{self, JacobianIr};
use sbmlsim_ir::optimize::{self, Plan, Program, Statement};
use sbmlsim_ir::{CompileError, EntityKind, Event, Expr, Model, SpeciesValue, Symbol};
use wasm_encoder::ValType::{F64, I32};

use wasm::{Ctx, Limits, ModuleBuilder};
pub use wasm::{MAX_FUNCTION_BYTES, MAX_LOCALS};

/// The reason of a difference quotient which was asked for.
pub const DIFFERENCE_REQUESTED: &str = "the difference quotient was requested";

/// Emits `model.wasm` and its description, with the analytic Jacobian where the model has one
/// ([`sbmlsim_ir::derive::jacobian`]), else the colored difference quotient.
///
/// Refuses a model of which a function exceeds the limits of WebAssembly engines
/// ([`MAX_FUNCTION_BYTES`], [`MAX_LOCALS`]).
pub fn emit(model: &Model) -> Result<(Vec<u8>, ModelDescription), CompileError> {
    emit_with(model, JacobianChoice::Auto)
}

/// Emits `model.wasm` and its description with the Jacobian of `choice`: `Auto` as [`emit`],
/// `DifferenceQuotient` the colored difference quotient (with the reason
/// [`DIFFERENCE_REQUESTED`]), `Analytic` the analytic Jacobian, refused (`Unsupported`, with the
/// reason) for a model which has none.
///
/// A `model_jac` of the analytic Jacobian above the limits of WebAssembly engines is the
/// colored difference quotient under `Auto` (with the reason naming its size) and refused
/// under `Analytic`.
pub fn emit_with(
    model: &Model,
    choice: JacobianChoice,
) -> Result<(Vec<u8>, ModelDescription), CompileError> {
    emit_limited(model, choice, Limits::default(), false)
}

/// [`emit_with`] with the optimizer ([`sbmlsim_ir::optimize`]): the functions are the programs
/// of [`optimize::plan`] of the model and of its Jacobian as [`emit_with`] derives it, so the
/// module is smaller and faster and gives the bits of the module of [`emit_with`].
///
/// The kind of the Jacobian (and its refusal under `Analytic`) is decided from `model_jac` as
/// written, as in [`emit_with`], so the description is the one of [`emit_with`] also near the
/// limits, where an optimized `model_jac` may be within them and the one as written not. A
/// module whose common subexpressions put a function above the limits (they add locals) is
/// emitted without them, and as written when it is above them still.
pub fn emit_optimized(
    model: &Model,
    choice: JacobianChoice,
) -> Result<(Vec<u8>, ModelDescription), CompileError> {
    emit_limited(model, choice, Limits::default(), true)
}

/// [`emit_with`] or [`emit_optimized`] with the limits of the functions given (lower ones in
/// tests).
fn emit_limited(
    model: &Model,
    choice: JacobianChoice,
    limits: Limits,
    optimized: bool,
) -> Result<(Vec<u8>, ModelDescription), CompileError> {
    // the kind of the Jacobian and its reason are those of the model as written, with and
    // without the optimizer
    let jacobian = choose_jacobian(model, choice, limits)?;
    let wasm = if optimized {
        // the common subexpressions add locals: without them when a function is above the
        // limits with them, and as written when it is above them still (the folding of a
        // literal boolean may add bytes), whose model_jac is known to be within them
        let plan = |cse| optimize::plan(model, &jacobian, cse);
        let within = |result: Result<Vec<u8>, CompileError>, next: &dyn Fn() -> _| match result {
            Err(error) if wasm::is_above_the_limits(&error) => next(),
            other => other,
        };
        within(module(model, &jacobian, limits, Some(&plan(true))), &|| {
            within(
                module(model, &jacobian, limits, Some(&plan(false))),
                &|| module(model, &jacobian, limits, None),
            )
        })?
    } else {
        module(model, &jacobian, limits, None)?
    };
    Ok((wasm, description(model, jacobian)))
}

/// Adds `model_jac(t, y, p, values)` as written to the module: the rules, their derivatives, the
/// entries; it traps for the colored difference quotient.
fn written_jac(
    m: &mut ModuleBuilder,
    model: &Model,
    jacobian: &JacobianIr,
) -> Result<(), CompileError> {
    let ctx = Ctx { t: 0, y: 1, p: 2 };
    let mut f = m.function(abi::JAC, &[F64, I32, I32, I32], &[], ctx);
    match jacobian {
        JacobianIr::Analytic {
            derivatives,
            entries,
            ..
        } => {
            if !entries.is_empty() {
                f.assign_locals(&model.assignment_rules);
            }
            for (k, d) in derivatives.iter().enumerate() {
                f.assign_local(Symbol::Derivative(k as u32), d);
            }
            for (k, e) in entries.iter().enumerate() {
                f.store_at(3, k as u32, e);
            }
        }
        JacobianIr::ColoredDifference { .. } => f.unreachable(),
    }
    f.finish()
}

/// The Jacobian of a model for `choice`, decided from the model as written: the analytic one
/// where the model has one, unless the difference quotient is asked for or `model_jac` as
/// written is above the limits (then the colored difference quotient under `Auto`, with the
/// reason naming the size of that function, and refused under `Analytic`).
///
/// The size of a function does not depend on the others: an import of mathlib has an index
/// below 128 in any order of the imports, one byte in the code.
fn choose_jacobian(
    model: &Model,
    choice: JacobianChoice,
    limits: Limits,
) -> Result<JacobianIr, CompileError> {
    let jacobian = match (derive::jacobian(model), choice) {
        (jacobian, JacobianChoice::DifferenceQuotient) => {
            return Ok(jacobian.into_difference(DIFFERENCE_REQUESTED));
        }
        (JacobianIr::ColoredDifference { reason, .. }, JacobianChoice::Analytic) => {
            return Err(CompileError::Unsupported {
                feature: format!("an analytic Jacobian: {reason}"),
                element: model.id.clone().unwrap_or_default(),
            });
        }
        (jacobian, _) => jacobian,
    };
    if matches!(jacobian, JacobianIr::ColoredDifference { .. }) {
        return Ok(jacobian);
    }
    let mut m = ModuleBuilder::default();
    m.limits = limits;
    match written_jac(&mut m, model, &jacobian) {
        Ok(()) => Ok(jacobian),
        // the analytic Jacobian is too large for one function: the difference quotient, which
        // needs no model_jac, unless the analytic one was asked for
        Err(CompileError::Unsupported { feature, .. }) if choice == JacobianChoice::Auto => {
            Ok(jacobian.into_difference(format!("model_jac would be {feature}")))
        }
        Err(error) => Err(error),
    }
}

/// Emits the statements of a program: a let into a local, the output `k` with `output`.
fn emit_program(
    f: &mut wasm::Body<'_>,
    program: &Program,
    mut output: impl FnMut(&mut wasm::Body<'_>, u32, &Expr),
) {
    let mut k = 0;
    for statement in &program.statements {
        match statement {
            Statement::Let(symbol, e) => f.assign_local(*symbol, e),
            Statement::Output(e) => {
                output(f, k, e);
                k += 1;
            }
        }
    }
}

/// The module of a model: the code as written, or the programs of `plan` (the optimizer).
fn module(
    model: &Model,
    jacobian: &JacobianIr,
    limits: Limits,
    plan: Option<&Plan>,
) -> Result<Vec<u8>, CompileError> {
    if let Some(plan) = plan {
        return optimized_module(model, jacobian, limits, plan);
    }
    let mut m = ModuleBuilder::default();
    m.limits = limits;
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
            Symbol::Time | Symbol::Derivative(_) | Symbol::Common(_) => {
                unreachable!("{:?} is no target", a.target)
            }
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

    written_jac(&mut m, model, jacobian)?;

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

    // model_sizes(out) and model_event_sizes(out)
    let sizes = sizes(model, jacobian);
    let mut f = m.function(abi::SIZES, &[I32], &[], none);
    for (k, x) in sizes.words().into_iter().enumerate() {
        f.store_i32_at(0, k as u32, x);
    }
    f.finish()?;
    let mut f = m.function(abi::EVENT_SIZES, &[I32], &[], none);
    for (k, x) in sizes.assignments.iter().enumerate() {
        f.store_i32_at(0, k as u32, *x);
    }
    f.finish()?;

    Ok(m.finish())
}

/// The module of a model from the programs of the optimizer, with the outputs of the code as
/// written (see [`module`]); `model_defaults`, `model_event_apply` and the sizes are the code
/// as written.
fn optimized_module(
    model: &Model,
    jacobian: &JacobianIr,
    limits: Limits,
    plan: &Plan,
) -> Result<Vec<u8>, CompileError> {
    let mut m = ModuleBuilder::default();
    m.limits = limits;
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

    // model_init(t, p, y, fixed): each initial evaluation in the block of its target
    let ctx = Ctx { t: 0, y: 2, p: 1 };
    let mut f = m.function(abi::INIT, &[F64, I32, I32, I32], &[], ctx);
    let n_params = model.parameters.len() as u32;
    for (a, program) in model.initial.iter().zip(&plan.init) {
        let k = match a.target {
            Symbol::Param(i) => i,
            Symbol::State(i) => n_params + i,
            Symbol::Time | Symbol::Derivative(_) | Symbol::Common(_) => {
                unreachable!("{:?} is no target", a.target)
            }
        };
        f.if_not_fixed(3, k);
        emit_program(&mut f, program, |f, _, e| f.store(a.target, e));
        f.end();
    }
    f.finish()?;

    // model_rhs(t, y, p, ydot), model_jac(t, y, p, values), model_roots(t, y, p, g)
    let ctx = Ctx { t: 0, y: 1, p: 2 };
    let mut f = m.function(abi::RHS, &[F64, I32, I32, I32], &[], ctx);
    emit_program(&mut f, &plan.rhs, |f, k, e| f.store_at(3, k, e));
    f.finish()?;
    let mut f = m.function(abi::JAC, &[F64, I32, I32, I32], &[], ctx);
    match (jacobian, &plan.jac) {
        (JacobianIr::Analytic { .. }, Some(program)) => {
            emit_program(&mut f, program, |f, k, e| f.store_at(3, k, e))
        }
        (JacobianIr::ColoredDifference { .. }, None) => f.unreachable(),
        _ => unreachable!("the plan is of another Jacobian"),
    }
    f.finish()?;
    let mut f = m.function(abi::ROOTS, &[F64, I32, I32, I32], &[], ctx);
    emit_program(&mut f, &plan.roots, |f, k, e| f.store_at(3, k, e));
    f.finish()?;

    // the functions of an event index `i` (the local 0)
    let indexed = Ctx { t: 1, y: 2, p: 3 };

    // model_trigger(i, t, y, p) -> i32
    let mut f = m.function(abi::TRIGGER, &[I32, F64, I32, I32], &[I32], indexed);
    emit_program(&mut f, &plan.trigger.prefix, |_, _, _| {
        unreachable!("a let")
    });
    for (k, branch) in plan.trigger.branches.iter().enumerate() {
        let branch = branch.as_ref().expect("every event has a trigger");
        f.if_index(0, k as u32);
        emit_program(&mut f, branch, |f, _, e| f.return_bool(e));
        f.end();
    }
    f.i32_const(0);
    f.finish()?;

    // model_priority(i, t, y, p) -> f64 and model_delay(i, t, y, p) -> f64, 0 without
    for (name, part) in [(abi::PRIORITY, &plan.priority), (abi::DELAY, &plan.delay)] {
        let mut f = m.function(name, &[I32, F64, I32, I32], &[F64], indexed);
        emit_program(&mut f, &part.prefix, |_, _, _| unreachable!("a let"));
        for (k, branch) in part.branches.iter().enumerate() {
            if let Some(branch) = branch {
                f.if_index(0, k as u32);
                emit_program(&mut f, branch, |f, _, e| f.return_f64(e));
                f.end();
            }
        }
        f.return_f64(&Expr::Number(0.0));
        f.finish()?;
    }

    // model_event_values(i, t, y, p, out)
    let mut f = m.function(abi::EVENT_VALUES, &[I32, F64, I32, I32, I32], &[], indexed);
    emit_program(&mut f, &plan.event_values.prefix, |_, _, _| {
        unreachable!("a let")
    });
    for (k, branch) in plan.event_values.branches.iter().enumerate() {
        let branch = branch.as_ref().expect("every event has its values");
        f.if_index(0, k as u32);
        emit_program(&mut f, branch, |f, j, e| f.store_at(4, j, e));
        f.return_();
        f.end();
    }
    f.finish()?;

    // model_event_apply(i, t, values, y, p), as written
    let apply = Ctx { t: 1, y: 3, p: 4 };
    let mut f = m.function(abi::EVENT_APPLY, &[I32, F64, I32, I32, I32], &[], apply);
    for (k, e) in model.events.iter().enumerate() {
        f.if_index(0, k as u32);
        event_apply(model, e, &mut f);
        f.return_();
        f.end();
    }
    f.finish()?;

    // model_selections(t, y, p, out)
    let ctx = Ctx { t: 0, y: 1, p: 2 };
    let mut f = m.function(abi::SELECTIONS, &[F64, I32, I32, I32], &[], ctx);
    emit_program(&mut f, &plan.selections, |f, k, e| f.store_at(3, k, e));
    f.finish()?;

    // model_assign(t, y, p): every rule stored before the next one reads the memory
    let mut f = m.function(abi::ASSIGN, &[F64, I32, I32], &[], ctx);
    let rules = &model.assignment_rules;
    emit_program(&mut f, &plan.assign, |f, k, e| {
        f.store(rules[k as usize].target, e)
    });
    f.finish()?;

    // model_sizes(out) and model_event_sizes(out)
    let sizes = sizes(model, jacobian);
    let mut f = m.function(abi::SIZES, &[I32], &[], none);
    for (k, x) in sizes.words().into_iter().enumerate() {
        f.store_i32_at(0, k as u32, x);
    }
    f.finish()?;
    let mut f = m.function(abi::EVENT_SIZES, &[I32], &[], none);
    for (k, x) in sizes.assignments.iter().enumerate() {
        f.store_i32_at(0, k as u32, *x);
    }
    f.finish()?;

    Ok(m.finish())
}

/// The sizes of the module of a model; the entries of the Jacobian are those of its pattern.
fn sizes(model: &Model, jacobian: &JacobianIr) -> ModelSizes {
    ModelSizes {
        states: model.states.len() as u32,
        parameters: model.parameters.len() as u32,
        selections: model.selections.len() as u32,
        jacobian_entries: jacobian.pattern().nnz() as u32,
        roots: model
            .events
            .iter()
            .map(|e| e.root_functions().len() as u32)
            .sum(),
        assignments: model
            .events
            .iter()
            .map(|e| e.assignments.len() as u32)
            .collect(),
    }
}

/// The body of `model_event_apply` for the event `e`; the values are in the local 2.
///
/// As libroadrunner 2.11 (probed, Task 13 report): the assignments are stored in document
/// order, those of compartments last; a species of a concentration gets the amount of its value
/// times its compartment as it is at its turn (a compartment the event assigns is still the one
/// before the event, one of an assignment rule follows the assignments before it); every other
/// species keeps its amount. For a species which stores its concentration (the target of a
/// rate rule) that is its concentration times its compartment before over after the event, when
/// the compartment changed (also through an assignment rule).
fn event_apply(model: &Model, e: &Event, f: &mut wasm::Body<'_>) {
    const VALUES: u32 = 2;
    let is_compartment = |s: Symbol| matches!(model.entity(s).kind, EntityKind::Compartment);
    let is_rule_target = |s: Symbol| model.assignment_rules.iter().any(|a| a.target == s);
    // the states which store a concentration, with their compartments
    let stored: Vec<(Symbol, Symbol)> = (0..model.states.len())
        .map(|i| Symbol::State(i as u32))
        .filter_map(|s| match &model.entity(s).kind {
            EntityKind::Species {
                compartment,
                value: SpeciesValue::Concentration,
                ..
            } => Some((s, *compartment)),
            _ => None,
        })
        .collect();
    let mut compartments: Vec<Symbol> = Vec::new();
    for (_, c) in &stored {
        if !compartments.contains(c) {
            compartments.push(*c);
        }
    }
    let lookup = |list: &[(Symbol, u32)], c: Symbol| {
        list.iter()
            .find(|(s, _)| *s == c)
            .map(|(_, l)| *l)
            .expect("a compartment of the list")
    };
    // the compartments before the assignments
    let mut rules_current = false;
    let before: Vec<(Symbol, u32)> = if compartments.is_empty() {
        Vec::new()
    } else {
        f.assign_locals(&model.assignment_rules);
        rules_current = true;
        compartments
            .iter()
            .map(|c| (*c, f.keep(&Expr::Symbol(*c))))
            .collect()
    };
    // the assignments in document order, the compartments last; the amounts of the species
    // which store their concentration are kept in locals until the compartments are known
    let mut amounts: Vec<(Symbol, u32, u32)> = Vec::new();
    let order = e
        .assignments
        .iter()
        .enumerate()
        .filter(|(_, a)| !is_compartment(a.target))
        .chain(
            e.assignments
                .iter()
                .enumerate()
                .filter(|(_, a)| is_compartment(a.target)),
        );
    for (j, a) in order {
        let j = j as u32;
        let Some(c) = a.compartment else {
            f.store_scaled(a.target, VALUES, Some(j), None, None);
            rules_current = false;
            continue;
        };
        // the compartment at the turn of this assignment
        if is_rule_target(c) && !rules_current {
            f.assign_locals(&model.assignment_rules);
            rules_current = true;
        }
        if stored.iter().any(|(s, _)| *s == a.target) {
            let now = f.keep(&Expr::Symbol(c));
            amounts.push((a.target, j, now));
        } else {
            let now = f.keep(&Expr::Symbol(c));
            f.store_scaled(a.target, VALUES, Some(j), Some(now), None);
            rules_current = false;
        }
    }
    if stored.is_empty() {
        f.forget_rules();
        return;
    }
    // the compartments after the assignments
    f.assign_locals(&model.assignment_rules);
    let after: Vec<(Symbol, u32)> = compartments
        .iter()
        .map(|c| (*c, f.keep(&Expr::Symbol(*c))))
        .collect();
    f.forget_rules();
    for (s, c) in &stored {
        let to = lookup(&after, *c);
        match amounts.iter().find(|(t, _, _)| t == s) {
            // value * now / after, the value itself when the compartment did not change
            Some((_, j, now)) => f.store_rescaled(*s, VALUES, Some(*j), *now, to),
            // the amount is kept: y * before / after, when the compartment changed
            None => f.store_rescaled(*s, VALUES, None, lookup(&before, *c), to),
        }
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
            Symbol::Derivative(_) => unreachable!("an initial value reads no derivative"),
            Symbol::Common(_) => {
                unreachable!("an initial value as written reads no common subexpression")
            }
        });
        match a.target {
            Symbol::State(i) => y[i as usize] = value,
            Symbol::Param(i) => p[i as usize] = value,
            Symbol::Time | Symbol::Derivative(_) | Symbol::Common(_) => {
                unreachable!("{:?} is no target", a.target)
            }
        }
    }
    (p, y)
}

/// The description of a model.
fn description(model: &Model, jacobian: JacobianIr) -> ModelDescription {
    let n_roots = sizes(model, &jacobian).roots;
    let (p, y) = initial_values(model);
    let entity = |e: &sbmlsim_ir::Entity, initial: f64| Entity {
        id: e.id.clone(),
        kind: match &e.kind {
            EntityKind::Species {
                compartment,
                has_only_substance_units,
                value,
                ..
            } => abi::EntityKind::Species {
                compartment: model.entity(*compartment).id.clone(),
                has_only_substance_units: *has_only_substance_units,
                stores_concentration: *value == SpeciesValue::Concentration,
            },
            EntityKind::Compartment => abi::EntityKind::Compartment,
            EntityKind::Parameter => abi::EntityKind::Parameter,
            EntityKind::SpeciesReference => abi::EntityKind::SpeciesReference,
        },
        initial,
        settable: e.settable,
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
        jacobian: match jacobian {
            JacobianIr::Analytic { pattern, .. } => JacobianKind::Analytic(pattern),
            JacobianIr::ColoredDifference {
                pattern,
                colors,
                reason,
            } => JacobianKind::ColoredDifference {
                pattern,
                colors,
                reason,
            },
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
        n_roots,
        units: model
            .selections
            .iter()
            .filter_map(|s| Some((s.id.clone(), s.unit.clone()?)))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

    /// `x' = -b` with the rule `b = x * x + y` and `y' = 1`: `model_jac` has one more local than
    /// any other function (the derivative `x + x` of the rule).
    fn chain() -> Model {
        let xml = format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m">
<listOfParameters><parameter id="x" value="1" constant="false"/><parameter id="y" value="1" constant="false"/><parameter id="b" constant="false"/></listOfParameters>
<listOfRules>
<assignmentRule variable="b"><math {MATH}><apply><plus/><apply><times/><ci>x</ci><ci>x</ci></apply><ci>y</ci></apply></math></assignmentRule>
<rateRule variable="x"><math {MATH}><apply><minus/><ci>b</ci></apply></math></rateRule>
<rateRule variable="y"><math {MATH}><cn>1</cn></math></rateRule>
</listOfRules></model></sbml>"#
        );
        sbmlsim_ir::build(&sbmlsim_sbml::read(&xml).unwrap()).unwrap()
    }

    /// `x' = exp(x) + exp(x)`, `y' = exp(y) * exp(y)`: the common subexpressions add a local
    /// per rate to `model_rhs`.
    fn shared() -> Model {
        let xml = format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m">
<listOfParameters><parameter id="x" value="1" constant="false"/><parameter id="y" value="1" constant="false"/></listOfParameters>
<listOfRules>
<rateRule variable="x"><math {MATH}><apply><plus/><apply><exp/><ci>x</ci></apply><apply><exp/><ci>x</ci></apply></apply></math></rateRule>
<rateRule variable="y"><math {MATH}><apply><times/><apply><exp/><ci>y</ci></apply><apply><exp/><ci>y</ci></apply></apply></math></rateRule>
</listOfRules></model></sbml>"#
        );
        sbmlsim_ir::build(&sbmlsim_sbml::read(&xml).unwrap()).unwrap()
    }

    #[test]
    fn an_optimized_module_above_the_limits_has_no_common_subexpressions() {
        let m = shared();
        // the calls of mathlib (of math_exp) in model_rhs, the third function of the module
        let exps = |wasm: &[u8]| {
            let bodies: Vec<wasmparser::FunctionBody> = wasmparser::Parser::new(0)
                .parse_all(wasm)
                .filter_map(|p| match p.unwrap() {
                    wasmparser::Payload::CodeSectionEntry(body) => Some(body),
                    _ => None,
                })
                .collect();
            bodies[2]
                .get_operators_reader()
                .unwrap()
                .into_iter()
                .filter(|op| matches!(op.as_ref().unwrap(), wasmparser::Operator::Call { .. }))
                .count()
        };
        // with the common subexpressions model_rhs has 4 parameters and 2 locals
        let (wasm, d) = emit_limited(&m, JacobianChoice::Auto, Limits::default(), true).unwrap();
        assert_eq!(exps(&wasm), 2);
        assert!(matches!(d.jacobian, JacobianKind::Analytic(_)));
        // above 5 locals: without them, every exp of the code as written
        let limits = Limits {
            bytes: MAX_FUNCTION_BYTES,
            locals: 5,
        };
        let (wasm, d) = emit_limited(&m, JacobianChoice::Auto, limits, true).unwrap();
        assert_eq!(exps(&wasm), 4);
        assert!(matches!(d.jacobian, JacobianKind::Analytic(_)));
        let (written, _) = emit_limited(&m, JacobianChoice::Auto, limits, false).unwrap();
        assert_eq!(exps(&written), 4);
        // a module of 4 locals (the parameters) at most is an error with and without them
        let tight = Limits {
            bytes: MAX_FUNCTION_BYTES,
            locals: 3,
        };
        assert!(emit_limited(&m, JacobianChoice::Auto, tight, true).is_err());
    }

    /// The kind of the Jacobian and the description do not depend on the optimizer, at, below
    /// and above the limits of bytes and of locals, also where only `model_jac` as written is
    /// above them.
    #[test]
    fn the_jacobian_is_chosen_from_the_model_as_written() {
        let m = chain();
        let mut limits: Vec<Limits> = (1..=10)
            .map(|locals| Limits {
                bytes: MAX_FUNCTION_BYTES,
                locals,
            })
            .collect();
        limits.extend((1..=120).map(|bytes| Limits {
            bytes,
            locals: MAX_LOCALS,
        }));
        let mut compared = 0;
        for limits in limits {
            for choice in [
                JacobianChoice::Auto,
                JacobianChoice::Analytic,
                JacobianChoice::DifferenceQuotient,
            ] {
                let written = emit_limited(&m, choice, limits, false);
                let optimized = emit_limited(&m, choice, limits, true);
                match (written, optimized) {
                    (Ok((_, a)), Ok((_, b))) => {
                        assert_eq!(a, b, "{limits:?} {choice:?}");
                        compared += 1;
                    }
                    (Err(a), Err(b)) => assert_eq!(a, b, "{limits:?} {choice:?}"),
                    // a function as written above the limits which the optimizer makes smaller
                    (Err(a), Ok((_, b))) => {
                        assert!(
                            !matches!(&a, CompileError::Unsupported { element, .. } if element == abi::JAC),
                            "{limits:?} {choice:?}: {a:?}"
                        );
                        assert_eq!(
                            b.jacobian.pattern(),
                            derive::jacobian(&m).pattern(),
                            "{limits:?} {choice:?}"
                        );
                    }
                    (Ok(_), Err(b)) => panic!("{limits:?} {choice:?}: only optimized {b:?}"),
                }
            }
        }
        assert!(compared > 200, "{compared}");

        // 5 locals: model_jac as written has 6 (the rule b and its derivative), the optimized
        // one 5 (no rule b, which the derivative x + x does not read), every other function 5
        let limits = Limits {
            bytes: MAX_FUNCTION_BYTES,
            locals: 5,
        };
        let analytic = derive::jacobian(&m);
        for cse in [true, false] {
            let plan = optimize::plan(&m, &analytic, cse);
            assert!(module(&m, &analytic, limits, Some(&plan)).is_ok());
        }
        let reason = "model_jac would be a function above the limits of WebAssembly engines (74 \
                      bytes, 6 locals; at most 7654321 bytes and 5 locals)";
        for optimized in [false, true] {
            let (_, d) = emit_limited(&m, JacobianChoice::Auto, limits, optimized).unwrap();
            let JacobianKind::ColoredDifference { reason: r, .. } = &d.jacobian else {
                panic!("{:?}", d.jacobian)
            };
            assert_eq!(r, reason);
            let error = emit_limited(&m, JacobianChoice::Analytic, limits, optimized).unwrap_err();
            assert!(
                matches!(&error, CompileError::Unsupported { element, .. } if element == abi::JAC),
                "{error:?}"
            );
        }
        // within the limits of model_jac as written both are analytic
        let limits = Limits {
            bytes: 74,
            locals: 6,
        };
        for optimized in [false, true] {
            let (_, d) = emit_limited(&m, JacobianChoice::Analytic, limits, optimized).unwrap();
            assert!(matches!(d.jacobian, JacobianKind::Analytic(_)));
        }
        let limits = Limits {
            bytes: 73,
            locals: 6,
        };
        for optimized in [false, true] {
            let (_, d) = emit_limited(&m, JacobianChoice::Auto, limits, optimized).unwrap();
            assert!(matches!(d.jacobian, JacobianKind::ColoredDifference { .. }));
        }
    }

    #[test]
    fn a_model_jac_above_the_limits_is_the_difference_quotient_under_auto() {
        let m = chain();
        let JacobianIr::Analytic { derivatives, .. } = derive::jacobian(&m) else {
            panic!()
        };
        assert_eq!(derivatives.len(), 1);
        // 4 parameters of model_jac, the rule and its derivative: 6 locals; every other
        // function has at most 5 (model_rhs and model_selections: 4 parameters and the rule)
        let limits = Limits {
            bytes: MAX_FUNCTION_BYTES,
            locals: 5,
        };
        let (_, d) = emit_limited(&m, JacobianChoice::Auto, limits, false).unwrap();
        let JacobianKind::ColoredDifference {
            pattern, reason, ..
        } = &d.jacobian
        else {
            panic!("{:?}", d.jacobian)
        };
        assert_eq!(pattern.row_idx, [0, 0]);
        assert_eq!(
            reason,
            "model_jac would be a function above the limits of WebAssembly engines (\
             74 bytes, 6 locals; at most 7654321 bytes and 5 locals)"
        );
        // the analytic Jacobian asked for is refused with the error of the limits
        let error = emit_limited(&m, JacobianChoice::Analytic, limits, false).unwrap_err();
        assert!(
            matches!(&error, CompileError::Unsupported { element, .. } if element == abi::JAC),
            "{error:?}"
        );
        // within the limits it is analytic
        let (_, d) = emit_limited(&m, JacobianChoice::Auto, Limits::default(), false).unwrap();
        assert!(matches!(d.jacobian, JacobianKind::Analytic(_)));
        // a function other than model_jac above the limits stays an error
        let tight = Limits {
            bytes: MAX_FUNCTION_BYTES,
            locals: 4,
        };
        let error = emit_limited(&m, JacobianChoice::Auto, tight, false).unwrap_err();
        assert!(
            matches!(&error, CompileError::Unsupported { element, .. } if element != abi::JAC),
            "{error:?}"
        );
    }
}
