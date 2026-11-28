# Units

The units of sbmlsim are those of [pint](https://pint.readthedocs.io), its registry `sbmlsim.units.ureg`. Since 0.42.0 the same arithmetic runs here, in the crate `sbmlsim-units`: a registry which parses the unit expressions sbmlsim uses, gives their factors to the root units of pint and their dimensions, defines units and converts values. It is one authority for the conversions of the scan planner, of python (`sbmlsim_core.units`) and of a page (`js/units.mjs` on `compiler.wasm`), and its numbers are pint's: the factor of every expression of the corpus of sbmlsim (`testdata/units_corpus.json`, the units of its packaged and test models, of its documentation and tests and the forms of PK-DB) is the one of pint to the last bit, and the parity holds it to 1e-15 relative.

| | Rust | Python | JavaScript |
|---|---|---|---|
| registry | `Registry::new()` | `units.Registry()` | `await createUnits(compiler)` |
| factor and dimensions | `parse(expr)` | `parse(expr)` | `parse(expr)` |
| conversion factor | `factor(from, to)` | `factor(from_, to)` | `factor(from, to)` |
| values converted | `convert(&mut values, from, to)` | `convert(values, from_, to)` | `convert(values, from, to)` |
| a unit defined | `define(name, expr)` | `define(name, expr)` | `define(name, expr)` |
| the units of a model | `with_model_units(&units, &unit_ids)` | `with_model_units(units, unit_ids)` | `withModelUnits({ units, unitIds })` |
| the unit of an entity | `unit_of(id)` | `unit_of(id)` | `unitOf(id)` |
| error | `UnitError` | `UnitError` | `UnitError` |

## Expressions

An expression is what pint reads for the units sbmlsim uses:

- products, quotients and integer powers (`*`, `/`, `**` or `^`, ` per `), parentheses, and juxtaposition as a product (`kg m`), with the priorities of pint: `**` binds tightest and to the right, then a sign, then `*`, `/` and juxtaposition alike from the left, so `1/kg m` is `m/kg`;
- numbers, whose value is part of the factor: `ml/min/(1.73*m^2)` and `micromol/(0.5*hr)`, the forms of PK-DB;
- the prefixes of SI from `q` to `Q` in their short and long forms, `µ`, `μ` and `u` for micro;
- the names of pint, short and long, and their plurals: `mg`, `milligram`, `milligrams`, `hr`, `hour`, `h`, `min`, `minute`, `day`, `l`, `liter`, `litre`, `mol`, `mole`, `M` and `molar`, `kg`, `m`, `s`, `dimensionless`, the derived units of SI (`newton`, `pascal`, `joule`, `watt`, `volt`, `ohm`, `katal`, ...), `mmHg`, `Da`, `percent`;
- the units sbmlsim adds: `none` and `item` as counts and `percent` as `0.01 * count`.

A name is read as pint reads it: a name or alias first, so `min` is a minute and `cd` a candela, else the first prefix and name which make it up (`mmol`, `µg`), the plural `s` last (`ms` is a millisecond, `grams` a gram). The table holds the units of pint which sbmlsim uses; a name of pint which it has not is an error, never another unit under a prefix.

A power of units is an integer: pint reads `m**0.5`, which no unit of sbmlsim has, and the registry refuses it (a number may have any power, `4**0.5 * m`).

## Factors and dimensions

A unit is a factor to the root units of pint, `meter`, `gram`, `second`, `mole`, `ampere`, `kelvin` and `candela`, and the exponents of their dimensions, `[length]`, `[mass]`, `[time]`, `[substance]`, `[current]`, `[temperature]` and `[luminosity]`. Counts (`count`, `item`, `none`, `percent`) and angles have no dimension.

The factor is computed as pint computes it, down the definitions of the units as pint defines them (`hour = 60 * minute`, `minute = 60 * second`, `liter = decimeter ** 3`, `molar = mole / liter`), the scales of the units in the numerator and in the denominator counted apart, equal scales together, a scale on both sides cancelled, and the powers multiplied in pint's order. So `l` is `0.0010000000000000002` m³ as in pint, and the factor of a conversion, which is the factor of the quotient of the two units, is exactly `1000` for `l` to `ml` and exactly `0.06` for `mg/min` to `g/hr`, where the quotient of their two factors to the root units would round twice. The powers are the `pow` of `libm`, so a factor has the same bits natively and in wasm.

```rust
use sbmlsim_units::Registry;

let registry = Registry::new();
let unit = registry.parse("mg/l")?; // factor 0.9999999999999999, [mass] / [length] ** 3
assert_eq!(registry.factor("l", "ml")?, 1000.0);
let mut values = [1.0, 2.0];
registry.convert(&mut values, "hr", "min")?; // [60.0, 120.0]
```

```python
from sbmlsim_core import units

registry = units.Registry()
registry.parse("mg/l")  # (0.9999999999999999, {"[length]": -3.0, "[mass]": 1.0})
registry.factor("mg/min", "g/hr")  # 0.06
```

```js
import { createCompiler } from "sbmlsim-core/compile";
import { createUnits } from "sbmlsim-core/units";

const units = await createUnits(await createCompiler({ compiler: compilerWasm }));
units.parse("mg/l"); // { factor: 0.9999999999999999, dimensions: { "[length]": -3, "[mass]": 1 } }
units.factor("l", "ml"); // 1000
units.convert([1, 2], "hr", "min"); // Float64Array [60, 120]
units.close();
```

`createUnits` takes `compiler.wasm` built with the feature `units` (the default) as a compiler of `compile.mjs`, an instance, a module or its bytes. The python dimensions are keyed by the names of pint with float exponents, so they compare with `ureg.Quantity(1, expr).to_root_units().dimensionality` directly.

## Defining units

`define(name, expression)` defines a unit by an expression of the units of the registry, as `define` of pint, e.g. an international unit of a substance, which the registry does not define since it is specific to the substance:

```python
registry.define("IU_insulin", "0.0347 * mg")
registry.factor("IU_insulin", "ug")  # 34.7
registry.define("IU_insulin", "0.0347 * mg")  # the same value again changes nothing
registry.define("IU_insulin", "0.045 * mg")  # UnitError: defined as '0.0347 * mg'
```

A name which is a unit already, also under a prefix or as a plural (`mg`, `hours`), is refused unless the expression gives it the same factor and dimensions.

## The units of a model

`with_model_units(units, unit_ids)` is a copy of the registry with the units of a model: `units` the unit expressions of its entities by id (the `units` of its `model/2` description, which `unit_of(id)` gives), `unit_ids` its unit definitions, an id with its expression. A unit id names a unit in every expression the copy parses, replaced by its expression as a whole and only once, as `UnitsInformation.quantity` of sbmlsim does:

```python
model = registry.with_model_units(unit_ids={"mg_per_min": "mg/min"})
model.factor("mg_per_min/kg", "mg/min/kg")  # 1.0, mg_per_min/kg is (mg/min)/kg
```

In a page `withModelUnits({ units, unitIds })` gives the copy, a registry of its own in the module which its caller closes, and `unitOf(id)` the unit of an entity (`null` without one). The `unit_definitions` of a `model/2` description (the unit definitions its units name by their id because pint has no unit of them) are such unit ids.

A unit id wins over a unit of the registry of the same name, since the model defines what its id means, and an empty expression is dimensionless.

## Errors

Every error names the expressions it is about: an expression which does not parse, a name the registry does not know, a power which is no integer, a number of zero or one which is not finite, two units of other dimensions (`cannot convert 'mg' ([mass]) to 'ml' ([length] ** 3)`), a unit defined again with another value, and a unit with an offset. The temperatures with an offset (`degC`, `degF`) are known to the parser and refused, naming the unit (`degree_Celsius`), since no factor converts them; `kelvin` and `degR` convert. The error is `UnitError` in Rust, `sbmlsim_core.UnitError` (a `SbmlsimCoreError`) in python and `UnitError` of `units.mjs` in JavaScript.
