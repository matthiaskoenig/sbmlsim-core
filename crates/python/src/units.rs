//! The units of pint in Rust (`sbmlsim-units`): a registry which parses unit expressions,
//! gives their factors and dimensions and defines units; `sbmlsim_core.units` names it.

use std::collections::BTreeMap;

use numpy::PyArray1;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use sbmlsim_units::Registry;

use crate::errors::UnitError;

/// The exception of an error of the units, which names the expressions.
fn unit_error(error: sbmlsim_units::UnitError) -> PyErr {
    UnitError::new_err(error.to_string())
}

/// A registry of units: the builtin units of pint which sbmlsim uses, the units a caller
/// defines and the unit ids of a model.
#[pyclass(module = "sbmlsim_core", name = "UnitRegistry")]
pub(crate) struct UnitRegistry {
    registry: Registry,
}

#[pymethods]
impl UnitRegistry {
    /// The registry of the builtin units.
    #[new]
    fn new() -> UnitRegistry {
        UnitRegistry {
            registry: Registry::new(),
        }
    }

    /// The factor of a unit expression to the root units of pint and its dimensions, by the
    /// names of pint (`"[length]"`) with their exponents as floats.
    fn parse<'py>(&self, py: Python<'py>, expression: &str) -> PyResult<(f64, Bound<'py, PyDict>)> {
        let unit = self.registry.parse(expression).map_err(unit_error)?;
        let dimensions = PyDict::new(py);
        for (name, exponent) in unit.dimensions() {
            dimensions.set_item(format!("[{name}]"), f64::from(exponent))?;
        }
        Ok((unit.factor, dimensions))
    }

    /// The factor which converts a value in the unit `from_` into the unit `to`.
    #[pyo3(signature = (from_, to))]
    fn factor(&self, from_: &str, to: &str) -> PyResult<f64> {
        self.registry.factor(from_, to).map_err(unit_error)
    }

    /// Values converted from the unit `from_` into the unit `to`, each multiplied by the
    /// factor, as an array of float64.
    #[pyo3(signature = (values, from_, to))]
    fn convert<'py>(
        &self,
        py: Python<'py>,
        values: Vec<f64>,
        from_: &str,
        to: &str,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let mut values = values;
        self.registry
            .convert(&mut values, from_, to)
            .map_err(unit_error)?;
        Ok(PyArray1::from_vec(py, values))
    }

    /// Defines a unit by an expression of the units of the registry.
    fn define(&mut self, name: &str, expression: &str) -> PyResult<()> {
        self.registry.define(name, expression).map_err(unit_error)
    }

    /// A copy of the registry with the units of a model: the unit expressions of its entities
    /// and its unit ids with their expressions.
    #[pyo3(signature = (units = None, unit_ids = None))]
    fn with_model_units(
        &self,
        units: Option<BTreeMap<String, String>>,
        unit_ids: Option<BTreeMap<String, String>>,
    ) -> UnitRegistry {
        UnitRegistry {
            registry: self
                .registry
                .with_model_units(&units.unwrap_or_default(), &unit_ids.unwrap_or_default()),
        }
    }

    /// The unit expression of an entity of the model, none for an entity without one.
    fn unit_of(&self, entity: &str) -> Option<String> {
        self.registry.unit_of(entity).map(str::to_owned)
    }
}
