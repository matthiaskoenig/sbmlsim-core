"""Tests of `Runtime`, `LoadedModel` and `Instance.simulate`."""

import math
from pathlib import Path
from typing import Any

import numpy
import pytest
import sbmlsim_core

ROOT = Path(__file__).resolve().parents[2]

MATH = 'xmlns="http://www.w3.org/1998/Math/MathML"'

# A stiff linear model, y' = -1000 (y - cos t), whose integration over a long time needs
# many more steps than two.
STIFF_SBML = f"""<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="stiff">
    <listOfParameters>
      <parameter id="y" value="0" constant="false"/>
      <parameter id="k" value="1000" constant="true"/>
    </listOfParameters>
    <listOfRules>
      <rateRule variable="y">
        <math {MATH}>
          <apply><times/><apply><minus/><ci>k</ci></apply>
            <apply><minus/><ci>y</ci><apply><cos/><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">time</csymbol></apply></apply>
          </apply>
        </math>
      </rateRule>
    </listOfRules>
  </model>
</sbml>"""


def request(**fields: Any) -> dict[str, Any]:
    """A request of `simulation/1` over `[0, end]` in `steps` equal intervals."""
    steps = fields.pop("steps")
    end = fields.pop("end")
    settings = {
        "relative_tolerance": 1e-6,
        "absolute_tolerance": {"scalar": 1e-12},
        "maximum_num_steps": 20000,
        "maximum_time_step": 0.0,
        "initial_time_step": 0.0,
        "stiff": True,
        "linear_solver": "auto",
        "jacobian": "auto",
    } | fields
    return {
        "contract": sbmlsim_core.SIMULATION_CONTRACT,
        "start": 0.0,
        "end": end,
        "output": {"uniform": {"steps": steps}},
        "preinit": [],
        "changes": [],
        "steady_state": None,
        "selections": [],
        "settings": settings,
    }


@pytest.fixture(scope="module")
def runtime() -> sbmlsim_core.Runtime:
    """The runtime of the embedded engine."""
    return sbmlsim_core.Runtime()


def loaded(runtime: sbmlsim_core.Runtime, sbml: str) -> sbmlsim_core.LoadedModel:
    """Compiles and loads an SBML model."""
    compiled = sbmlsim_core.compile_sbml(sbml)
    return runtime.load(compiled.wasm, compiled.description)


@pytest.fixture(scope="module")
def decay(runtime: sbmlsim_core.Runtime) -> sbmlsim_core.LoadedModel:
    """The decay model of the host tests: y' = -k y, y(0) = 1, k = 0.5."""
    sbml = (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    return loaded(runtime, sbml)


@pytest.fixture(scope="module")
def stiff(runtime: sbmlsim_core.Runtime) -> sbmlsim_core.LoadedModel:
    """A stiff model which cannot be integrated within two steps."""
    return loaded(runtime, STIFF_SBML)


def test_simulate_gives_the_columns_and_an_array(decay):
    out = decay.instance().simulate(request(end=10.0, steps=10))
    assert out.columns[0] == "time"
    assert out.values.shape == (11, len(out.columns))
    assert out.values.dtype == numpy.float64
    assert out.values.flags.c_contiguous
    assert out.values[0, 0] == 0.0 and out.values[-1, 0] == 10.0
    y = out.columns.index("y")
    assert out.values[0, y] == 1.0
    assert out.values[-1, y] == pytest.approx(math.exp(-5.0), rel=1e-4)


def test_an_increment_of_a_change_adds_to_the_state_before_it(decay):
    req = request(end=10.0, steps=2)
    req["changes"] = [{"time": 5.0, "values": [], "increments": [["y", 1.0]]}]
    out = decay.instance().simulate(req)
    y = out.columns.index("y")
    # the state before the change and after it, at its time
    assert out.values[1, 0] == out.values[2, 0] == 5.0
    assert out.values[2, y] == out.values[1, y] + 1.0


def test_initial_values_are_the_model_as_loaded(decay):
    values = decay.initial_values()
    assert values["y"] == 1.0
    assert values["k"] == 0.5
    assert values["t_half"] == pytest.approx(math.log(2) / 0.5)


def test_the_description_is_the_one_given(decay):
    assert decay.description["contract"] == sbmlsim_core.MODEL_CONTRACT


def test_stats_counts_the_steps(decay):
    instance = decay.instance()
    instance.simulate(request(end=10.0, steps=10))
    assert instance.stats()["steps"] > 0


def test_a_reused_instance_gives_the_bits_of_a_fresh_one(decay):
    reused = decay.instance()
    first = reused.simulate(request(end=10.0, steps=10))
    reused.simulate(request(end=3.0, steps=7))
    again = reused.simulate(request(end=10.0, steps=10))
    fresh = decay.instance().simulate(request(end=10.0, steps=10))
    assert first.values.tobytes() == again.values.tobytes() == fresh.values.tobytes()


def test_an_unknown_field_of_a_request_is_refused(decay):
    with pytest.raises(ValueError, match="nonsense"):
        decay.instance().simulate(request(end=10.0, steps=10) | {"nonsense": 1})


def test_a_request_with_a_wrong_type_is_refused(decay):
    with pytest.raises(ValueError, match="end"):
        decay.instance().simulate(request(end="ten", steps=10))


def test_a_request_the_engine_refuses_raises_a_request_error(decay):
    with pytest.raises(sbmlsim_core.RequestError):
        decay.instance().simulate(
            request(end=10.0, steps=10) | {"selections": ["nope"]}
        )


def test_an_integration_which_fails_raises_with_its_flag(stiff):
    with pytest.raises(sbmlsim_core.IntegrationError) as caught:
        stiff.instance().simulate(request(end=1e6, steps=10, maximum_num_steps=2))
    assert caught.value.flag != 0 and caught.value.time >= 0.0


def test_the_errors_of_a_simulation_share_a_base():
    for error in (
        sbmlsim_core.RequestError,
        sbmlsim_core.IntegrationError,
        sbmlsim_core.UnsupportedError,
        sbmlsim_core.InternalError,
    ):
        assert issubclass(error, sbmlsim_core.SimulationError)
    assert issubclass(sbmlsim_core.SimulationError, sbmlsim_core.SbmlsimCoreError)


def test_load_refuses_a_description_of_another_contract(runtime):
    compiled = sbmlsim_core.compile_sbml(
        (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    )
    with pytest.raises(sbmlsim_core.SbmlsimCoreError, match="contract"):
        runtime.load(compiled.wasm, compiled.description | {"contract": "model/1"})


def test_load_refuses_bytes_which_are_no_module(runtime):
    compiled = sbmlsim_core.compile_sbml(
        (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    )
    with pytest.raises(sbmlsim_core.SbmlsimCoreError):
        runtime.load(b"not wasm", compiled.description)


def test_a_result_which_does_not_fit_is_a_request_error():
    # an output of 3 million rows of 4 columns, 96 MB, into an instance of at most 32 MiB
    runtime = sbmlsim_core.Runtime(max_memory_bytes=32 * 1024 * 1024)
    sbml = (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    instance = loaded(runtime, sbml).instance()
    with pytest.raises(sbmlsim_core.RequestError, match="does not fit into the memory"):
        instance.simulate(request(end=1.0, steps=3_000_000))
    # the instance answers the next request
    assert instance.simulate(request(end=1.0, steps=10)).values.shape == (11, 4)


def test_runtime_timeout_requires_nonnegative_integer():
    for value in (-1, 2**64):
        with pytest.raises(OverflowError):
            sbmlsim_core.Runtime(timeout_ms=value)
    with pytest.raises(TypeError):
        sbmlsim_core.Runtime(timeout_ms=1.5)
