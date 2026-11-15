//! The contracts of sbmlsim-core: `model/2` and `simulation/1`.
//!
//! A compiled model is a WebAssembly module which imports its linear memory and exports the
//! functions below. Every pointer is an `i32` offset into the imported memory and every value
//! is an `f64`, unless stated otherwise. `p` is the vector of the parameters (including the
//! compartments and species references), followed by the [`ModelDescription::n_constants`]
//! constants of the model, `y` the vector of the states, `t` the time.
//!
//! The constants are the values of the subexpressions of the math which depend on nothing but
//! the parameters which no assignment rule assigns (directly or through the assignment rules
//! of such parameters alone): `pow(BW, 0.75)` of a parameter `BW`, say. `model_init` and
//! `model_assign` compute them into `p` behind the parameters; the other functions read them
//! there instead of computing them in every call, with the bits of the computation. The engine
//! calls `model_assign` after every change of a parameter (by a change of the request or an
//! event), so the constants are those of the current parameters whenever a function reads
//! them.
//!
//! - `model_defaults(p, y)` writes the SBML values of the parameters and states.
//! - `model_init(t, p, y, fixed)` evaluates the initial assignments and the initial values of
//!   assignment rules in dependency order, then the constants; `fixed` is a byte per entity
//!   (parameters, then states), a fixed entity keeps its value.
//! - `model_rhs(t, y, p, ydot)` is the right hand side of the ODEs.
//! - `model_jac(t, y, p, values)` writes the values of the Jacobian in CSC order of the pattern
//!   (an analytic Jacobian); it traps for a model of a colored difference quotient.
//! - `model_roots(t, y, p, g)` writes the [`ModelDescription::n_roots`] root functions of the
//!   triggers: one per comparison of each trigger (`a - b` of `a > b` and `a >= b`, `b - a` of
//!   `a < b` and `a <= b`, `a - b` of `a == b` and `a != b`, every adjacent pair of a chain) and
//!   one per number a trigger reads as a boolean, in the order of the events and, within a
//!   trigger, in the written order. The engine locates their zeros (in both directions) and then
//!   evaluates the triggers themselves with `model_trigger`. After them come the
//!   [`ModelDescription::n_discontinuity_roots`] root functions of the discontinuities of the
//!   math: one per branch (but the `otherwise`) of every `piecewise` of the assignment rules,
//!   the rates and the triggers, which changes its sign where the branch the `piecewise` selects
//!   changes (`a - b` of a condition `a > b`, `min` and `max` of the conditions of `and` and
//!   `or`, see `sbmlsim_ir::Model::discontinuity_roots`), without those which depend neither on
//!   the time nor on a state. At their zeros the engine starts the integrator again from the
//!   state there, as libroadrunner does at its piecewise triggers.
//! - `model_trigger(i, t, y, p) -> i32` is the trigger of event `i` (1 true, 0 false).
//! - `model_priority(i, t, y, p) -> f64` is the priority of event `i`, 0 for an event without
//!   one.
//! - `model_delay(i, t, y, p) -> f64` is the delay of event `i`, 0 for an event without one.
//! - `model_event_values(i, t, y, p, out)` writes the values of the assignments of event `i`,
//!   [`EventInfo::n_assignments`] values in document order, each in the quantity its math gives
//!   (the concentration of a species of a concentration).
//! - `model_event_apply(i, t, values, y, p)` applies values of `model_event_values` of event `i`
//!   at the time `t`, as libroadrunner 2.11 does (and the expected results of the SBML Test
//!   Suite, case 01506): the targets in document order, the compartments last; a species of a
//!   concentration gets the amount `value * compartment` of its compartment at its turn (a
//!   compartment the event assigns is still the one before the event, one of an assignment rule
//!   follows the assignments before it), and every other species keeps its amount, also one
//!   which stores its concentration (the target of a rate rule) when its compartment changes
//!   (directly or through an assignment rule).
//! - `model_selections(t, y, p, mask, out)` writes the values of
//!   [`ModelDescription::selections`] whose byte of `mask` (one per selection, in its order) is
//!   not 0, each at its position in `out`; it may write others too (the compiler computes only
//!   the cheap ones, a load or two, without looking at the mask).
//! - `model_assign(t, y, p)` writes the current values of the targets of assignment rules into
//!   `p` and `y`, then the constants into `p` (after a change or an event).
//! - `model_sizes(out)` writes the sizes of the model as seven `i32` (little endian) at `out`:
//!   the numbers of states, parameters, selections, entries of the Jacobian, root functions,
//!   constants and events. `model_event_sizes(out)` writes the number of assignments of every event, one `i32`
//!   per event. Together they are [`ModelSizes`], the length of every vector the engine lends to
//!   the model (`p`, `y`, `fixed`, `ydot`, the values of the Jacobian, `g`, the values of each
//!   event, the selections). The engine compares them with the description when it loads the
//!   model, so a `model.json` which does not belong to its `model.wasm` is an error and not a
//!   model which reads or writes past its vectors.
//!
//! Every function but `model_init`, `model_event_apply` and `model_assign` reads `p` and `y`
//! only, and every function computes the values of the assignment rules it reads itself, so it
//! does not depend on the values of the targets of the rules in `p` and `y`.
//!
//! `model/2` is `model/1` with the constants ([`ModelDescription::n_constants`], their word in
//! `model_sizes`, their computation by `model_init` and `model_assign`) and the `mask` of
//! `model_selections`, which make the right hand side and the output faster.
//!
//! Revisions of `model/1` before it was replaced by `model/2` (neither was released): the
//! field [`ModelDescription::n_roots`], the exports `model_sizes` and `model_event_sizes` and
//! the time of `model_event_apply` were added with the events, the field
//! `stores_concentration` of [`EntityKind::Species`] with the changes, the field
//! [`ModelDescription::n_discontinuity_roots`] and the root functions of the discontinuities
//! after those of the triggers with the root finding of the conditions of `piecewise`, the
//! field [`ModelDescription::compiler`] (the compiler and its version) before the contracts
//! were frozen (no version change, nothing was released).
//!
//! Revisions of `simulation/1` before its first release: the `seed` of a request is 1 when it
//! is absent ([`DEFAULT_SEED`]), set before the contracts were frozen (no version change).
//!
//! `engine.wasm` carries the custom section [`ENGINE_SECTION`] with the JSON
//! `{"memory_minimum_pages":n,"output_format":k}`: the minimum of the memory it imports and
//! [`ENGINE_OUTPUT_FORMAT`], the version of the layout of its output and its counters, which
//! the hosts check before they use it.

mod request;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use request::*;

/// The version of the description of a compiled model.
pub const MODEL_CONTRACT: &str = "model/2";
/// The version of the request and the result of a simulation.
pub const SIMULATION_CONTRACT: &str = "simulation/1";

/// The custom section of `engine.wasm` with the minimum of the memory it imports and the
/// version of its output.
pub const ENGINE_SECTION: &str = "sbmlsim.engine";

/// The version of the output of `engine.wasm`: the layout of `engine_output_ptr` (the length of
/// the JSON header, the header, the values) and of `engine_stats_ptr` (the words of
/// [`SimulationStats`]), as `crates/engine/src/exports.rs` documents them. A host refuses an
/// `engine.wasm` of another version.
pub const ENGINE_OUTPUT_FORMAT: u32 = 1;

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
/// Export name of `model_event_sizes`.
pub const EVENT_SIZES: &str = "model_event_sizes";

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
    /// The compiler which compiled the model and its version, e.g. `sbmlsim-compiler 0.1.0`.
    pub compiler: String,
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
    /// The number of root functions of the discontinuities of the math, the last ones of the
    /// `n_roots` (the ones before them are those of the triggers of the events).
    pub n_discontinuity_roots: u32,
    /// The number of constants of the model, which `p` holds behind the parameters (see the
    /// crate documentation).
    pub n_constants: u32,
    /// The unit expression of every entity id.
    pub units: BTreeMap<String, String>,
}

/// The sizes of a model, which `model_sizes` and `model_event_sizes` write and the
/// description gives: the length of every vector the engine lends to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSizes {
    /// The number of states.
    pub states: u32,
    /// The number of parameters.
    pub parameters: u32,
    /// The number of selections.
    pub selections: u32,
    /// The number of entries of the pattern of the Jacobian.
    pub jacobian_entries: u32,
    /// The number of root functions.
    pub roots: u32,
    /// The number of constants (behind the parameters in `p`).
    pub constants: u32,
    /// The number of events.
    pub events: u32,
    /// The number of assignments of every event, one per event (of a model whose number of
    /// events is not the one of its description, the engine does not read them: empty).
    pub assignments: Vec<u32>,
}

impl ModelSizes {
    /// The number of `i32` values `model_sizes` writes.
    pub const WORDS: usize = 7;

    /// The sizes a description gives.
    pub fn of(desc: &ModelDescription) -> ModelSizes {
        let pattern = desc.jacobian.pattern();
        ModelSizes {
            states: desc.states.len() as u32,
            parameters: desc.parameters.len() as u32,
            selections: desc.selections.len() as u32,
            jacobian_entries: pattern.row_idx.len() as u32,
            roots: desc.n_roots,
            constants: desc.n_constants,
            events: desc.events.len() as u32,
            assignments: desc.events.iter().map(|e| e.n_assignments).collect(),
        }
    }

    /// The words of `model_sizes`: the states, parameters, selections, entries of the Jacobian,
    /// root functions, constants and events.
    pub fn words(&self) -> [u32; Self::WORDS] {
        [
            self.states,
            self.parameters,
            self.selections,
            self.jacobian_entries,
            self.roots,
            self.constants,
            self.events,
        ]
    }

    /// The sizes of the words of `model_sizes` and the numbers of assignments of
    /// `model_event_sizes` (as many as the last word says, or none when they were not read).
    pub fn from_words(words: [u32; Self::WORDS], assignments: Vec<u32>) -> ModelSizes {
        let [
            states,
            parameters,
            selections,
            jacobian_entries,
            roots,
            constants,
            events,
        ] = words;
        ModelSizes {
            states,
            parameters,
            selections,
            jacobian_entries,
            roots,
            constants,
            events,
            assignments,
        }
    }
}

impl std::fmt::Display for ModelSizes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} states, {} parameters, {} selections, {} entries of the Jacobian, {} root \
             functions, {} constants, {} events of {:?} assignments",
            self.states,
            self.parameters,
            self.selections,
            self.jacobian_entries,
            self.roots,
            self.constants,
            self.events,
            self.assignments
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
        /// Whether the entity stores the concentration of the species (a species of a
        /// concentration which is the target of a rule), else its amount.
        stores_concentration: bool,
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

impl SparsityPattern {
    /// The number of entries.
    pub fn nnz(&self) -> usize {
        self.row_idx.len()
    }

    /// The rows of the entries of column `j`.
    ///
    /// # Panics
    ///
    /// For a column out of range or a pattern which is not a CSC pattern.
    pub fn rows(&self, j: usize) -> &[u32] {
        &self.row_idx[self.col_ptr[j] as usize..self.col_ptr[j + 1] as usize]
    }

    /// Whether the pattern is a CSC pattern of `n` states: `n + 1` column pointers from 0, not
    /// decreasing, to the number of entries, and every row below `n`.
    pub fn is_csc_of(&self, n: usize) -> bool {
        self.n as usize == n
            && self.col_ptr.len() == n + 1
            && self.col_ptr[0] == 0
            && self.col_ptr.windows(2).all(|w| w[0] <= w[1])
            && self.col_ptr[n] as usize == self.row_idx.len()
            && self.row_idx.iter().all(|i| (*i as usize) < n)
    }

    /// The greedy coloring of the columns of a CSC pattern, as `JacobianSparsity` of the
    /// sparse difference quotient prototype of libroadrunner (`roadrunner-sparsedq`): a
    /// distance-1 coloring of the column intersection graph, the columns with the most entries
    /// first (ties by column), each the smallest color no column with which it shares a row
    /// has. Two columns of a color share no row, so one evaluation of the right hand side with
    /// the increments of every column of a color gives the differences of all of them.
    ///
    /// # Panics
    ///
    /// For a pattern which is not a CSC pattern ([`SparsityPattern::is_csc_of`]).
    pub fn greedy_colors(&self) -> Vec<u32> {
        let n = self.n as usize;
        // the columns of every row (the transposed pattern)
        let mut row_ptr = vec![0usize; n + 1];
        for &i in &self.row_idx {
            row_ptr[i as usize + 1] += 1;
        }
        for i in 0..n {
            row_ptr[i + 1] += row_ptr[i];
        }
        let mut next = row_ptr.clone();
        let mut columns = vec![0usize; self.row_idx.len()];
        for j in 0..n {
            for &i in self.rows(j) {
                columns[next[i as usize]] = j;
                next[i as usize] += 1;
            }
        }
        let mut order: Vec<usize> = (0..n).collect();
        // a stable sort: columns of as many entries keep their order
        order.sort_by_key(|&j| std::cmp::Reverse(self.rows(j).len()));
        const NONE: u32 = u32::MAX;
        let mut colors = vec![NONE; n];
        // `forbidden[c] == j`: a column which shares a row with `j` has the color `c`
        let mut forbidden = vec![usize::MAX; n + 1];
        for &j in &order {
            for &i in self.rows(j) {
                let i = i as usize;
                for &other in &columns[row_ptr[i]..row_ptr[i + 1]] {
                    if colors[other] != NONE {
                        forbidden[colors[other] as usize] = j;
                    }
                }
            }
            let mut c = 0;
            while forbidden[c] == j {
                c += 1;
            }
            colors[j] = c as u32;
        }
        colors
    }

    /// Checks a coloring of the columns of a CSC pattern: a color below `n` for every column,
    /// and no two columns of a color share a row. The error names the first conflict.
    ///
    /// # Panics
    ///
    /// For a pattern which is not a CSC pattern ([`SparsityPattern::is_csc_of`]).
    pub fn check_colors(&self, colors: &[u32]) -> Result<(), String> {
        let n = self.n as usize;
        if colors.len() != n {
            return Err(format!("{} colors for {n} columns", colors.len()));
        }
        if let Some(j) = colors.iter().position(|c| *c as usize >= n.max(1)) {
            return Err(format!(
                "column {j} has the color {}, at least the number of columns {n}",
                colors[j]
            ));
        }
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&j| colors[j]);
        // the color of the last column which has an entry in the row, by row
        let mut owner: Vec<Option<(u32, usize)>> = vec![None; n];
        for j in order {
            let c = colors[j];
            for &i in self.rows(j) {
                if let Some((owner_color, other)) = owner[i as usize]
                    && owner_color == c
                {
                    return Err(format!(
                        "the columns {other} and {j} of the color {c} share the row {i}"
                    ));
                }
                owner[i as usize] = Some((c, j));
            }
        }
        Ok(())
    }
}

/// How the Jacobian is provided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JacobianKind {
    /// `model_jac` computes it on the pattern, in its CSC order. A call whose entries are not
    /// all finite (an infinite derivative at a finite state, e.g. of `sqrt(x)` at `x = 0`) is
    /// computed by the engine with the sparse difference quotient over the greedy coloring of
    /// the pattern ([`SparsityPattern::greedy_colors`]) instead.
    Analytic(SparsityPattern),
    /// Difference quotients over the colors of the columns of the pattern; `model_jac` traps.
    ColoredDifference {
        /// The pattern.
        pattern: SparsityPattern,
        /// The color of every column ([`SparsityPattern::greedy_colors`]).
        colors: Vec<u32>,
        /// Why the model has no analytic Jacobian, naming the operator and the element.
        reason: String,
    },
}

impl JacobianKind {
    /// The pattern.
    pub fn pattern(&self) -> &SparsityPattern {
        match self {
            JacobianKind::Analytic(pattern) => pattern,
            JacobianKind::ColoredDifference { pattern, .. } => pattern,
        }
    }
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
        let message = check_contract("model/1", MODEL_CONTRACT)
            .unwrap_err()
            .to_string();
        assert!(message.contains("model/1") && message.contains("model/2"));
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
            compiler: "sbmlsim-compiler 0.1.0".to_string(),
            id: "m".to_string(),
            states: vec![Entity {
                id: "S".to_string(),
                kind: EntityKind::Species {
                    compartment: "c".to_string(),
                    has_only_substance_units: false,
                    stores_concentration: false,
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
            n_discontinuity_roots: 1,
            n_constants: 2,
            units: BTreeMap::new(),
        };
        let sizes = ModelSizes::of(&d);
        assert_eq!(sizes.words(), [1, 0, 1, 1, 3, 2, 1]);
        assert_eq!(sizes.assignments, [2]);
        assert_eq!(
            ModelSizes::from_words(sizes.words(), sizes.assignments.clone()),
            sizes
        );
        let back: ModelDescription =
            serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(d, back);
    }
}
