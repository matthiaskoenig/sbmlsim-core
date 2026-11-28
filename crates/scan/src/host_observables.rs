//! The round of the observables of the host (sbmlsim's `Custom` observables): the native time
//! and columns of every point which they read, and the values the host computes of them.
//!
//! A host observable is a function of the host, `function(time, values)`, which sbmlsim calls
//! once per point with the finite time points of the simulation (the steady state after the
//! end, `inf`, is none) and the values of the symbols it reads at them: the timecourse of a
//! selection or of a timecourse observable, the number of a value per simulation. It answers
//! one number (`kind: scalar`) or a value per time point (`kind: timecourse`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What the host computes: the observables of the host of every point, from its time and the
/// columns they read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostObservables {
    /// The ids of the observables, in the order the run evaluates them.
    pub observables: Vec<String>,
    /// The points, in the order of the scan.
    pub points: Vec<HostPoint>,
    /// The symbols of the columns which are a value per simulation (a value per simulation of
    /// an observable or a parameter `<pk>.<parameter>` of a PK observable), sorted, so a host
    /// tells one value from a timecourse of one time point; the others are timecourses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scalars: Vec<String>,
}

/// The native output of one point for the observables of the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostPoint {
    /// The flat index of the point in the scan.
    pub index: usize,
    /// The finite time points of the simulation, in the time of the result.
    pub time: Vec<f64>,
    /// Every symbol the observables read to its values: a value per time point of a
    /// timecourse, one value of a value per simulation.
    pub columns: BTreeMap<String, Vec<f64>>,
}

/// What the host computed: per observable, the values of every point in the order of
/// [`HostObservables::points`] (one value of a scalar, a value per time point of a
/// timecourse), and the points whose function failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostObserved {
    /// Observable id to the values of every point; the values of a point of an error are
    /// ignored.
    pub values: BTreeMap<String, Vec<Vec<f64>>>,
    /// The functions which failed on a point; such a point fails.
    #[serde(default)]
    pub errors: Vec<HostError>,
}

/// A function of the host which failed on a point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostError {
    /// The id of the observable.
    pub observable: String,
    /// The flat index of the point in the scan.
    pub index: usize,
    /// The error: its type and its message, `ValueError: ...`.
    pub message: String,
}
