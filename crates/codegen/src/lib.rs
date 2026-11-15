//! The code generation of sbmlsim-core: `model.wasm` and its [`ModelDescription`] from the
//! model representation.
//!
//! [`emit`] translates an [`sbmlsim_ir::Model`] into a WebAssembly module of the contract
//! `model/1` (see `sbmlsim_abi`): it imports the memory of the engine as `env.memory` and the
//! functions of mathlib it calls as `math.*`, and exports the twelve functions of the contract.
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
//! - the event functions: `model_roots` writes nothing (no root functions), the functions of
//!   an event index trap (no index is valid). A model with events is refused until the events
//!   are generated.
//!
//! The crate does no file system or OS access, so it builds for `wasm32-unknown-unknown`.

mod math;
mod wasm;

use std::collections::BTreeMap;

use sbmlsim_abi::{
    self as abi, Entity, JacobianKind, MODEL_CONTRACT, ModelDescription, Selection, SparsityPattern,
};
use sbmlsim_ir::{CompileError, EntityKind, Model, Symbol};
use wasm_encoder::ValType::{F64, I32};

use wasm::{Ctx, ModuleBuilder};
pub use wasm::{MAX_FUNCTION_BYTES, MAX_LOCALS};

/// Emits `model.wasm` and its description.
///
/// Refuses a model with events (until they are generated) and a model of which a function
/// exceeds the limits of WebAssembly engines ([`MAX_FUNCTION_BYTES`], [`MAX_LOCALS`]).
pub fn emit(model: &Model) -> Result<(Vec<u8>, ModelDescription), CompileError> {
    if let Some((i, event)) = model.events.iter().enumerate().next() {
        return Err(CompileError::Unsupported {
            feature: "events".to_string(),
            element: event.id.clone().unwrap_or_else(|| format!("event #{i}")),
        });
    }
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

    // model_roots(t, y, p, g): no root functions
    m.function(abi::ROOTS, &[F64, I32, I32, I32], &[], none)
        .finish()?;

    // the functions of an event index: no index is valid
    for (name, params, results) in [
        (abi::TRIGGER, &[I32, F64, I32, I32][..], &[I32][..]),
        (abi::PRIORITY, &[I32, F64, I32, I32], &[F64]),
        (abi::DELAY, &[I32, F64, I32, I32], &[F64]),
        (abi::EVENT_VALUES, &[I32, F64, I32, I32, I32], &[]),
        (abi::EVENT_APPLY, &[I32, I32, I32, I32], &[]),
    ] {
        let mut f = m.function(name, params, results, none);
        f.unreachable();
        f.finish()?;
    }

    // model_selections(t, y, p, out)
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

    Ok(m.finish())
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
        events: Vec::new(),
        units: model
            .selections
            .iter()
            .filter_map(|s| Some((s.id.clone(), s.unit.clone()?)))
            .collect::<BTreeMap<_, _>>(),
    }
}
