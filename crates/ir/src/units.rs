//! The unit expressions of the entities and selections, text a unit registry such as pint
//! parses; nothing is computed with them.

use std::collections::HashMap;

use sbmlsim_sbml::{Compartment, Model, Species};

/// The unit expressions of a model.
pub(crate) struct Units<'m> {
    model: &'m Model,
    definitions: HashMap<&'m str, &'m str>,
}

impl<'m> Units<'m> {
    pub fn new(model: &'m Model) -> Units<'m> {
        let definitions = model
            .unit_definitions
            .iter()
            .map(|u| (u.id.as_str(), u.expression.as_str()))
            .collect();
        Units { model, definitions }
    }

    /// The expression of a unit reference: a unit definition, else the unit kind itself.
    pub fn resolve(&self, unit: &str) -> String {
        self.definitions
            .get(unit)
            .map_or_else(|| unit.to_string(), |e| (*e).to_string())
    }

    fn of(&self, own: Option<&String>, default: Option<&String>) -> Option<String> {
        own.or(default).map(|u| self.resolve(u))
    }

    /// The unit of the size of a compartment: its own, else the default of the model for its
    /// spatial dimensions.
    pub fn compartment(&self, c: &Compartment) -> Option<String> {
        let m = self.model;
        let default = match c.spatial_dimensions {
            Some(3.0) => m.volume_units.as_ref(),
            Some(2.0) => m.area_units.as_ref(),
            Some(1.0) => m.length_units.as_ref(),
            _ => None,
        };
        self.of(c.units.as_ref(), default)
    }

    /// The unit of the amount of a species.
    pub fn substance(&self, s: &Species) -> Option<String> {
        self.of(
            s.substance_units.as_ref(),
            self.model.substance_units.as_ref(),
        )
    }

    /// The unit of the concentration of a species in its compartment.
    pub fn concentration(&self, s: &Species, c: Option<&Compartment>) -> Option<String> {
        Some(quotient(&self.substance(s)?, &self.compartment(c?)?))
    }

    /// The unit of a parameter.
    pub fn parameter(&self, units: Option<&String>) -> Option<String> {
        self.of(units, None)
    }

    /// The unit of the rate of a reaction, extent per time.
    pub fn reaction(&self) -> Option<String> {
        Some(quotient(
            &self.resolve(self.model.extent_units.as_ref()?),
            &self.time()?,
        ))
    }

    /// The unit of the time of the model.
    pub fn time(&self) -> Option<String> {
        Some(self.resolve(self.model.time_units.as_ref()?))
    }
}

/// `a/b`, a part which is more than a name in parentheses.
pub(crate) fn quotient(a: &str, b: &str) -> String {
    let part = |u: &str| {
        if u.chars().all(|c| c.is_alphanumeric() || c == '_') {
            u.to_string()
        } else {
            format!("({u})")
        }
    };
    format!("{}/{}", part(a), part(b))
}

#[cfg(test)]
mod tests {
    use super::quotient;

    #[test]
    fn quotients() {
        assert_eq!(quotient("mole", "litre"), "mole/litre");
        assert_eq!(
            quotient("0.001*mole", "litre**-1"),
            "(0.001*mole)/(litre**-1)"
        );
    }
}
