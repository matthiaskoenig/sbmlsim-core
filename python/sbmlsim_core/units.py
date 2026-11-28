"""The units of pint in Rust (since 0.45.0).

The crate `sbmlsim-units` parses the unit expressions sbmlsim uses and gives their factors to
the root units of pint and their dimensions, the numbers of `sbmlsim.units.ureg`: the factor
of every expression of the corpus of sbmlsim is pint's to the last bit. A conversion is the
factor of the quotient of the two units, as pint converts:

```python
from sbmlsim_core import units

registry = units.Registry()
registry.factor("mg/min", "g/hr")  # 0.06
registry.parse("mg/l")  # (0.9999999999999999, {"[length]": -3.0, "[mass]": 1.0})
registry.convert([1.0, 2.0], "hr", "min")  # array([ 60., 120.])
registry.define("IU_insulin", "0.0347 * mg")
model = registry.with_model_units(unit_ids={"mg_per_min": "mg/min"})
model.factor("mg_per_min/kg", "mg/min/kg")  # 1.0
```

Two units of other dimensions, a unit with an offset (`degC`), a name the registry does not
know and a unit defined again with another value raise `UnitError`. See the page Units of the
documentation.
"""

from sbmlsim_core._core import UnitError
from sbmlsim_core._core import UnitRegistry as Registry

__all__ = ["Registry", "UnitError"]
