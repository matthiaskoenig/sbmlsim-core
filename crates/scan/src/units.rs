//! The times and the values of a scan in the units of the model, sbmlsim's `_Converter`,
//! `_in_model_unit` and `target_values`.
//!
//! A conversion is the value times the factor of the pair of units ([`Registry::factor`],
//! pint's factor bitwise), which is what sbmlsim's path through pint computes: a quantity
//! `value * factor(unit, model unit)` and a number of a simulation with a time unit
//! `value * factor(time unit, model time unit)`.

use sbmlsim_units::Registry;

use crate::error::ScanError;
use crate::model::ModelInfo;
use crate::request::{Time, Values};
use crate::text;

/// Converts the times and the values of one simulation into the units of a model.
pub(crate) struct Converter<'m> {
    model: &'m ModelInfo,
    /// The factor of the time unit of the simulation into the one of the model, 1 without a
    /// time unit.
    factor: f64,
}

impl<'m> Converter<'m> {
    /// The converter of a simulation of a time unit.
    ///
    /// # Errors
    ///
    /// A time unit which is no time of the model.
    pub fn new(model: &'m ModelInfo, time_unit: Option<&str>) -> Result<Converter<'m>, ScanError> {
        let factor = match time_unit {
            None => 1.0,
            Some(unit) => model
                .registry()
                .factor(unit, model.time_unit())
                .map_err(|err| {
                    ScanError::units(format!(
                        "The time unit '{unit}' of the simulation cannot be converted into the \
                         time unit '{}' of the model: {err}",
                        model.time_unit()
                    ))
                })?,
        };
        Ok(Converter { model, factor })
    }

    /// A time in the time unit of the model.
    ///
    /// # Errors
    ///
    /// A quantity which is no time of the model.
    pub fn time(&self, time: &Time) -> Result<f64, ScanError> {
        match time {
            Time::Number(x) => Ok(x * self.factor),
            Time::Quantity { value, unit } => {
                let factor = self
                    .registry()
                    .factor(unit, self.model.time_unit())
                    .map_err(|err| {
                        ScanError::units(format!(
                            "The time '{time}' cannot be converted into the time unit '{}' of the \
                         model: {err}",
                            self.model.time_unit()
                        ))
                    })?;
                Ok(value * factor)
            }
        }
    }

    fn registry(&self) -> &Registry {
        self.model.registry()
    }
}

/// The factor of a quantity into the unit of its target in the model.
///
/// # Errors
///
/// A target without a unit in the model, or a unit which cannot be converted into it.
pub(crate) fn model_factor(
    model: &ModelInfo,
    target: &str,
    unit: &str,
    shown: &dyn Fn() -> String,
) -> Result<f64, ScanError> {
    let Some(model_unit) = model.unit(target) else {
        return Err(ScanError::units(format!(
            "'{target}' has no unit in the model, its value '{}' cannot be converted: give a \
             number in the unit the model means.",
            shown()
        )));
    };
    let model_unit = if model_unit.is_empty() {
        "dimensionless"
    } else {
        model_unit
    };
    model.registry().factor(unit, model_unit).map_err(|err| {
        ScanError::units(format!(
            "The value '{}' of '{target}' cannot be converted into the unit '{model_unit}' of \
             the model: {err}",
            shown()
        ))
    })
}

/// A quantity in the unit of its target in the model.
pub(crate) fn in_model_unit(
    model: &ModelInfo,
    target: &str,
    value: f64,
    unit: &str,
) -> Result<f64, ScanError> {
    let factor = model_factor(model, target, unit, &|| {
        format!("{} {unit}", text::number(value))
    })?;
    Ok(value * factor)
}

/// The values of a dimension of a target as numbers in the unit of the target in the model.
///
/// # Errors
///
/// A target which is no target of the model, or a quantity which cannot be converted.
pub(crate) fn target_values(
    model: &ModelInfo,
    target: &str,
    values: &Values,
) -> Result<Vec<f64>, ScanError> {
    model.kind(target)?;
    match values {
        Values::Numbers(xs) => Ok(xs.clone()),
        Values::Quantity { value, unit } => {
            let factor = model_factor(model, target, unit, &|| {
                let numbers: Vec<String> = value.iter().map(|x| text::number(*x)).collect();
                format!("[{}] {unit}", numbers.join(" "))
            })?;
            Ok(value.iter().map(|x| x * factor).collect())
        }
    }
}
