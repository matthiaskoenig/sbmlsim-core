//! The requests of `simulation/1` of the plans of the points, sbmlsim's
//! `executor_core._fold_of` (`check_supported`, `_preinit_values`, `_preinit_value`,
//! `_check_change`, `_steady_state`, `_steady_state_after`) and the selections of a run
//! (`Simulator._compile`, `observables.identity_graph`).
//!
//! A value of a request is a number, so every formula of a plan is folded. A formula before
//! the initialization is folded here without the model where its symbols are known (the
//! numbers the same pre-initialization sets, or the values of the model as loaded). A formula
//! at a change is folded by [`crate::fold::Fold`]: one which reads nothing but the time is
//! evaluated at the time of the change, and one which reads the state of the model by
//! simulating the segment which ends at the change, a round of its own.

use sbmlsim_abi::{
    IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SteadyState,
    SteadyStateAfter,
};

use crate::error::ScanError;
use crate::expand::RESERVED;
use crate::formula::CompiledFormula;
use crate::model::{ModelInfo, TargetKind};
use crate::plan::{Assigned, Assignment, OutputMode, Plan, PlanEvent, preinit_targets};

/// The columns of the output of a plan, `time` first: sbmlsim's `output_columns`.
pub fn output_columns(selections: &[String]) -> Vec<String> {
    if selections.first().is_some_and(|s| s == "time") {
        selections.to_vec()
    } else {
        std::iter::once("time".to_string())
            .chain(selections.iter().cloned())
            .collect()
    }
}

/// The selections of the requests of a run without observables, `time` first: the selections
/// of the model which `keep` keeps (all of them without it), sbmlsim's `identity_graph`.
///
/// # Errors
///
/// A `keep` which names no selection or keeps nothing, and a selection which is a name of the
/// result.
pub fn run_selections(
    selections: &[String],
    keep: Option<&[String]>,
) -> Result<Vec<String>, ScanError> {
    let mut outputs: Vec<&String> = Vec::new();
    for s in selections.iter().filter(|s| *s != "time") {
        if !outputs.contains(&s) {
            outputs.push(s);
        }
    }
    let kept: Vec<String> = match keep {
        None => outputs.iter().map(|s| (*s).clone()).collect(),
        Some(keep) => {
            let mut kept: Vec<String> = Vec::new();
            for key in keep {
                if !outputs.contains(&key) {
                    let all: Vec<&str> = outputs.iter().map(|s| s.as_str()).collect();
                    return Err(ScanError::request(format!(
                        "'keep' names '{key}', which is neither an observable of the run nor a \
                         selection of the model: {}.",
                        crate::text::list(&all)
                    )));
                }
                if !kept.contains(key) {
                    kept.push(key.clone());
                }
            }
            if kept.is_empty() {
                return Err(ScanError::request(
                    "'keep' keeps nothing; name at least one observable.",
                ));
            }
            kept
        }
    };
    let mut reserved: Vec<&str> = kept
        .iter()
        .map(String::as_str)
        .filter(|s| RESERVED.contains(s) && *s != "time")
        .collect();
    reserved.sort_unstable();
    if !reserved.is_empty() {
        return Err(ScanError::request(format!(
            "The selections {} are names of the result ({}), select other entities, see \
             `SBMLModel.set_selections`.",
            crate::text::list(&reserved),
            crate::expand::reserved_text()
        )));
    }
    Ok(output_columns(&kept))
}

/// The request of the plan of a point without its changes, which its fold adds
/// ([`crate::fold::Fold`]), and the check of the plan, sbmlsim's `_fold_of` and
/// `check_supported`.
///
/// # Errors
///
/// A formula the planner cannot fold (see the module), which fails every point alike.
pub(crate) fn base_request(
    plan: &Plan,
    model: &ModelInfo,
    selections: &[String],
    settings: &IntegratorSettings,
) -> Result<SimulationRequest, ScanError> {
    for event in &plan.events {
        check_change(event, model)?;
    }
    let plain = plain_values(&preinit_targets(plan));
    let output = match plan.output {
        OutputMode::Integrator => Output::IntegratorSteps,
        OutputMode::Times => Output::Times(plan.times.clone()),
    };
    let steady_state = match &plan.steady_state {
        None => None,
        Some(steady) => Some(SteadyState {
            preinit: preinit_values(&steady.preinit, &plain, model)?,
            absolute_tolerance: steady.absolute_tolerance,
            relative_tolerance: steady.relative_tolerance,
            max_time: steady.max_time,
        }),
    };
    let steady_state_after = plan
        .steady_state_output
        .as_ref()
        .map(|steady| SteadyStateAfter {
            from: if model.time_dependent() {
                plan.end
            } else {
                0.0
            },
            absolute_tolerance: steady.absolute_tolerance,
            relative_tolerance: steady.relative_tolerance,
            max_time: steady.max_time,
        });
    Ok(SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: plan.start,
        end: plan.end,
        output,
        preinit: preinit_values(&plan.preinit, &plain, model)?,
        changes: Vec::new(),
        steady_state,
        steady_state_after,
        before_events: false,
        selections: output_columns(selections),
        settings: settings.clone(),
        seed: sbmlsim_abi::DEFAULT_SEED,
        sensitivities: Vec::new(),
        local_errors: false,
    })
}

/// The pre-initialization assignments of a number by their target, the ones of the steady
/// state after the ones of the plan.
fn plain_values<'a>(assignments: &[&'a Assignment]) -> Vec<(&'a str, &'a Assignment)> {
    let mut plain: Vec<(&str, &Assignment)> = Vec::new();
    for a in assignments.iter().filter(|a| a.number().is_some()) {
        match plain.iter_mut().find(|(t, _)| *t == a.target) {
            Some(entry) => entry.1 = a,
            None => plain.push((&a.target, a)),
        }
    }
    plain
}

/// Pre-initialization assignments as the `(id, value)` pairs of the request, a formula
/// folded.
fn preinit_values(
    assignments: &[Assignment],
    plain: &[(&str, &Assignment)],
    model: &ModelInfo,
) -> Result<Vec<(String, f64)>, ScanError> {
    assignments
        .iter()
        .map(|a| {
            let value = match &a.value {
                Assigned::Number(x) => *x,
                Assigned::Formula(formula) => preinit_value(a, formula, plain, model)?,
            };
            Ok((a.target.clone(), value))
        })
        .collect()
}

/// The value of a formula before the initialization, sbmlsim's `_preinit_value`: a symbol
/// which the same pre-initialization sets to a number of a parameter is that number, else the
/// value of the model as loaded, which a pre-initialization of numbers must not be able to move
/// (a constant parameter); the time is 0.
fn preinit_value(
    assignment: &Assignment,
    formula: &str,
    plain: &[(&str, &Assignment)],
    model: &ModelInfo,
) -> Result<f64, ScanError> {
    let no_fold = |reason: String| {
        ScanError::unsupported(format!(
            "The engine 'core' does not simulate the formula '{formula}' of '{}' before the \
             initialization of the model '{}': {reason}. A value of a request of the engine is a \
             number.",
            assignment.target, model.label
        ))
    };
    let compiled = CompiledFormula::new(formula).map_err(ScanError::request)?;
    let mut values = Vec::with_capacity(compiled.symbols().len());
    for symbol in compiled.symbols() {
        if let Some((_, assigned)) = plain.iter().find(|(t, _)| t == symbol) {
            if assigned.kind != TargetKind::Parameter {
                return Err(no_fold(format!(
                    "it reads '{symbol}', which the pre-initialization sets as a {}",
                    assigned.kind.text()
                )));
            }
            values.push(assigned.number().expect("a plain value is a number"));
            continue;
        }
        let Some(loaded) = model.loaded_value(symbol) else {
            return Err(no_fold(format!(
                "it reads '{symbol}', whose value as loaded the description of the model does \
                 not carry"
            )));
        };
        if !plain.is_empty() && !model.constant_parameter(symbol) {
            return Err(no_fold(format!(
                "it reads '{symbol}', which the pre-initialization does not set to a number and \
                 which is no constant parameter of the model, so the values it sets could move \
                 it and they are not on the model when the formula is evaluated"
            )));
        }
        values.push(loaded);
    }
    // the clock is 0 before the initialization
    Ok(compiled.evaluate(&values, 0.0))
}

/// Refuses a change whose formula reads a symbol the engine has no output of, sbmlsim's
/// `_check_change`: the fold reads the state at the time of the change as the outputs of the
/// engine.
fn check_change(event: &PlanEvent, model: &ModelInfo) -> Result<(), ScanError> {
    for a in &event.assignments {
        let Assigned::Formula(formula) = &a.value else {
            continue;
        };
        let compiled = CompiledFormula::new(formula).map_err(ScanError::request)?;
        let mut unknown: Vec<&String> = compiled
            .symbols()
            .iter()
            .filter(|s| !model.has_selection(s))
            .collect();
        unknown.sort_unstable();
        if !unknown.is_empty() {
            return Err(ScanError::unsupported(format!(
                "The engine 'core' does not simulate the formula '{formula}' of '{}' at the time \
                 {} of the model '{}': it reads {}, which the engine has no output of, \
                 so the state of the model at the time of the change cannot be read.",
                a.target,
                crate::text::number(event.time),
                model.label,
                crate::text::list(&unknown)
            )));
        }
    }
    Ok(())
}
