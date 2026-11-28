//! The plans of a scan, the contract `plan/1`: every simulation of a scan compiled against
//! every one of its models, as a host which simulates elsewhere than in the engine reads them
//! (sbmlsim's executors of roadrunner, COPASI and AMICI, and its fit).
//!
//! [`compile_plans`] compiles the simulation of a scan (or every simulation of its dimension of
//! simulations) against the model of the run (or every model of its dimension of models) and
//! gives, per simulation and model ([`CompiledPlan`]):
//!
//! - `plan`, the [`Plan`]: the start and the end in the time unit of the model, the
//!   assignments before the initialization, the assignments of the changes merged into one
//!   event per time, the steady state before the start, the output (the steps of the integrator
//!   or the output times, merged within the resolution of the time), the shift of the time of
//!   the result and the steady state after the end, every quantity in the unit of its target in
//!   the model and the kind of every target resolved; a formula stays text;
//! - `values`, the values of every dimension of values in the units of that model (a dimension
//!   of simulations or of models has none), which a host applies to the plan of a point as
//!   `Plan::with_values` does;
//! - `at`, the time of every dimension with `at` in the time unit of that model.
//!
//! The plan of a point of the scan is the plan of its simulation and model with the values of
//! the point applied ([`crate::Scan::point_plan`]), which is what the planner sends the engine.
//!
//! # JSON
//!
//! `{"contract": "plan/1", "plans": [...]}`, the plans by their key `s * n_models + m` of the
//! simulation `s` and the model `m` (sbmlsim's order: every model of a simulation, then the next
//! simulation). A plan is
//!
//! ```json
//! {"simulation": 0, "model": null,
//!  "plan": {"start": 0.0, "end": 10.0, "time_shift": 0.0,
//!           "preinit": [{"target": "k", "kind": "parameter", "value": 1.0}],
//!           "events": [{"time": 5.0, "assignments": [{"target": "[S]",
//!                       "kind": "species concentration", "formula": "S0 + 1"}]}],
//!           "steady_state": null, "output": "times", "times": [0.0, 5.0, 10.0],
//!           "steady_state_output": null},
//!  "values": [[{"target": "k", "values": [1.0, 2.0]}]],
//!  "at": [null]}
//! ```
//!
//! with `model` the key of the model in the dimension of models (`null` for the model of the
//! run), the kinds of sbmlsim's `TargetKind` (`parameter`, `compartment`, `species amount`,
//! `species concentration`), an assignment of a `value` or of a `formula`, a steady state
//! `{"preinit", "absolute_tolerance", "relative_tolerance", "max_time"}` and an output
//! `integrator` (no times) or `times`. A number is a number, `NaN` is `null` and the
//! infinities are the strings `"inf"` and `"-inf"`, as in `scanresult/1`, so the plans read back
//! as the plans, every number by its bits.

use serde::{Deserialize, Serialize};

use crate::error::ScanError;
use crate::expand::{Expanded, check_scan};
use crate::model::TargetKind;
use crate::plan::{Assigned, Assignment, OutputMode, Plan, PlanEvent, SteadyStatePlan};
use crate::request::{Ordered, SCAN_CONTRACT, ScanDefinition, Value};
use crate::{changes_of, resolve_models};
use sbmlsim_abi::ModelDescription;

/// The contract of the plans of a scan.
pub const PLAN_CONTRACT: &str = "plan/1";

/// A number of JSON, see the module: a number, `null` for `NaN`, `"inf"` and `"-inf"` for the
/// infinities.
mod number {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// A number as JSON writes it.
    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    pub(super) enum Number {
        Finite(f64),
        Text(String),
        Null(()),
    }

    pub(super) fn encode(x: f64) -> Number {
        if x.is_finite() {
            Number::Finite(x)
        } else if x.is_nan() {
            Number::Null(())
        } else if x > 0.0 {
            Number::Text("inf".to_string())
        } else {
            Number::Text("-inf".to_string())
        }
    }

    pub(super) fn decode<E: serde::de::Error>(number: Number) -> Result<f64, E> {
        match number {
            Number::Finite(x) => Ok(x),
            Number::Null(()) => Ok(f64::NAN),
            Number::Text(text) => match text.as_str() {
                "NaN" => Ok(f64::NAN),
                "inf" => Ok(f64::INFINITY),
                "-inf" => Ok(f64::NEG_INFINITY),
                other => Err(E::custom(format!(
                    "{other:?} is no number (a number, null, \"inf\" or \"-inf\")"
                ))),
            },
        }
    }

    pub fn serialize<S: Serializer>(x: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        encode(*x).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        decode(Number::deserialize(deserializer)?)
    }

    /// The numbers of an array.
    pub mod many {
        use serde::ser::SerializeSeq;
        use serde::{Deserialize, Deserializer, Serializer};

        use super::{Number, decode, encode};

        pub fn serialize<S: Serializer>(values: &[f64], serializer: S) -> Result<S::Ok, S::Error> {
            let mut seq = serializer.serialize_seq(Some(values.len()))?;
            for x in values {
                seq.serialize_element(&encode(*x))?;
            }
            seq.end()
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<f64>, D::Error> {
            Vec::<Number>::deserialize(deserializer)?
                .into_iter()
                .map(decode)
                .collect()
        }
    }

    /// A number of a field which may be absent: absent for none, so a present `null` is
    /// `NaN`.
    pub mod field {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        use super::{Number, decode, encode};

        pub fn serialize<S: Serializer>(x: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
            x.map(encode).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<f64>, D::Error> {
            decode(Number::deserialize(deserializer)?).map(Some)
        }
    }

    /// Times which may be absent, `null` for none.
    pub mod optional_many {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        use super::{Number, decode, encode};

        pub fn serialize<S: Serializer>(
            values: &[Option<f64>],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            values
                .iter()
                .map(|x| x.map(encode))
                .collect::<Vec<_>>()
                .serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<Option<f64>>, D::Error> {
            Vec::<Option<Number>>::deserialize(deserializer)?
                .into_iter()
                .map(|x| x.map(decode).transpose())
                .collect()
        }
    }
}

/// The plans of a scan, see the module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPlans {
    /// The contract, [`PLAN_CONTRACT`].
    pub contract: String,
    /// The plan of every simulation and model, by its key.
    pub plans: Vec<CompiledPlan>,
}

/// One simulation of a scan compiled against one of its models, see the module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPlan {
    /// The index of the simulation in the dimension of simulations, 0 without one.
    pub simulation: usize,
    /// The key of the model in the dimension of models, `None` for the model of the run.
    pub model: Option<String>,
    /// The plan.
    pub plan: PlanDocument,
    /// Per dimension the values of its targets in the units of the model, in the order of the
    /// request (empty for a dimension of simulations or of models).
    pub values: Vec<Vec<TargetValues>>,
    /// Per dimension the time of its values in the time unit of the model, `None` for a
    /// dimension without `at`.
    #[serde(with = "number::optional_many")]
    pub at: Vec<Option<f64>>,
}

/// The values of one target of a dimension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetValues {
    /// The target.
    pub target: String,
    /// The value of every point of the dimension, in the unit of the target in the model.
    #[serde(with = "number::many")]
    pub values: Vec<f64>,
}

/// A [`Plan`] as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDocument {
    /// The start, in the time unit of the model.
    #[serde(with = "number")]
    pub start: f64,
    /// The end.
    #[serde(with = "number")]
    pub end: f64,
    /// Added to the time of the result.
    #[serde(with = "number")]
    pub time_shift: f64,
    /// The assignments before the initialization.
    pub preinit: Vec<AssignmentDocument>,
    /// The assignments at times, sorted by time.
    pub events: Vec<EventDocument>,
    /// The steady state before the start.
    pub steady_state: Option<SteadyStateDocument>,
    /// `integrator` or `times`.
    pub output: String,
    /// The output times of `times`, empty otherwise.
    #[serde(with = "number::many")]
    pub times: Vec<f64>,
    /// The steady state after the end, the output at the time `inf`.
    pub steady_state_output: Option<SteadyStateDocument>,
}

/// An [`Assignment`] as JSON: a `value` or a `formula`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignmentDocument {
    /// The target.
    pub target: String,
    /// The kind of the target, sbmlsim's `TargetKind`.
    pub kind: String,
    /// The number, in the unit of the target in the model.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "number::field"
    )]
    pub value: Option<f64>,
    /// The formula of the math of PEtab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
}

/// A [`PlanEvent`] as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventDocument {
    /// The time in the time unit of the model.
    #[serde(with = "number")]
    pub time: f64,
    /// The assignments, evaluated together and then applied.
    pub assignments: Vec<AssignmentDocument>,
}

/// A [`SteadyStatePlan`] as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteadyStateDocument {
    /// The assignments before the initialization of the steady state.
    pub preinit: Vec<AssignmentDocument>,
    /// The absolute tolerance of the rates of change.
    #[serde(with = "number")]
    pub absolute_tolerance: f64,
    /// The relative tolerance of the rates of change.
    #[serde(with = "number")]
    pub relative_tolerance: f64,
    /// The longest pre-equilibration, in the time unit of the model.
    #[serde(with = "number")]
    pub max_time: f64,
}

/// The kind of a target by its text, sbmlsim's `TargetKind`.
fn kind_of(text: &str) -> Result<TargetKind, ScanError> {
    [
        TargetKind::Parameter,
        TargetKind::Compartment,
        TargetKind::SpeciesAmount,
        TargetKind::SpeciesConcentration,
    ]
    .into_iter()
    .find(|kind| kind.text() == text)
    .ok_or_else(|| ScanError::request(format!("'{text}' is no kind of a target.")))
}

impl From<&Assignment> for AssignmentDocument {
    fn from(a: &Assignment) -> Self {
        let (value, formula) = match &a.value {
            Assigned::Number(x) => (Some(*x), None),
            Assigned::Formula(f) => (None, Some(f.clone())),
        };
        AssignmentDocument {
            target: a.target.clone(),
            kind: a.kind.text().to_string(),
            value,
            formula,
        }
    }
}

impl AssignmentDocument {
    /// The assignment.
    ///
    /// # Errors
    ///
    /// An unknown kind, or not exactly one of a value and a formula.
    pub fn assignment(&self) -> Result<Assignment, ScanError> {
        let value = match (&self.value, &self.formula) {
            (Some(x), None) => Assigned::Number(*x),
            (None, Some(f)) => Assigned::Formula(f.clone()),
            _ => {
                return Err(ScanError::request(format!(
                    "The assignment of '{}' has a value or a formula, not both or neither.",
                    self.target
                )));
            }
        };
        Ok(Assignment {
            target: self.target.clone(),
            kind: kind_of(&self.kind)?,
            value,
        })
    }
}

fn documents(assignments: &[Assignment]) -> Vec<AssignmentDocument> {
    assignments.iter().map(AssignmentDocument::from).collect()
}

fn assignments(documents: &[AssignmentDocument]) -> Result<Vec<Assignment>, ScanError> {
    documents
        .iter()
        .map(AssignmentDocument::assignment)
        .collect()
}

impl From<&SteadyStatePlan> for SteadyStateDocument {
    fn from(s: &SteadyStatePlan) -> Self {
        SteadyStateDocument {
            preinit: documents(&s.preinit),
            absolute_tolerance: s.absolute_tolerance,
            relative_tolerance: s.relative_tolerance,
            max_time: s.max_time,
        }
    }
}

impl SteadyStateDocument {
    /// The steady state.
    ///
    /// # Errors
    ///
    /// An assignment which is none, see [`AssignmentDocument::assignment`].
    pub fn steady_state(&self) -> Result<SteadyStatePlan, ScanError> {
        Ok(SteadyStatePlan {
            preinit: assignments(&self.preinit)?,
            absolute_tolerance: self.absolute_tolerance,
            relative_tolerance: self.relative_tolerance,
            max_time: self.max_time,
        })
    }
}

impl From<&Plan> for PlanDocument {
    fn from(plan: &Plan) -> Self {
        PlanDocument {
            start: plan.start,
            end: plan.end,
            time_shift: plan.time_shift,
            preinit: documents(&plan.preinit),
            events: plan
                .events
                .iter()
                .map(|e| EventDocument {
                    time: e.time,
                    assignments: documents(&e.assignments),
                })
                .collect(),
            steady_state: plan.steady_state.as_ref().map(SteadyStateDocument::from),
            output: match plan.output {
                OutputMode::Integrator => "integrator",
                OutputMode::Times => "times",
            }
            .to_string(),
            times: plan.times.clone(),
            steady_state_output: plan
                .steady_state_output
                .as_ref()
                .map(SteadyStateDocument::from),
        }
    }
}

impl PlanDocument {
    /// The plan.
    ///
    /// # Errors
    ///
    /// An unknown output or kind, or an assignment which is none.
    pub fn plan(&self) -> Result<Plan, ScanError> {
        let output = match self.output.as_str() {
            "integrator" => OutputMode::Integrator,
            "times" => OutputMode::Times,
            other => {
                return Err(ScanError::request(format!(
                    "The output of a plan is 'integrator' or 'times', not '{other}'."
                )));
            }
        };
        Ok(Plan {
            start: self.start,
            end: self.end,
            preinit: assignments(&self.preinit)?,
            events: self
                .events
                .iter()
                .map(|e| {
                    Ok(PlanEvent {
                        time: e.time,
                        assignments: assignments(&e.assignments)?,
                    })
                })
                .collect::<Result<_, ScanError>>()?,
            steady_state: self
                .steady_state
                .as_ref()
                .map(SteadyStateDocument::steady_state)
                .transpose()?,
            output,
            times: self.times.clone(),
            time_shift: self.time_shift,
            steady_state_output: self
                .steady_state_output
                .as_ref()
                .map(SteadyStateDocument::steady_state)
                .transpose()?,
        })
    }
}

/// What the plans of a scan are compiled from: the scan and the changes of its models, the
/// fields of a request `scan/1` which decide its plans (the other
/// fields of such a request are read past, so a request `scan/1` is one).
#[derive(Debug, Clone, Deserialize)]
pub struct PlanRequest {
    /// The contract, [`SCAN_CONTRACT`].
    pub contract: String,
    /// The scan.
    pub scan: ScanDefinition,
    /// The changes of the model of the run, the defaults of the pre-initialization of its
    /// simulations.
    #[serde(default)]
    pub changes: Ordered<Value>,
    /// The changes of the models of a dimension of models by their key, the defaults of the
    /// pre-initialization of their simulations.
    #[serde(default)]
    pub model_changes: Ordered<Ordered<Value>>,
}

/// Compiles every simulation of a scan against every one of its models, see the module.
///
/// The models are the model of the run (`description`) or, for a scan with a dimension of
/// models, the model of every key of that dimension (`models`), as in
/// [`crate::Scan::open_with_models`], with the changes of every model (`changes`,
/// `model_changes`).
///
/// # Errors
///
/// A request which does not fit itself or its models: the checks of the scan and of its models,
/// and every check of compiling a simulation against a model, its values and times, with the
/// messages of [`crate::Scan::open_with_models`].
pub fn compile_plans(
    request: &PlanRequest,
    description: Option<ModelDescription>,
    models: Vec<(String, ModelDescription)>,
) -> Result<CompiledPlans, ScanError> {
    if request.contract != SCAN_CONTRACT {
        return Err(ScanError::request(format!(
            "The contract of the request is '{}', not '{SCAN_CONTRACT}'.",
            request.contract
        )));
    }
    check_scan(&request.scan)?;
    let (infos, keys, labels) = resolve_models(
        &request.scan,
        &[],
        &request.changes,
        &request.model_changes,
        description,
        models,
    )?;
    let changes = changes_of(&keys, &request.changes, &request.model_changes);
    let expanded = Expanded::new(&request.scan, &infos, &labels, &changes)?;
    Ok(compiled_plans(&expanded, &keys))
}

/// The plans of a compiled scan, by their keys.
pub(crate) fn compiled_plans(expanded: &Expanded, keys: &[Option<String>]) -> CompiledPlans {
    let plans = expanded
        .plans
        .iter()
        .enumerate()
        .map(|(key, plan)| {
            let m = expanded.model_of_plan(key);
            CompiledPlan {
                simulation: key / expanded.n_models,
                model: keys[m].clone(),
                plan: PlanDocument::from(plan),
                values: expanded.vectors[m]
                    .iter()
                    .map(|vector| {
                        vector
                            .values
                            .iter()
                            .map(|(target, values)| TargetValues {
                                target: target.clone(),
                                values: values.clone(),
                            })
                            .collect()
                    })
                    .collect(),
                at: expanded.at_times[key].clone(),
            }
        })
        .collect();
    CompiledPlans {
        contract: PLAN_CONTRACT.to_string(),
        plans,
    }
}
