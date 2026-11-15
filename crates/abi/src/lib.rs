//! The contracts of sbmlsim-core: `model/1` and `simulation/1`.
//!
//! A compiled model is a WebAssembly module which imports its linear memory and exports the
//! functions below. Every pointer is an `i32` offset into the imported memory and every value
//! is an `f64`, unless stated otherwise. `p` is the vector of the parameters (including the
//! compartments and species references), `y` the vector of the states, `t` the time.
//!
//! - `model_defaults(p, y)` writes the SBML values of the parameters and states.
//! - `model_init(t, p, y, fixed)` evaluates the initial assignments and the initial values of
//!   assignment rules in dependency order; `fixed` is a byte per entity (parameters, then
//!   states), a fixed entity keeps its value.
//! - `model_rhs(t, y, p, ydot)` is the right hand side of the ODEs.
//! - `model_jac(t, y, p, values)` writes the values of the Jacobian in CSC order of the pattern.
//! - `model_roots(t, y, p, g)` writes the [`ModelDescription::n_roots`] root functions of the
//!   triggers: one per comparison of each trigger (`a - b` of `a > b` and `a >= b`, `b - a` of
//!   `a < b` and `a <= b`, `a - b` of `a == b` and `a != b`, every adjacent pair of a chain) and
//!   one per number a trigger reads as a boolean, in the order of the events and, within a
//!   trigger, in the written order. The engine locates their zeros (in both directions) and then
//!   evaluates the triggers themselves with `model_trigger`.
//! - `model_trigger(i, t, y, p) -> i32` is the trigger of event `i` (1 true, 0 false).
//! - `model_priority(i, t, y, p) -> f64` is the priority of event `i`, 0 for an event without
//!   one.
//! - `model_delay(i, t, y, p) -> f64` is the delay of event `i`, 0 for an event without one.
//! - `model_event_values(i, t, y, p, out)` writes the values of the assignments of event `i`,
//!   [`EventInfo::n_assignments`] values in document order, each in the quantity its math gives
//!   (the concentration of a species of a concentration).
//! - `model_event_apply(i, values, y, p)` applies values of `model_event_values` of event `i`:
//!   every target gets its value, except that a species of a concentration keeps the amount
//!   `value * compartment` of its compartment before the assignments of the event (as
//!   libroadrunner 2.11 and the expected results of the SBML Test Suite, case 01506), and a
//!   species which stores its concentration keeps its amount when the event changes its
//!   compartment.
//! - `model_selections(t, y, p, out)` writes every value of [`ModelDescription::selections`],
//!   in its order.
//! - `model_assign(t, y, p)` writes the current values of the targets of assignment rules into
//!   `p` and `y` (after a change or an event).
//! - `model_sizes(out)` writes the sizes of the model as six `i32` (little endian) at `out`: the
//!   numbers of states, parameters, selections, root functions, events and event values (the
//!   sum of the assignments of the events), [`ModelSizes`]. The engine compares them with the
//!   description when it loads the model, so a `model.json` which does not belong to its
//!   `model.wasm` is an error and not a model which reads or writes past its vectors.
//!
//! Every function but `model_init`, `model_event_apply` and `model_assign` reads `p` and `y`
//! only, and every function computes the values of the assignment rules it reads itself, so it
//! does not depend on the values of the targets of the rules in `p` and `y`.
//!
//! Revisions of `model/1` before its first release: the field [`ModelDescription::n_roots`] and
//! the export `model_sizes` were added with the events (no version change, nothing was
//! released).

mod request;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use request::*;

/// The version of the description of a compiled model.
pub const MODEL_CONTRACT: &str = "model/1";
/// The version of the request and the result of a simulation.
pub const SIMULATION_CONTRACT: &str = "simulation/1";

/// Export name of `model_defaults`.
pub const DEFAULTS: &str = "model_defaults";
/// Export name of `model_init`.
pub const INIT: &str = "model_init";
/// Export name of `model_rhs`.
pub const RHS: &str = "model_rhs";
/// Export name of `model_jac`.
pub const JAC: &str = "model_jac";
/// Export name of `model_roots`.
pub const ROOTS: &str = "model_roots";
/// Export name of `model_trigger`.
pub const TRIGGER: &str = "model_trigger";
/// Export name of `model_priority`.
pub const PRIORITY: &str = "model_priority";
/// Export name of `model_delay`.
pub const DELAY: &str = "model_delay";
/// Export name of `model_event_values`.
pub const EVENT_VALUES: &str = "model_event_values";
/// Export name of `model_event_apply`.
pub const EVENT_APPLY: &str = "model_event_apply";
/// Export name of `model_selections`.
pub const SELECTIONS: &str = "model_selections";
/// Export name of `model_assign`.
pub const ASSIGN: &str = "model_assign";
/// Export name of `model_sizes`.
pub const SIZES: &str = "model_sizes";

/// A contract version which is not the expected one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("contract mismatch: found {found}, expected {expected}")]
pub struct ContractError {
    /// The version found in the data.
    pub found: String,
    /// The version this build supports.
    pub expected: String,
}

/// Checks that a contract version is the expected one.
pub fn check_contract(found: &str, expected: &str) -> Result<(), ContractError> {
    if found == expected {
        Ok(())
    } else {
        Err(ContractError {
            found: found.to_string(),
            expected: expected.to_string(),
        })
    }
}

/// The description of a compiled model, next to its WebAssembly module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDescription {
    /// The contract, [`MODEL_CONTRACT`].
    pub contract: String,
    /// The id of the model.
    pub id: String,
    /// The states, in the order of the state vector.
    pub states: Vec<Entity>,
    /// The parameters, in the order of the parameter vector.
    pub parameters: Vec<Entity>,
    /// The values `model_selections` writes, in its order.
    pub selections: Vec<Selection>,
    /// The Jacobian of the model.
    pub jacobian: JacobianKind,
    /// The events of the model.
    pub events: Vec<EventInfo>,
    /// The number of root functions `model_roots` writes.
    pub n_roots: u32,
    /// The unit expression of every entity id.
    pub units: BTreeMap<String, String>,
}

/// The sizes of a model, which `model_sizes` writes and the description gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSizes {
    /// The number of states.
    pub states: u32,
    /// The number of parameters.
    pub parameters: u32,
    /// The number of selections.
    pub selections: u32,
    /// The number of root functions.
    pub roots: u32,
    /// The number of events.
    pub events: u32,
    /// The number of event values, the sum of the assignments of the events.
    pub event_values: u32,
}

impl ModelSizes {
    /// The number of `i32` values `model_sizes` writes.
    pub const WORDS: usize = 6;

    /// The sizes a description gives.
    pub fn of(desc: &ModelDescription) -> ModelSizes {
        ModelSizes {
            states: desc.states.len() as u32,
            parameters: desc.parameters.len() as u32,
            selections: desc.selections.len() as u32,
            roots: desc.n_roots,
            events: desc.events.len() as u32,
            event_values: desc.events.iter().map(|e| e.n_assignments).sum(),
        }
    }

    /// The sizes in the order of `model_sizes`.
    pub fn to_words(self) -> [u32; Self::WORDS] {
        [
            self.states,
            self.parameters,
            self.selections,
            self.roots,
            self.events,
            self.event_values,
        ]
    }

    /// The sizes of the words `model_sizes` writes.
    pub fn from_words(words: [u32; Self::WORDS]) -> ModelSizes {
        let [states, parameters, selections, roots, events, event_values] = words;
        ModelSizes {
            states,
            parameters,
            selections,
            roots,
            events,
            event_values,
        }
    }
}

impl std::fmt::Display for ModelSizes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} states, {} parameters, {} selections, {} root functions, {} events, {} event \
             values",
            self.states,
            self.parameters,
            self.selections,
            self.roots,
            self.events,
            self.event_values
        )
    }
}

/// A state or a parameter of a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    /// The SBML id.
    pub id: String,
    /// What the entity is.
    pub kind: EntityKind,
    /// The initial value as loaded; JSON has no number for NaN and the infinities, so they are
    /// the strings `"NaN"`, `"inf"` and `"-inf"` (as the CSV of the command line writes them).
    #[serde(with = "nonfinite")]
    pub initial: f64,
    /// Whether a change may set it.
    pub settable: bool,
}

/// A number which may be NaN or infinite, as JSON: a finite number is a number, NaN and the
/// infinities are the strings `"NaN"`, `"inf"` and `"-inf"` (`serde_json` would write `null`,
/// which reads back as no number).
mod nonfinite {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(x: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        match *x {
            x if x.is_finite() => serializer.serialize_f64(x),
            x if x.is_nan() => serializer.serialize_str("NaN"),
            x if x > 0.0 => serializer.serialize_str("inf"),
            _ => serializer.serialize_str("-inf"),
        }
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Finite(f64),
        Text(String),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        let text = match Number::deserialize(deserializer)? {
            Number::Finite(x) => return Ok(x),
            Number::Text(text) => text,
        };
        match text.as_str() {
            "NaN" => Ok(f64::NAN),
            "inf" => Ok(f64::INFINITY),
            "-inf" => Ok(f64::NEG_INFINITY),
            other => Err(D::Error::custom(format!(
                "{other:?} is no number (a number, \"NaN\", \"inf\" or \"-inf\")"
            ))),
        }
    }
}

/// The kind of an [`Entity`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// A species.
    Species {
        /// The compartment of the species.
        compartment: String,
        /// The `hasOnlySubstanceUnits` attribute.
        has_only_substance_units: bool,
    },
    /// A compartment.
    Compartment,
    /// A parameter.
    Parameter,
    /// A species reference.
    SpeciesReference,
}

/// A value `model_selections` writes at `index`; `[S]` is the id of a concentration selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    /// The id of the selection.
    pub id: String,
    /// The position in the output of `model_selections`.
    pub index: u32,
}

/// A sparsity pattern in compressed sparse column form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SparsityPattern {
    /// The dimension of the square matrix.
    pub n: u32,
    /// The start of every column in `row_idx`, `n + 1` entries.
    pub col_ptr: Vec<u32>,
    /// The row of every entry.
    pub row_idx: Vec<u32>,
}

/// How the Jacobian is provided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JacobianKind {
    /// `model_jac` computes it on the pattern.
    Analytic(SparsityPattern),
    /// Difference quotients over the colors of the columns of the pattern.
    ColoredDifference {
        /// The pattern.
        pattern: SparsityPattern,
        /// The color of every column.
        colors: Vec<u32>,
    },
}

/// What the engine needs to know of an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventInfo {
    /// The id of the event, empty for an event without one.
    pub id: String,
    /// Whether the event has a delay.
    pub has_delay: bool,
    /// Whether the event has a priority.
    pub has_priority: bool,
    /// Whether the trigger is persistent.
    pub persistent: bool,
    /// The initial value of the trigger.
    pub initial_value: bool,
    /// Whether the assignments use the values at the time of the trigger.
    pub use_values_from_trigger_time: bool,
    /// The number of assignments.
    pub n_assignments: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_request() -> SimulationRequest {
        SimulationRequest {
            contract: SIMULATION_CONTRACT.to_string(),
            start: -1.0,
            end: 10.5,
            output: Output::Times(vec![0.0, 1.0, 2.5]),
            preinit: vec![("k".to_string(), 2.0)],
            changes: vec![Change {
                time: 3.0,
                values: vec![("dose".to_string(), 100.0), ("V".to_string(), 1.5)],
            }],
            steady_state: Some(SteadyState {
                preinit: vec![("x".to_string(), 1.0)],
                absolute_tolerance: 1e-9,
                relative_tolerance: 1e-7,
                max_time: 1e6,
            }),
            selections: vec!["time".to_string(), "[S]".to_string()],
            settings: IntegratorSettings {
                relative_tolerance: 1e-8,
                absolute_tolerance: Tolerance::PerState(vec![1e-12, 1e-10]),
                maximum_num_steps: 500,
                maximum_time_step: 0.5,
                initial_time_step: 1e-6,
                stiff: false,
                linear_solver: LinearSolver::Klu,
                jacobian: JacobianChoice::DifferenceQuotient,
            },
            seed: 42,
        }
    }

    #[test]
    fn request_roundtrips_as_json() {
        let request = full_request();
        let json = serde_json::to_string(&request).unwrap();
        let back: SimulationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(request, back);
        for output in [Output::Uniform { steps: 7 }, Output::IntegratorSteps] {
            let r = SimulationRequest {
                output,
                ..full_request()
            };
            let back: SimulationRequest =
                serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(r, back);
        }
    }

    #[test]
    fn misspelled_field_is_an_error() {
        let mut value = serde_json::to_value(full_request()).unwrap();
        value["sed"] = serde_json::json!(1);
        assert!(serde_json::from_value::<SimulationRequest>(value).is_err());
    }

    #[test]
    fn contract_mismatch_names_both() {
        let message = check_contract("model/2", MODEL_CONTRACT)
            .unwrap_err()
            .to_string();
        assert!(message.contains("model/2") && message.contains("model/1"));
        assert!(check_contract(MODEL_CONTRACT, MODEL_CONTRACT).is_ok());
    }

    #[test]
    fn settings_default_to_roadrunner() {
        let s = IntegratorSettings::default();
        assert_eq!(s.relative_tolerance, 1e-6);
        assert_eq!(s.absolute_tolerance, Tolerance::Scalar(1e-12));
        assert_eq!(s.maximum_num_steps, 20000);
        assert_eq!(s.maximum_time_step, 0.0);
        assert_eq!(s.initial_time_step, 0.0);
        assert!(s.stiff);
        assert_eq!(s.linear_solver, LinearSolver::Auto);
        assert_eq!(s.jacobian, JacobianChoice::Auto);
        let t = SteadyState::default();
        assert_eq!(
            (t.absolute_tolerance, t.relative_tolerance, t.max_time),
            (1e-8, 1e-6, 1e8)
        );
    }

    #[test]
    fn initial_values_which_are_not_finite_roundtrip() {
        let entity = |initial| Entity {
            id: "x".to_string(),
            kind: EntityKind::Parameter,
            initial,
            settable: true,
        };
        for (x, text) in [
            (f64::NAN, r#""NaN""#),
            (f64::INFINITY, r#""inf""#),
            (f64::NEG_INFINITY, r#""-inf""#),
            (0.1, "0.1"),
            (-0.0, "-0.0"),
        ] {
            let json = serde_json::to_string(&entity(x)).unwrap();
            assert!(json.contains(&format!(r#""initial":{text}"#)), "{json}");
            let back: Entity = serde_json::from_str(&json).unwrap();
            assert!(back.initial.to_bits() == x.to_bits() || (x.is_nan() && back.initial.is_nan()));
        }
        let json = r#"{"id":"x","kind":"parameter","initial":"infinity","settable":true}"#;
        let error = serde_json::from_str::<Entity>(json)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no number"), "{error}");
        assert!(serde_json::from_str::<Entity>(&json.replace(r#""infinity""#, "null")).is_err());
    }

    #[test]
    fn description_roundtrips_as_json() {
        let d = ModelDescription {
            contract: MODEL_CONTRACT.to_string(),
            id: "m".to_string(),
            states: vec![Entity {
                id: "S".to_string(),
                kind: EntityKind::Species {
                    compartment: "c".to_string(),
                    has_only_substance_units: false,
                },
                initial: 1.0,
                settable: true,
            }],
            parameters: vec![],
            selections: vec![Selection {
                id: "[S]".to_string(),
                index: 0,
            }],
            jacobian: JacobianKind::Analytic(SparsityPattern {
                n: 1,
                col_ptr: vec![0, 1],
                row_idx: vec![0],
            }),
            events: vec![EventInfo {
                id: "e".to_string(),
                has_delay: true,
                has_priority: false,
                persistent: false,
                initial_value: true,
                use_values_from_trigger_time: true,
                n_assignments: 2,
            }],
            n_roots: 3,
            units: BTreeMap::new(),
        };
        let sizes = ModelSizes::of(&d);
        assert_eq!(sizes.to_words(), [1, 0, 1, 3, 1, 2]);
        assert_eq!(ModelSizes::from_words(sizes.to_words()), sizes);
        let back: ModelDescription =
            serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(d, back);
    }
}
