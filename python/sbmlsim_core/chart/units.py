"""The units of the data of a chart, without a dependency on pint or xarray.

The data of an element may be quantities and data arrays, which the chart reads
by duck typing, so neither pint nor xarray is a dependency:

- a quantity is an object with `.to(unit)`, `.magnitude` and `.units`, e.g. a
  quantity of pint: it is converted with its own registry, `quantity.to(unit)`
  with the unit as text, so the units of the registry it was made with (the
  units sbmlsim defines among them) are known;
- a data array is an object with `.values`, `.dims` and `.attrs`, e.g. a
  `xarray.DataArray`: `attrs["units"]` is its unit as text, which is converted
  with the application registry of pint (`pint.get_application_registry()`,
  which a package makes its own registry with `pint.set_application_registry`)
  when pint is installed; pint is imported only then.

`short_unit` formats the label of a unit itself: it reads the expressions of
pint (`nanogram / milliliter`, `ng/ml`, `mg/(l*h)`, `m**2`, `1/hour`) and
writes them in the short pretty form of pint (`f"{unit:~P}"`, `ng/ml`, `mg/h/l`,
`m²`, `1/h`), so a label is the same with and without pint.
"""

from __future__ import annotations

import importlib
import re
from collections.abc import Callable
from typing import Any, Final

#: the prefixes of pint: the name, the symbol and the aliases
_PREFIXES: Final = (
    ("quecto", "q"),
    ("ronto", "r"),
    ("yocto", "y"),
    ("zepto", "z"),
    ("atto", "a"),
    ("femto", "f"),
    ("pico", "p"),
    ("nano", "n"),
    ("micro", "μ", "µ", "u", "mu", "mc"),
    ("milli", "m"),
    ("centi", "c"),
    ("deci", "d"),
    ("deca", "da", "deka"),
    ("hecto", "h"),
    ("kilo", "k"),
    ("mega", "M"),
    ("giga", "G"),
    ("tera", "T"),
    ("peta", "P"),
    ("exa", "E"),
    ("zetta", "Z"),
    ("yotta", "Y"),
    ("ronna", "R"),
    ("quetta", "Q"),
)

#: the units: the name of pint, its symbol (`None` for a unit pint shows by its
#: name) and the aliases; `percent`, `none` and `item` are units of sbmlsim's
#: registry without a symbol
_UNITS: Final[tuple[tuple[str, str | None, tuple[str, ...]], ...]] = (
    ("gram", "g", ()),
    ("meter", "m", ("metre",)),
    ("second", "s", ("sec",)),
    ("liter", "l", ("L", "litre")),
    ("mole", "mol", ()),
    ("molar", "M", ()),
    ("kelvin", "K", ("degK",)),
    ("ampere", "A", ()),
    ("candela", "cd", ()),
    ("minute", "min", ()),
    ("hour", "h", ("hr",)),
    ("day", "d", ()),
    ("week", None, ()),
    ("month", None, ()),
    ("year", "a", ("yr", "julian_year")),
    ("dalton", "Da", ()),
    ("inch", "in", ()),
    ("foot", "ft", ()),
    ("pound", "lb", ()),
    ("millimeter_Hg", "mmHg", ("mm_Hg", "millimeter_Hg_0C")),
    ("degree_Celsius", "°C", ("degC", "celsius")),
    ("delta_degree_Celsius", "Δ°C", ("delta_degC",)),
    ("enzyme_unit", "U", ("enzymatic_unit",)),
    ("katal", "kat", ()),
    ("joule", "J", ()),
    ("calorie", "cal", ()),
    ("watt", "W", ()),
    ("newton", "N", ()),
    ("pascal", "Pa", ()),
    ("bar", "bar", ()),
    ("standard_atmosphere", "atm", ("atmosphere",)),
    ("beats_per_minute", "bpm", ()),
    ("hertz", "Hz", ()),
    ("volt", "V", ()),
    ("ohm", "Ω", ()),
    ("count", None, ()),
    ("percent", None, ("%",)),
    ("none", None, ()),
    ("item", None, ()),
)

#: the name of a prefix by its name, symbol or alias
_PREFIX_NAMES: Final = {key: p[0] for p in _PREFIXES for key in p}
#: the symbol of a prefix by its name
_PREFIX_SYMBOLS: Final = {p[0]: p[1] for p in _PREFIXES}
#: the name of a unit by its name, symbol or alias
_UNIT_NAMES: Final = {
    key: name
    for name, symbol, aliases in _UNITS
    for key in (name, *((symbol,) if symbol else ()), *aliases)
}
#: the symbol of a unit by its name, its name for a unit without a symbol
_UNIT_SYMBOLS: Final = {name: symbol or name for name, symbol, _ in _UNITS}

#: the tokens of a unit expression: `**`, an operator, a number or a name
_TOKEN: Final = re.compile(
    r"\s*(?:(\*\*|[*/^()])|(-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)|((?:[^\W\d]|°)[\w°]*|%))"
)

_SUPERSCRIPTS: Final = str.maketrans("0123456789-.", "⁰¹²³⁴⁵⁶⁷⁸⁹⁻⋅")

#: the unit `dimensionless`, which a unit drops
_DIMENSIONLESS: Final = "dimensionless"
#: the offset unit of pint and its difference
_CELSIUS: Final = "degree_Celsius"
_DELTA_CELSIUS: Final = "delta_degree_Celsius"


def _unit(name: str) -> tuple[str, str] | None:
    """Get the name of pint and the symbol of a unit name, `None` for an unknown one.

    A name is a unit, its plural (`hours`) or a prefix and a unit (`ng`,
    `milliliter`, `uM`), the unit first as pint does.
    """
    for candidate in (name, name[:-1] if name.endswith("s") else None):
        if not candidate:
            continue
        if candidate in _UNIT_NAMES:
            unit = _UNIT_NAMES[candidate]
            return unit, _UNIT_SYMBOLS[unit]
        for prefix in sorted(_PREFIX_NAMES, key=len, reverse=True):
            rest = candidate[len(prefix) :]
            if candidate.startswith(prefix) and rest in _UNIT_NAMES:
                unit = _UNIT_NAMES[rest]
                full = _PREFIX_NAMES[prefix]
                return full + unit, _PREFIX_SYMBOLS[full] + _UNIT_SYMBOLS[unit]
    return None


class _Parser:
    """A parser of a unit expression into the exponents of its units.

    The grammar of pint without factors: products (`*` or a space) and
    quotients of units, powers (`**` or `^`) with numbers, brackets, and `1`
    as the numerator of a reciprocal (`1/hour`).
    """

    def __init__(self, text: str) -> None:
        self.tokens: list[tuple[str, str]] = []
        position = 0
        text = text.strip()
        while position < len(text):
            match = _TOKEN.match(text, position)
            if match is None or match.end() == position:
                raise ValueError(text)
            operator, number, name = match.groups()
            if operator:
                self.tokens.append(("op", operator))
            elif number:
                self.tokens.append(("number", number))
            else:
                self.tokens.append(("name", name))
            position = match.end()
        self.k = 0
        # the symbols of the units by their names
        self.symbols: dict[str, str] = {}

    def peek(self) -> tuple[str, str] | None:
        return self.tokens[self.k] if self.k < len(self.tokens) else None

    def take(self) -> tuple[str, str]:
        token = self.peek()
        if token is None:
            raise ValueError("end")
        self.k += 1
        return token

    def parse(self) -> dict[str, float]:
        exponents = self.product()
        if self.peek() is not None:
            raise ValueError(self.peek())
        return exponents

    def product(self) -> dict[str, float]:
        exponents = self.power()
        while (token := self.peek()) is not None and token != ("op", ")"):
            if token in (("op", "*"), ("op", "/")):
                self.k += 1
                sign = -1.0 if token[1] == "/" else 1.0
            else:
                # juxtaposition is a product, as in `mmol l**-1`
                sign = 1.0
            for name, exponent in self.power().items():
                exponents[name] = exponents.get(name, 0.0) + sign * exponent
        return exponents

    def power(self) -> dict[str, float]:
        exponents = self.factor()
        if self.peek() in (("op", "**"), ("op", "^")):
            self.k += 1
            kind, value = self.take()
            if kind != "number":
                raise ValueError(value)
            exponents = {name: e * float(value) for name, e in exponents.items()}
        return exponents

    def factor(self) -> dict[str, float]:
        kind, value = self.take()
        if kind == "op" and value == "(":
            exponents = self.product()
            if self.take() != ("op", ")"):
                raise ValueError(")")
            return exponents
        if kind == "number":
            # only `1`, the numerator of a reciprocal; a factor is no unit
            if float(value) != 1.0:
                raise ValueError(value)
            return {}
        if kind != "name":
            raise ValueError(value)
        if value == _DIMENSIONLESS:
            return {}
        unit = _unit(value)
        if unit is None:
            raise ValueError(value)
        self.symbols[unit[0]] = unit[1]
        return {unit[0]: 1.0}


def _power(symbol: str, exponent: float) -> str:
    """Get a symbol with its exponent as superscript, pint's pretty form."""
    if exponent == 1.0:
        return symbol
    text = str(int(exponent)) if exponent.is_integer() else str(exponent)
    return symbol + text.translate(_SUPERSCRIPTS)


def short_unit(unit: str | None) -> str:
    """Get a unit in the short form of pint, e.g. `ng/ml` for `nanogram / milliliter`.

    The form of `f"{unit:~P}"` of pint without pint: the units of the
    numerator joined by `⋅`, those of the denominator each after a `/`, both
    sorted by the names of pint, exponents as superscripts and units which
    cancel dropped, e.g. `mg/h/l` for `mg/(l*h)` and `1/h` for `1/hour`.

    Args:
        unit: the unit as text, the long form of pint (`str` of a unit of
            pint) or the short one.

    Returns:
        The short form, empty for no unit and for a dimensionless one, and the
        unit as given for a text which is no product of the units and prefixes
        the chart knows, e.g. one with a factor or a unit of a model only.
    """
    if not unit or not unit.strip():
        return ""
    try:
        parser = _Parser(unit)
        exponents = {n: e for n, e in parser.parse().items() if e != 0.0}
    except ValueError:
        return unit
    # an offset unit in a product or a power is its difference, as in pint
    if exponents.get(_CELSIUS, 1.0) != 1.0 or (
        _CELSIUS in exponents and len(exponents) > 1
    ):
        exponents[_DELTA_CELSIUS] = exponents.get(_DELTA_CELSIUS, 0.0) + exponents.pop(
            _CELSIUS
        )
        parser.symbols[_DELTA_CELSIUS] = _UNIT_SYMBOLS[_DELTA_CELSIUS]
    numerator = [
        _power(parser.symbols[name], e)
        for name, e in sorted(exponents.items())
        if e > 0
    ]
    denominator = [
        _power(parser.symbols[name], -e)
        for name, e in sorted(exponents.items())
        if e < 0
    ]
    if not denominator:
        return "⋅".join(numerator)
    return "⋅".join(numerator or ["1"]) + "".join(f"/{d}" for d in denominator)


def is_quantity(data: Any) -> bool:
    """Check whether data are a quantity: `.to(unit)`, `.magnitude` and `.units`."""
    return (
        callable(getattr(data, "to", None))
        and hasattr(data, "magnitude")
        and hasattr(data, "units")
    )


def is_data_array(data: Any) -> bool:
    """Check whether data are a data array: `.values`, `.dims` and `.attrs`."""
    return (
        not is_quantity(data)
        and hasattr(data, "values")
        and hasattr(data, "dims")
        and isinstance(getattr(data, "attrs", None), dict)
    )


def is_dimensionality_error(error: BaseException) -> bool:
    """Check whether an error is pint's error of units of different dimensions."""
    return all(hasattr(error, a) for a in ("units1", "units2", "dim1", "dim2"))


def application_quantity() -> Callable[[Any, str], Any] | None:
    """Get the constructor of the quantities of pint's application registry.

    Returns:
        `pint.get_application_registry().Quantity`, `None` without pint.
    """
    try:
        # imported by name: pint is optional and no dependency of the package
        pint = importlib.import_module("pint")
    except ImportError:
        return None
    return pint.get_application_registry().Quantity
