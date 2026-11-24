"""Tests of `Runtime`, `LoadedModel` and `Instance.simulate`."""

import json
import math
import subprocess
import sys
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


def test_algebraic_rules_have_consistent_initial_values_and_trajectories(runtime):
    model = loaded(
        runtime,
        f"""<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
<model id="dae"><listOfParameters>
<parameter id="x" value="1" constant="false"/>
<parameter id="z" constant="false"/>
<parameter id="k" value="0.5" constant="true"/>
</listOfParameters><listOfRules>
<rateRule variable="x"><math {MATH}>
<apply><minus/><apply><times/><ci>k</ci><ci>x</ci></apply></apply>
</math></rateRule>
<algebraicRule><math {MATH}>
<apply><minus/><ci>z</ci><apply><times/><ci>x</ci><ci>x</ci></apply></apply>
</math></algebraicRule>
</listOfRules></model></sbml>""",
    )
    assert model.description["n_algebraic"] == 1
    assert model.initial_values()["z"] == pytest.approx(1.0)
    req = request(end=2.0, steps=8, relative_tolerance=1e-10)
    req["selections"] = ["x", "z"]
    result = model.instance().simulate(req)
    assert result.columns == ["time", "x", "z"]
    expected = numpy.exp(-0.5 * result.values[:, 0])
    numpy.testing.assert_allclose(result.values[:, 1], expected, rtol=2e-8)
    numpy.testing.assert_allclose(result.values[:, 2], expected**2, rtol=2e-8)


def test_compilation_cache_is_reused_by_fresh_python_processes(tmp_path):
    cache = tmp_path / "compilation-cache"
    script = """
import json
import sys
from pathlib import Path
import sbmlsim_core

runtime = sbmlsim_core.Runtime(cache_dir=Path(sys.argv[1]))
compiled = sbmlsim_core.compile_sbml(Path(sys.argv[2]).read_text())
model = runtime.load(compiled.wasm, compiled.description)
print(json.dumps([str(runtime.cache_dir), model.initial_values()]))
"""
    command = [
        sys.executable,
        "-c",
        script,
        str(cache),
        str(ROOT / "crates/host/tests/decay.xml"),
    ]
    first = subprocess.check_output(command, text=True, timeout=60)
    assert cache.is_dir()
    assert any((cache / "modules").rglob("*"))
    second = subprocess.check_output(command, text=True, timeout=60)
    assert json.loads(first) == json.loads(second)
    assert Path(json.loads(first)[0]).samefile(cache)


def test_unavailable_compilation_cache_falls_back_to_compilation(tmp_path):
    blocked = tmp_path / "a-file"
    blocked.write_text("not a directory")
    runtime = sbmlsim_core.Runtime(cache_dir=blocked / "cache")
    assert runtime.cache_dir is None
    sbml = (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    assert loaded(runtime, sbml).initial_values()["y"] == 1.0
    uncached = sbmlsim_core.Runtime(cache=False)
    assert uncached.cache_dir is None
    assert loaded(uncached, sbml).initial_values()["y"] == 1.0


def test_cache_directory_requires_caching_to_be_enabled(tmp_path):
    with pytest.raises(ValueError, match="cache_dir requires cache=True"):
        sbmlsim_core.Runtime(cache=False, cache_dir=tmp_path)


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


def test_stats_carry_the_state_of_the_solver(decay):
    instance = decay.instance()
    instance.simulate(request(end=10.0, steps=10))
    stats = instance.stats()
    assert isinstance(stats["steps"], int)
    assert stats["last_order"] >= 1
    # the times and the steps of the state are floats, not the bits of their words
    for name in (
        "actual_initial_time_step",
        "last_time_step",
        "current_time_step",
        "current_time",
    ):
        assert isinstance(stats[name], float), name
        assert stats[name] > 0.0, name
    assert stats["current_time"] >= 10.0


def test_the_maximum_bdf_order_bounds_the_order(decay):
    instance = decay.instance()
    free = request(end=10.0, steps=10, relative_tolerance=1e-10)
    instance.simulate(free)
    unbounded = instance.stats()
    instance.simulate(free | {"settings": free["settings"] | {"maximum_bdf_order": 2}})
    bounded = instance.stats()
    assert unbounded["last_order"] > 2
    assert bounded["last_order"] <= 2
    assert bounded["steps"] > unbounded["steps"]


def test_a_setting_of_the_wrong_method_is_refused(decay):
    instance = decay.instance()
    req = request(end=10.0, steps=10, stiff=False, stability_limit_detection=True)
    with pytest.raises(sbmlsim_core.RequestError, match="stability_limit_detection"):
        instance.simulate(req)


def test_gmres_matches_decay_and_reports_iterative_counts(decay):
    instance = decay.instance()
    req = request(end=10.0, steps=10, linear_solver="gmres", relative_tolerance=1e-9)
    result = instance.simulate(req)
    expected = decay.instance().simulate(
        request(end=10.0, steps=10, relative_tolerance=1e-9)
    )
    numpy.testing.assert_allclose(result.values, expected.values, rtol=1e-7, atol=1e-10)
    stats = instance.stats()
    for name in (
        "linear_iterations",
        "jacobian_vector_evals",
        "preconditioner_evals",
        "preconditioner_solves",
    ):
        assert stats[name] > 0
    assert instance.simulate(req).values.tobytes() == result.values.tobytes()
    assert instance.stats() == stats


def test_delay_feedback_initial_values_and_replay(runtime):
    sbml = f"""<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
<model id="delayed"><listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><apply>
<csymbol definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>x</ci><cn>1</cn>
</apply></math></rateRule></listOfRules></model></sbml>"""
    compiled = sbmlsim_core.compile_sbml(sbml)
    assert compiled.description["n_delays"] > 0
    model = runtime.load(compiled.wasm, compiled.description)
    assert model.initial_values()["x"] == 1.0
    instance = model.instance()
    req = request(end=3.0, steps=2, relative_tolerance=1e-9)
    result = instance.simulate(req)
    numpy.testing.assert_allclose(
        result.values[:, 1], [1.0, 2.625, 6 + 1 / 6], rtol=1e-6
    )
    assert instance.simulate(req).values.tobytes() == result.values.tobytes()


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


def test_a_request_without_sensitivities_has_none(decay):
    out = decay.instance().simulate(request(end=1.0, steps=1))
    assert out.sensitivities is None


def test_the_forward_sensitivities_of_a_run(decay):
    # y' = -k y with y(0) = 1 and k = 0.5, so dy/dk = -t exp(-k t) and
    # d(t_half)/dk = -ln(2) / k^2; `time` is no function of a parameter
    req = request(end=4.0, steps=8)
    req["sensitivities"] = ["k", "y"]
    out = decay.instance().simulate(req)
    sensitivities = out.sensitivities
    assert sensitivities is not None
    assert sensitivities.parameters == ["k", "y"]
    assert sensitivities.values.shape == (9, len(out.columns), 2)
    assert sensitivities.values.dtype == numpy.float64
    assert sensitivities.values.flags.c_contiguous
    time = out.values[:, out.columns.index("time")]
    assert numpy.all(sensitivities.values[:, out.columns.index("time"), :] == 0.0)
    y = out.columns.index("y")
    numpy.testing.assert_allclose(
        sensitivities.values[:, y, 0],
        -time * numpy.exp(-0.5 * time),
        rtol=1e-4,
        atol=1e-8,
    )
    # the sensitivity of a state to its own initial value
    numpy.testing.assert_allclose(
        sensitivities.values[:, y, 1], numpy.exp(-0.5 * time), rtol=1e-4, atol=1e-8
    )
    # an assignment rule follows the sensitivities of the states and the parameter
    half = out.columns.index("t_half")
    assert sensitivities.values[0, half, 0] == pytest.approx(
        -math.log(2) / 0.25, rel=1e-6
    )
    assert sensitivities.values[0, half, 1] == pytest.approx(0.0, abs=1e-9)
