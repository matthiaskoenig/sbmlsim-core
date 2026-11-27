"""The units of a chart: the short form of a unit and the optional pint and xarray."""

from __future__ import annotations

import subprocess
import sys
from typing import Any

import numpy as np
import pytest
from sbmlsim_core.chart import Chart, ChartAxis, axis_label, builder, short_unit

#: units with their short form, which is the one of pint (`f"{unit:~P}"`)
SHORT = {
    "": "",
    "dimensionless": "",
    "1": "",
    "hr": "h",
    "hour": "h",
    "hours": "h",
    "nanogram / milliliter": "ng/ml",
    "ng/mL": "ng/ml",
    "mg/l/h": "mg/h/l",
    "mg/(l*h)": "mg/h/l",
    "1/hour": "1/h",
    "1 / minute": "1/min",
    "s**-1": "1/s",
    "m^-1": "1/m",
    "m**2": "m²",
    "m^2": "m²",
    "kg/m**2": "kg/m²",
    "(mg/l)**2": "mg²/l²",
    "m**0.5": "m⁰⋅⁵",
    "mg*h/l": "h⋅mg/l",
    "nanogram * hour / milliliter": "h⋅ng/ml",
    "ml/min/kg": "ml/kg/min",
    "milligram / kilogram / day": "mg/d/kg",
    "day*year": "d⋅a",
    "mg/mg": "",
    "hr/h": "",
    "µM": "μM",
    "uM": "μM",
    "micromolar": "μM",
    "mcg/kg": "μg/kg",
    "mmol l**-1": "mmol/l",
    "L/min": "l/min",
    "dL": "dl",
    "mmHg": "mmHg",
    "millimeter_Hg": "mmHg",
    "degC": "°C",
    "mg/degC": "mg/Δ°C",
    "kDa": "kDa",
    "U/l": "U/l",
    "weeks": "week",
    "percent": "percent",
    "percent/hour": "percent/h",
    "count/ml": "count/ml",
    "ohm": "Ω",
    # a factor, a unit of a model and what is no expression are shown as given
    "ml/min/(1.73*m**2)": "ml/min/(1.73*m**2)",
    "10**3/µl": "10**3/µl",
    "IU_insulin/ml": "IU_insulin/ml",
    "cells/ul": "cells/ul",
    "mmol l-1": "mmol l-1",
    "mg.h/l": "mg.h/l",
}


@pytest.mark.parametrize(("unit", "short"), SHORT.items())
def test_short_unit(unit: str, short: str) -> None:
    assert short_unit(unit) == short


def test_short_unit_is_the_one_of_pint() -> None:
    """The short form is pint's for every unit pint parses, given short or long."""
    pint = pytest.importorskip("pint")
    registry = pint.UnitRegistry()
    # the units without a symbol of the registry of sbmlsim
    registry.define("none = count")
    registry.define("item = count")
    registry.define("percent = 0.01*count")
    for unit in SHORT:
        try:
            parsed = registry.Unit(unit)
        except Exception:
            continue
        assert short_unit(unit) == f"{parsed:~P}", unit
        assert short_unit(str(parsed)) == f"{parsed:~P}", unit
    assert short_unit(None) == ""


def test_axis_label_of_a_unit_of_pint() -> None:
    pint = pytest.importorskip("pint")
    assert axis_label("time", pint.UnitRegistry().Unit("hr")) == "time [h]"


def test_the_chart_imports_neither_pint_nor_xarray() -> None:
    code = (
        "import sys, sbmlsim_core.chart as c; "
        "c.Chart().plot().curve([0, 1], [0, 1]); "
        "print(sorted({'pint', 'xarray'} & set(sys.modules)))"
    )
    out = subprocess.run(
        [sys.executable, "-c", code], capture_output=True, text=True, check=True
    )
    assert out.stdout.strip() == "[]"


#: the minutes of the units of `_Quantity`
_MINUTES = {"hour": 60.0, "minute": 1.0}


class _Quantity:
    """A quantity of a registry of its own, by duck typing: hours and minutes."""

    def __init__(self, magnitude: Any, units: str) -> None:
        self.magnitude = np.asarray(magnitude, dtype=float)
        self.units = units

    def to(self, unit: str) -> _Quantity:
        if unit not in _MINUTES:
            raise KeyError(unit)
        factor = _MINUTES[self.units] / _MINUTES[unit]
        return _Quantity(self.magnitude * factor, unit)


class _DataArray:
    """A data array by duck typing."""

    def __init__(self, values: Any, units: str | None, name: str | None = None):
        self.values = np.asarray(values, dtype=float)
        self.dims = ("time",)
        self.attrs = {"units": units} if units else {}
        self.name = name


def test_a_quantity_is_converted_by_its_own_registry() -> None:
    plot = Chart().plot(x=ChartAxis(label="time", unit="hour"))
    plot.curve(_Quantity([30.0, 90.0], "minute"), [1.0, 2.0])
    assert plot.spec()["elements"][0]["x"] == [0.5, 1.5]
    assert plot.spec()["x"]["unit"] == "h"
    with pytest.raises(ValueError, match="axis x"):
        plot.curve(_Quantity([1.0], "mM"), [1.0])

    # an axis without a unit takes the one of the first quantity
    plot = Chart().plot()
    plot.curve(_Quantity([1.0, 2.0], "hour"), [1.0, 2.0])
    plot.curve(_Quantity([60.0], "minute"), [1.0])
    spec = plot.spec()
    assert spec["x"]["unit"] == "h"
    assert spec["elements"][1]["x"] == [1.0]


def test_a_data_array_without_pint(monkeypatch: pytest.MonkeyPatch) -> None:
    """A data array is taken in its unit, a conversion needs pint."""
    monkeypatch.setattr(builder, "application_quantity", lambda: None)
    plot = Chart().plot()
    plot.curve(_DataArray([0.0, 60.0], "minute", name="time"), [1.0, 2.0])
    plot.curve(_DataArray([1.0], "minute"), [1.0])
    plot.curve(_DataArray([1.0], None), [1.0])
    spec = plot.spec()
    assert (spec["x"]["label"], spec["x"]["unit"]) == ("time", "min")
    assert spec["elements"][1]["x"] == [1.0]
    with pytest.raises(ValueError, match="pint, which is not installed"):
        plot.curve(_DataArray([1.0], "hour"), [1.0])


def test_a_data_array_is_converted_by_the_application_registry() -> None:
    pytest.importorskip("pint")
    plot = Chart().plot(x=ChartAxis(unit="hr"))
    plot.curve(_DataArray([30.0, 90.0], "minute"), [1.0, 2.0])
    assert plot.spec()["elements"][0]["x"] == [0.5, 1.5]
