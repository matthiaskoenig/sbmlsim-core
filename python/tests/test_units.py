"""The units of pint in Rust (`sbmlsim_core.units`): the corpus of sbmlsim and the errors."""

import json
from pathlib import Path

import numpy as np
import pytest
import sbmlsim_core
from sbmlsim_core import units

#: the corpus of sbmlsim, the factors and dimensions of `sbmlsim.units.ureg`
CORPUS = Path(__file__).parents[2] / "testdata" / "units_corpus.json"


def corpus() -> list[dict]:
    return json.loads(CORPUS.read_text())


def test_every_expression_matches_pint() -> None:
    registry = units.Registry()
    entries = corpus()
    assert len(entries) == 177
    for entry in entries:
        factor, dimensions = registry.parse(entry["expression"])
        expected = entry["factor_to_root"]
        assert abs(factor - expected) <= 1e-15 * abs(expected), entry
        assert dimensions == entry["dimensions"], entry


def test_factors_of_pint() -> None:
    registry = units.Registry()
    assert registry.factor("l", "ml") == 1000.0
    assert registry.factor("mg/min", "g/hr") == 0.06
    assert registry.factor(from_="hr", to="min") == 60.0
    assert registry.parse("percent") == (0.01, {})


def test_errors() -> None:
    registry = units.Registry()
    assert issubclass(units.UnitError, sbmlsim_core.SbmlsimCoreError)
    assert units.UnitError is sbmlsim_core.UnitError
    with pytest.raises(
        units.UnitError, match=r"cannot convert 'mg' \(\[mass\]\) to 'ml'"
    ):
        registry.factor("mg", "ml")
    with pytest.raises(units.UnitError, match="degree_Celsius"):
        registry.factor("degC", "K")
    with pytest.raises(units.UnitError, match="'foo'"):
        registry.parse("mg/foo")
    with pytest.raises(units.UnitError, match=r"power 0\.5"):
        registry.parse("m**0.5")


def test_define_and_model_units() -> None:
    registry = units.Registry()
    registry.define("IU_insulin", "0.0347 * mg")
    registry.define("IU_insulin", "0.0347 * mg")
    with pytest.raises(units.UnitError, match="IU_insulin"):
        registry.define("IU_insulin", "0.045 * mg")
    assert registry.factor("IU_insulin", "ug") == 34.7
    model = registry.with_model_units(
        units={"CL": "mg_per_min/kg"}, unit_ids={"mg_per_min": "mg/min"}
    )
    assert model.unit_of("CL") == "mg_per_min/kg"
    assert model.unit_of("V") is None
    assert model.factor("mg_per_min/kg", "mg/min/kg") == 1.0
    # the copy keeps the units defined before it, the registry has no unit ids
    assert model.factor("IU_insulin", "mg") == 0.0347
    with pytest.raises(units.UnitError):
        registry.parse("mg_per_min")


def test_convert() -> None:
    registry = units.Registry()
    converted = registry.convert([1.0, 2.0], "hr", "min")
    assert converted.dtype == np.float64
    assert converted.tolist() == [60.0, 120.0]
    assert registry.convert(np.array([1.0]), "l", "ml").tolist() == [1000.0]
    with pytest.raises(units.UnitError, match="cannot convert"):
        registry.convert([1.0], "mg", "ml")
