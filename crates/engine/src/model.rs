//! The functions of a compiled model, [`ModelFns`], and the test models.
//!
//! A compiled model is a WebAssembly module with the exports of the contract `model/1` (see
//! `sbmlsim_abi`); the host of the engine implements [`ModelFns`] by calling them. `p` is the
//! vector of the parameters, `y` the vector of the states and `t` the time, in the orders of the
//! [`ModelDescription`](sbmlsim_abi::ModelDescription) of the model.

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
    /// Applies the evaluated assignments of event `i`.
    fn event_apply(&self, i: u32, values: &[f64], y: &mut [f64], p: &mut [f64]);
    /// Writes every selection of the description, in its order.
    fn selections(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]);
    /// Writes the current values of the targets of the assignment rules into `p` and `y`.
    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]);
    /// The number of root functions.
    fn n_roots(&self) -> usize;
}

#[cfg(any(test, feature = "test-models"))]
pub use test_models::{Decay, Robertson};

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
            units: BTreeMap::new(),
        }
    }

    /// The functions of a model without events.
    macro_rules! no_events {
        () => {
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
            fn event_apply(&self, _i: u32, _values: &[f64], _y: &mut [f64], _p: &mut [f64]) {}
            fn n_roots(&self) -> usize {
                0
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
}
