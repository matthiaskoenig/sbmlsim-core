//! The result of a scan, the contract `scanresult/1`: the arrays of sbmlsim's `ScanResult`
//! (an `xarray.Dataset`) as columns, from which a host rebuilds the dataset with its names,
//! coordinates and attributes.
//!
//! The layout is the one of sbmlsim's `Simulator.run` (`_Compiled.assemble`):
//!
//! - the dimensions of the scan first, in their order, or the dimension `_index` of the flat
//!   indices of the points of a block ([`ScanResult::block`]);
//! - then the time: `time` on a grid ([`Layout::Grid`], a coordinate `time`) or `_point`, the
//!   output points of every point padded with `NaN` ([`Layout::Ragged`], a variable `time`);
//! - the variables in the order of the dataset: `time` of a ragged result, the kept
//!   timecourses, the kept values per simulation (no time dimension), and `status` of a scan
//!   of `on_error: flag` (`0` for a point which ran, `1` for one which failed);
//! - the coordinates in the order of the dataset: `_index` of a block, then per dimension its
//!   labels (under its id), the values of its targets (under the target, or
//!   `<dimension>.<target>` where a variable or a reserved name has the name of the target, a
//!   target an earlier dimension changes left out) and its covariates, then `time` of a grid.
//!
//! Every array is flat in C order with the shape of its dimensions; `NaN` and the infinities
//! are values of the arrays. As JSON a number of an array (of a variable, of the numbers of a
//! coordinate, of `changes`) is a number, `NaN` is `null` and the infinities are the strings
//! `"inf"` and `"-inf"`, as the oracle fixtures of sbmlsim write them (`"NaN"` is read as well),
//! so the serialized result reads back as the result, every value by its bits; a binding which
//! carries the arrays in another form carries `NaN` and the infinities as values.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The numbers of an array as JSON, see the module: a number, `null` for `NaN`, `"inf"` and
/// `"-inf"` for the infinities.
mod numbers {
    use serde::de::Error as _;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(values: &[f64], serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(values.len()))?;
        for x in values {
            if x.is_finite() {
                seq.serialize_element(x)?;
            } else if x.is_nan() {
                seq.serialize_element(&())?;
            } else if *x > 0.0 {
                seq.serialize_element("inf")?;
            } else {
                seq.serialize_element("-inf")?;
            }
        }
        seq.end()
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Finite(f64),
        Text(String),
        Null(()),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<f64>, D::Error> {
        Vec::<Number>::deserialize(deserializer)?
            .into_iter()
            .map(|n| match n {
                Number::Finite(x) => Ok(x),
                Number::Null(()) => Ok(f64::NAN),
                Number::Text(text) => match text.as_str() {
                    "NaN" => Ok(f64::NAN),
                    "inf" => Ok(f64::INFINITY),
                    "-inf" => Ok(f64::NEG_INFINITY),
                    other => Err(D::Error::custom(format!(
                        "{other:?} is no number (a number, null, \"inf\" or \"-inf\")"
                    ))),
                },
            })
            .collect()
    }
}

/// The contract of a result.
pub const RESULT_CONTRACT: &str = "scanresult/1";

/// The most failed points a result names in [`ScanResult::errors`], `MAX_ERRORS` of sbmlsim.
pub const MAX_ERRORS: usize = 10;

/// How the time of the result is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// One grid of times for every point, the coordinate `time`.
    Grid,
    /// The output points of every point over `_point`, padded with `NaN`, the variable `time`.
    Ragged,
}

/// A dimension of the arrays and its size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    /// The name of the dimension.
    pub name: String,
    /// Its size.
    pub size: usize,
}

/// The values of a coordinate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordValues {
    /// Numbers: the values of a target or a covariate as the request gives them, the times of
    /// a grid.
    Numbers(#[serde(with = "numbers")] Vec<f64>),
    /// The labels of a dimension as the request gives them (numbers or texts), which a host
    /// turns into an array as it turns the labels of a dimension into one.
    Labels(Vec<Json>),
    /// The flat indices of the points of a block.
    Indices(Vec<usize>),
}

/// A coordinate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coord {
    /// Its name.
    pub name: String,
    /// The dimensions it is over.
    pub dims: Vec<String>,
    /// Its unit, the entry of `units` of the result; `None` for `_index`, which has none.
    pub unit: Option<String>,
    /// Its values.
    pub values: CoordValues,
}

/// A variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    /// Its name.
    pub name: String,
    /// The dimensions it is over.
    pub dims: Vec<String>,
    /// The size of every dimension.
    pub shape: Vec<usize>,
    /// Its unit, `""` without one.
    pub unit: String,
    /// The values, flat in C order.
    #[serde(with = "numbers")]
    pub values: Vec<f64>,
}

/// A point which failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailedPoint {
    /// The flat index of the point in the scan.
    pub index: usize,
    /// The labels and the values of the point (`dose=1, PODOSE=10.0 mg`), empty for a scan
    /// without dimensions.
    pub point: String,
    /// The failure: the type of the error and its message.
    pub message: String,
}

impl FailedPoint {
    /// The text sbmlsim records in `attrs["errors"]`: `"{point}: {message}"`, the message
    /// alone without dimensions.
    pub fn text(&self) -> String {
        if self.point.is_empty() {
            self.message.clone()
        } else {
            format!("{}: {}", self.point, self.message)
        }
    }
}

/// The points of a block of a scan, sbmlsim's attribute `block`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    /// The first flat index.
    pub start: usize,
    /// The flat index after the last.
    pub stop: usize,
    /// The shape of the scan.
    pub shape: Vec<usize>,
    /// The dimensions of the scan.
    pub dims: Vec<String>,
    /// The dimension of the scan of every coordinate along `_index`, in their order.
    pub coords: Vec<(String, String)>,
}

/// The result of a scan, see the module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanResult {
    /// The contract, [`RESULT_CONTRACT`].
    pub contract: String,
    /// The dimensions of the arrays with their sizes, in the order of the dataset.
    pub sizes: Vec<Size>,
    /// The dimensions of the scan, sbmlsim's attribute `dims` (also of a block).
    pub dims: Vec<String>,
    /// How the time is laid out.
    pub layout: Layout,
    /// The coordinates, in the order of the dataset.
    pub coords: Vec<Coord>,
    /// The variables, in the order of the dataset.
    pub variables: Vec<Variable>,
    /// The unit of every variable and coordinate, sbmlsim's attribute `units` in its order.
    pub units: Vec<(String, String)>,
    /// The times of the changes after the start in the time of the result, sorted.
    #[serde(with = "numbers")]
    pub changes: Vec<f64>,
    /// The first [`MAX_ERRORS`] points which failed, in the order of the scan; `None` for a scan
    /// of `on_error: raise`, which has no attribute `errors`.
    pub errors: Option<Vec<FailedPoint>>,
    /// The block of the points, `None` for a run of the whole scan.
    pub block: Option<Block>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bits of the numbers of a result, `NaN` as one value.
    fn bits(result: &ScanResult) -> Vec<u64> {
        let canonical = |x: &f64| {
            if x.is_nan() {
                f64::NAN.to_bits()
            } else {
                x.to_bits()
            }
        };
        let mut out: Vec<u64> = result.changes.iter().map(canonical).collect();
        for v in &result.variables {
            out.extend(v.values.iter().map(canonical));
        }
        for c in &result.coords {
            if let CoordValues::Numbers(xs) = &c.values {
                out.extend(xs.iter().map(canonical));
            }
        }
        out
    }

    #[test]
    fn a_result_with_nan_and_the_infinities_reads_back_as_itself() {
        let result = ScanResult {
            contract: RESULT_CONTRACT.to_string(),
            sizes: vec![Size {
                name: "time".to_string(),
                size: 4,
            }],
            dims: Vec::new(),
            layout: Layout::Grid,
            coords: vec![Coord {
                name: "time".to_string(),
                dims: vec!["time".to_string()],
                unit: Some("second".to_string()),
                values: CoordValues::Numbers(vec![0.0, -0.0, 5.0, f64::INFINITY]),
            }],
            variables: vec![Variable {
                name: "S".to_string(),
                dims: vec!["time".to_string()],
                shape: vec![4],
                unit: String::new(),
                values: vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.1 + 0.2],
            }],
            units: vec![("time".to_string(), "second".to_string())],
            changes: vec![f64::NEG_INFINITY, 5e-324],
            errors: None,
            block: None,
        };
        let text = serde_json::to_string(&result).unwrap();
        assert!(
            text.contains(r#""values":[null,"inf","-inf",0.30000000000000004]"#),
            "{text}"
        );
        let back: ScanResult = serde_json::from_str(&text).unwrap();
        assert_eq!(bits(&back), bits(&result));
        assert_eq!(back.variables[0].name, "S");
        // the fixtures of sbmlsim, and "NaN"
        let read: Variable = serde_json::from_str(
            r#"{"name": "x", "dims": [], "shape": [], "unit": "", "values": [null, "NaN", "inf", 1]}"#,
        )
        .unwrap();
        assert!(read.values[0].is_nan() && read.values[1].is_nan());
        assert_eq!(read.values[2..], [f64::INFINITY, 1.0]);
        assert!(
            serde_json::from_str::<Variable>(
                r#"{"name": "x", "dims": [], "shape": [], "unit": "", "values": ["nan!"]}"#
            )
            .is_err()
        );
    }
}
