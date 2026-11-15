//! The functions of a compiled model, [`ModelFns`], and the test models.
//!
//! A compiled model is a WebAssembly module with the exports of the contract `model/1` (see
//! `sbmlsim_abi`); the host of the engine implements [`ModelFns`] by calling them. `p` is the
//! vector of the parameters, `y` the vector of the states and `t` the time, in the orders of the
//! [`ModelDescription`](sbmlsim_abi::ModelDescription) of the model.

use sbmlsim_abi::ModelSizes;

/// The functions of a compiled model, one per export of the contract `model/1`.
pub trait ModelFns {
    /// Writes the SBML values of the parameters and states.
    fn defaults(&self, p: &mut [f64], y: &mut [f64]);
    /// Evaluates the initial assignments and the initial values of the assignment rules in their
    /// order of dependency; `fixed` has a byte per entity (the parameters, then the states), a
    /// fixed entity (not 0) keeps its value.
    fn init(&self, t: f64, p: &mut [f64], y: &mut [f64], fixed: &[u8]);
    /// The right hand side of the ODEs.
    fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]);
    /// The values of the Jacobian, in the CSC order of the pattern of the description.
    fn jac(&self, t: f64, y: &[f64], p: &[f64], values: &mut [f64]);
    /// The root functions of the triggers of the events.
    fn roots(&self, t: f64, y: &[f64], p: &[f64], g: &mut [f64]);
    /// The trigger of event `i`.
    fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool;
    /// The priority of event `i`.
    fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64;
    /// The delay of event `i`.
    fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64;
    /// Evaluates the assignments of event `i` into `out`.
    fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]);
    /// Applies the evaluated assignments of event `i` at the time `t`.
    fn event_apply(&self, i: u32, t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]);
    /// Writes every selection of the description, in its order.
    fn selections(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]);
    /// Writes the current values of the targets of the assignment rules into `p` and `y`.
    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]);
    /// The sizes of the model (`model_sizes`), which the engine compares with the description.
    fn sizes(&self) -> ModelSizes;
}

#[cfg(any(test, feature = "test-models"))]
pub use test_models::{Decay, Derived, EventModel, Fun, NanRhs, Robertson, Target, TestEvent};

#[cfg(any(test, feature = "test-models"))]
mod test_models {
    use std::collections::BTreeMap;

    use sbmlsim_abi::{
        Entity, EntityKind, JacobianKind, MODEL_CONTRACT, ModelDescription, Selection,
        SparsityPattern,
    };

    use super::ModelFns;

    /// An entity of a test model (a parameter, or a state of a rate rule).
    fn entity(id: &str, initial: f64, settable: bool) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Parameter,
            initial,
            settable,
        }
    }

    /// The selections of the given ids, in their order.
    fn selections(ids: &[&str]) -> Vec<Selection> {
        ids.iter()
            .enumerate()
            .map(|(index, id)| Selection {
                id: id.to_string(),
                index: index as u32,
            })
            .collect()
    }

    /// A description of the contract `model/1` without events and units.
    fn description(
        id: &str,
        states: Vec<Entity>,
        parameters: Vec<Entity>,
        selections: Vec<Selection>,
        pattern: SparsityPattern,
    ) -> ModelDescription {
        ModelDescription {
            contract: MODEL_CONTRACT.to_string(),
            id: id.to_string(),
            states,
            parameters,
            selections,
            jacobian: JacobianKind::Analytic(pattern),
            events: vec![],
            n_roots: 0,
            units: BTreeMap::new(),
        }
    }

    /// The functions of a model without events; the sizes are the ones of its description.
    macro_rules! no_events {
        () => {
            fn sizes(&self) -> sbmlsim_abi::ModelSizes {
                sbmlsim_abi::ModelSizes::of(&self.description())
            }
            fn roots(&self, _t: f64, _y: &[f64], _p: &[f64], _g: &mut [f64]) {}
            fn trigger(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> bool {
                false
            }
            fn priority(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> f64 {
                0.0
            }
            fn delay(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> f64 {
                0.0
            }
            fn event_values(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64], _out: &mut [f64]) {}
            fn event_apply(
                &self,
                _i: u32,
                _t: f64,
                _values: &[f64],
                _y: &mut [f64],
                _p: &mut [f64],
            ) {
            }
        };
    }

    /// `y' = -k y`, `y(0) = 1`, with the assignment rule `t_half = ln(2) / k`.
    ///
    /// The parameters are `[k, t_half]` (`t_half` is not settable), the state is `[y]`, the
    /// selections are `[y, t_half]`.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Decay {
        /// The rate.
        pub k: f64,
    }

    impl Decay {
        /// The description of the model.
        pub fn description(&self) -> ModelDescription {
            description(
                "decay",
                vec![entity("y", 1.0, true)],
                vec![
                    entity("k", self.k, true),
                    entity("t_half", 2f64.ln() / self.k, false),
                ],
                selections(&["y", "t_half"]),
                SparsityPattern {
                    n: 1,
                    col_ptr: vec![0, 1],
                    row_idx: vec![0],
                },
            )
        }
    }

    impl ModelFns for Decay {
        fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
            p[0] = self.k;
            p[1] = 2f64.ln() / self.k;
            y[0] = 1.0;
        }
        fn init(&self, _t: f64, p: &mut [f64], _y: &mut [f64], fixed: &[u8]) {
            if fixed[1] == 0 {
                p[1] = 2f64.ln() / p[0];
            }
        }
        fn rhs(&self, _t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
            ydot[0] = -p[0] * y[0];
        }
        fn jac(&self, _t: f64, _y: &[f64], p: &[f64], values: &mut [f64]) {
            values[0] = -p[0];
        }
        fn selections(&self, _t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
            out[0] = y[0];
            out[1] = p[1];
        }
        fn assign(&self, _t: f64, _y: &mut [f64], p: &mut [f64]) {
            p[1] = 2f64.ln() / p[0];
        }
        no_events!();
    }

    /// The Robertson problem: `y1' = -k1 y1 + k3 y2 y3`, `y3' = k2 y2^2`, `y2' = -y1' - y3'`,
    /// `y(0) = (1, 0, 0)`, `k = (0.04, 3e7, 1e4)`, as the SUNDIALS example `cvRoberts_dns`.
    ///
    /// Its Jacobian has 7 entries, `J31` and `J33` are structurally zero, so the pattern lacks a
    /// diagonal entry and CVODE grows the sparse matrix when it adds the identity.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Robertson;

    impl Robertson {
        /// The description of the model.
        pub fn description(&self) -> ModelDescription {
            description(
                "robertson",
                vec![
                    entity("y1", 1.0, true),
                    entity("y2", 0.0, true),
                    entity("y3", 0.0, true),
                ],
                vec![
                    entity("k1", 0.04, true),
                    entity("k2", 3.0e7, true),
                    entity("k3", 1.0e4, true),
                ],
                selections(&["y1", "y2", "y3"]),
                SparsityPattern {
                    n: 3,
                    col_ptr: vec![0, 2, 5, 7],
                    row_idx: vec![0, 1, 0, 1, 2, 0, 1],
                },
            )
        }
    }

    impl ModelFns for Robertson {
        fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
            p.copy_from_slice(&[0.04, 3.0e7, 1.0e4]);
            y.copy_from_slice(&[1.0, 0.0, 0.0]);
        }
        fn init(&self, _t: f64, _p: &mut [f64], _y: &mut [f64], _fixed: &[u8]) {}
        fn rhs(&self, _t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
            let yd1 = -p[0] * y[0] + p[2] * y[1] * y[2];
            let yd3 = p[1] * y[1] * y[1];
            ydot[0] = yd1;
            ydot[1] = -yd1 - yd3;
            ydot[2] = yd3;
        }
        fn jac(&self, _t: f64, y: &[f64], p: &[f64], values: &mut [f64]) {
            values.copy_from_slice(&[
                -p[0],
                p[0],
                p[2] * y[2],
                -p[2] * y[2] - 2.0 * p[1] * y[1],
                2.0 * p[1] * y[1],
                p[2] * y[1],
                -p[2] * y[1],
            ]);
        }
        fn selections(&self, _t: f64, y: &[f64], _p: &[f64], out: &mut [f64]) {
            out.copy_from_slice(y);
        }
        fn assign(&self, _t: f64, _y: &mut [f64], _p: &mut [f64]) {}
        no_events!();
    }

    /// `y' = -k y` for `t <= 1` and a right hand side of NaN after, so CVODE cannot go on.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct NanRhs;

    impl NanRhs {
        /// The description of the model.
        pub fn description(&self) -> ModelDescription {
            description(
                "nanrhs",
                vec![entity("y", 1.0, true)],
                vec![entity("k", 1.0, true)],
                selections(&["y"]),
                SparsityPattern {
                    n: 1,
                    col_ptr: vec![0, 1],
                    row_idx: vec![0],
                },
            )
        }
    }

    impl ModelFns for NanRhs {
        fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
            p[0] = 1.0;
            y[0] = 1.0;
        }
        fn init(&self, _t: f64, _p: &mut [f64], _y: &mut [f64], _fixed: &[u8]) {}
        fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
            ydot[0] = if t > 1.0 { f64::NAN } else { -p[0] * y[0] };
        }
        fn jac(&self, t: f64, _y: &[f64], p: &[f64], values: &mut [f64]) {
            values[0] = if t > 1.0 { f64::NAN } else { -p[0] };
        }
        fn selections(&self, _t: f64, y: &[f64], _p: &[f64], out: &mut [f64]) {
            out[0] = y[0];
        }
        fn assign(&self, _t: f64, _y: &mut [f64], _p: &mut [f64]) {}
        no_events!();
    }

    /// `y' = -a y`, `y(0) = 1`, with the initial assignment `a = 2 k` of the settable parameter
    /// `a`, which `init` writes unless `a` is fixed (set by the pre-initialization).
    ///
    /// The parameters are `[k, a]`, the state is `[y]`, the selections are `[y, a]`.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Derived {
        /// The parameter `k`.
        pub k: f64,
    }

    impl Derived {
        /// The description of the model.
        pub fn description(&self) -> ModelDescription {
            description(
                "derived",
                vec![entity("y", 1.0, true)],
                vec![entity("k", self.k, true), entity("a", 2.0 * self.k, true)],
                selections(&["y", "a"]),
                SparsityPattern {
                    n: 1,
                    col_ptr: vec![0, 1],
                    row_idx: vec![0],
                },
            )
        }
    }

    impl ModelFns for Derived {
        fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
            p[0] = self.k;
            p[1] = 2.0 * self.k;
            y[0] = 1.0;
        }
        fn init(&self, _t: f64, p: &mut [f64], _y: &mut [f64], fixed: &[u8]) {
            if fixed[1] == 0 {
                p[1] = 2.0 * p[0];
            }
        }
        fn rhs(&self, _t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
            ydot[0] = -p[1] * y[0];
        }
        fn jac(&self, _t: f64, _y: &[f64], p: &[f64], values: &mut [f64]) {
            values[0] = -p[1];
        }
        fn selections(&self, _t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
            out[0] = y[0];
            out[1] = p[1];
        }
        fn assign(&self, _t: f64, _y: &mut [f64], _p: &mut [f64]) {}
        no_events!();
    }

    /// A function of the time, the states and the parameters of an [`EventModel`].
    pub type Fun = fn(f64, &[f64], &[f64]) -> f64;

    /// The target of an assignment of a [`TestEvent`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Target {
        /// The state of this index.
        State(usize),
        /// The parameter of this index.
        Param(usize),
    }

    /// An event of an [`EventModel`], its trigger, root functions, delay, priority and
    /// assignments as plain functions.
    #[derive(Debug, Clone)]
    pub struct TestEvent {
        /// The id.
        pub id: &'static str,
        /// The trigger.
        pub trigger: fn(f64, &[f64], &[f64]) -> bool,
        /// The root functions of the trigger.
        pub roots: Vec<Fun>,
        /// The delay.
        pub delay: Option<Fun>,
        /// The priority.
        pub priority: Option<Fun>,
        /// Whether the trigger is persistent.
        pub persistent: bool,
        /// The value of the trigger before the start.
        pub initial_value: bool,
        /// Whether the values are the ones at the time of the trigger.
        pub use_values_from_trigger_time: bool,
        /// The assignments.
        pub assignments: Vec<(Target, Fun)>,
    }

    impl TestEvent {
        /// An event of the trigger and its root functions, persistent, with the initial value
        /// true, the values at the trigger, no delay, no priority and no assignments.
        pub fn new(
            id: &'static str,
            trigger: fn(f64, &[f64], &[f64]) -> bool,
            roots: Vec<Fun>,
        ) -> TestEvent {
            TestEvent {
                id,
                trigger,
                roots,
                delay: None,
                priority: None,
                persistent: true,
                initial_value: true,
                use_values_from_trigger_time: true,
                assignments: Vec::new(),
            }
        }
    }

    /// A model of states, parameters, a right hand side and events, all given as functions.
    ///
    /// The selections are the states, then the parameters, by their ids; the Jacobian is the
    /// colored difference quotient over the dense pattern (the engine uses CVODE's).
    #[derive(Debug, Clone)]
    pub struct EventModel {
        /// The states, `(id, initial value)`.
        pub states: Vec<(&'static str, f64)>,
        /// The parameters, `(id, value)`.
        pub parameters: Vec<(&'static str, f64)>,
        /// The right hand side.
        pub rhs: fn(f64, &[f64], &[f64], &mut [f64]),
        /// The events.
        pub events: Vec<TestEvent>,
    }

    impl EventModel {
        /// The description of the model.
        pub fn description(&self) -> ModelDescription {
            let n = self.states.len() as u32;
            let ids: Vec<&str> = self
                .states
                .iter()
                .chain(&self.parameters)
                .map(|(id, _)| *id)
                .collect();
            ModelDescription {
                contract: MODEL_CONTRACT.to_string(),
                id: "events".to_string(),
                states: self
                    .states
                    .iter()
                    .map(|(id, x)| entity(id, *x, true))
                    .collect(),
                parameters: self
                    .parameters
                    .iter()
                    .map(|(id, x)| entity(id, *x, true))
                    .collect(),
                selections: selections(&ids),
                jacobian: JacobianKind::ColoredDifference {
                    pattern: SparsityPattern {
                        n,
                        col_ptr: (0..=n).map(|j| j * n).collect(),
                        row_idx: (0..n).flat_map(|_| 0..n).collect(),
                    },
                    colors: (0..n).collect(),
                },
                events: self
                    .events
                    .iter()
                    .map(|e| sbmlsim_abi::EventInfo {
                        id: e.id.to_string(),
                        has_delay: e.delay.is_some(),
                        has_priority: e.priority.is_some(),
                        persistent: e.persistent,
                        initial_value: e.initial_value,
                        use_values_from_trigger_time: e.use_values_from_trigger_time,
                        n_assignments: e.assignments.len() as u32,
                    })
                    .collect(),
                n_roots: self.events.iter().map(|e| e.roots.len() as u32).sum(),
                units: BTreeMap::new(),
            }
        }
    }

    impl ModelFns for EventModel {
        fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
            for (v, (_, x)) in p.iter_mut().zip(&self.parameters) {
                *v = *x;
            }
            for (v, (_, x)) in y.iter_mut().zip(&self.states) {
                *v = *x;
            }
        }
        fn init(&self, _t: f64, _p: &mut [f64], _y: &mut [f64], _fixed: &[u8]) {}
        fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
            (self.rhs)(t, y, p, ydot)
        }
        fn jac(&self, _t: f64, _y: &[f64], _p: &[f64], _values: &mut [f64]) {
            unreachable!("the Jacobian of an event model is a difference quotient")
        }
        fn roots(&self, t: f64, y: &[f64], p: &[f64], g: &mut [f64]) {
            let all = self.events.iter().flat_map(|e| &e.roots);
            for (g, root) in g.iter_mut().zip(all) {
                *g = root(t, y, p);
            }
        }
        fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
            (self.events[i as usize].trigger)(t, y, p)
        }
        fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
            self.events[i as usize].priority.map_or(0.0, |f| f(t, y, p))
        }
        fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
            self.events[i as usize].delay.map_or(0.0, |f| f(t, y, p))
        }
        fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
            for (o, (_, f)) in out.iter_mut().zip(&self.events[i as usize].assignments) {
                *o = f(t, y, p);
            }
        }
        fn event_apply(&self, i: u32, _t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]) {
            for (v, (target, _)) in values.iter().zip(&self.events[i as usize].assignments) {
                match target {
                    Target::State(k) => y[*k] = *v,
                    Target::Param(k) => p[*k] = *v,
                }
            }
        }
        fn selections(&self, _t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
            for (o, v) in out.iter_mut().zip(y.iter().chain(p)) {
                *o = *v;
            }
        }
        fn assign(&self, _t: f64, _y: &mut [f64], _p: &mut [f64]) {}
        fn sizes(&self) -> sbmlsim_abi::ModelSizes {
            sbmlsim_abi::ModelSizes::of(&self.description())
        }
    }
}
