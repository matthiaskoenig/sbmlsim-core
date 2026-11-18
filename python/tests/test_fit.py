"""Tests of `LoadedModel.fit`, the fit of a model on the batch of the engine (`fit/1`)."""

import _thread
import sys
import threading
import time
from pathlib import Path
from typing import Any

import numpy
import pytest
import sbmlsim_core

ROOT = Path(__file__).resolve().parents[2]


def request(**fields: Any) -> dict[str, Any]:
    """A request of `simulation/1` over `[0, end]` in `steps` equal intervals."""
    steps = fields.pop("steps", None)
    end = fields.pop("end")
    output = fields.pop("output", {"uniform": {"steps": steps}})
    preinit = fields.pop("preinit", [])
    settings = {
        "relative_tolerance": 1e-8,
        "absolute_tolerance": {"scalar": 1e-12},
        "maximum_num_steps": 20000,
        "maximum_time_step": 0.0,
        "initial_time_step": 0.0,
        "stiff": False,
        "linear_solver": "auto",
        "jacobian": "auto",
    } | fields
    return {
        "contract": sbmlsim_core.SIMULATION_CONTRACT,
        "start": 0.0,
        "end": end,
        "output": output,
        "preinit": preinit,
        "changes": [],
        "steady_state": None,
        "selections": ["time", "y"],
        "settings": settings,
    }


def slow_request() -> dict[str, Any]:
    """A condition of about 80 ms with 11 rows, a long time in small steps."""
    return request(
        end=1e6, steps=10, maximum_time_step=10.0, maximum_num_steps=100_000_000
    )


@pytest.fixture(scope="module")
def decay() -> sbmlsim_core.LoadedModel:
    """The decay model of the host tests: y' = -k y, y(0) = 1, k = 0.5."""
    sbml = (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    compiled = sbmlsim_core.compile_sbml(sbml)
    return sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)


def simulate(
    decay: sbmlsim_core.LoadedModel, **over: Any
) -> tuple[list[float], list[float]]:
    """The time and `y` of the model at k = 0.3, which is the data of a fit."""
    out = decay.instance().simulate(request(preinit=[("k", 0.3)], **over))
    return (
        list(out.values[:, out.columns.index("time")]),
        list(out.values[:, out.columns.index("y")]),
    )


@pytest.fixture(scope="module")
def data(decay: sbmlsim_core.LoadedModel) -> tuple[list[float], list[float]]:
    """The data of the fit: the time and `y` of the model at k = 0.3."""
    return simulate(decay, end=10.0, steps=10)


@pytest.fixture(scope="module")
def slow(decay: sbmlsim_core.LoadedModel) -> dict[str, Any]:
    """A fit of a condition of about 80 ms, whose generations are long enough to time."""
    condition = slow_request()
    times, values = simulate(
        decay,
        end=condition["end"],
        steps=10,
        maximum_time_step=condition["settings"]["maximum_time_step"],
        maximum_num_steps=condition["settings"]["maximum_num_steps"],
    )
    return fit_request(
        (times, values),
        condition=condition,
        optimizer={
            "algorithm": {
                "differential_evolution": {"seed": 1, "population": 8, "polish": False}
            },
            "max_iterations": 200,
        },
    )


def fit_request(
    data: tuple[list[float], list[float]],
    condition: dict[str, Any] | None = None,
    **over: Any,
) -> dict[str, Any]:
    """The fit of `k` against the data, from a start of three."""
    times, values = data
    return {
        "contract": sbmlsim_core.FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": 3.0, "lower": 0.01, "upper": 10.0, "scale": "log10"}
        ],
        "conditions": [
            {"id": "decay", "request": condition or request(end=10.0, steps=10)}
        ],
        "curves": [
            {
                "id": "y of the decay",
                "condition": "decay",
                "column": "y",
                "times": times,
                "values": values,
            }
        ],
        "optimizer": {"max_iterations": 50},
    } | over


def pair_request(data: tuple[list[float], list[float]], **over: Any) -> dict[str, Any]:
    """The fit of `k` and of the initial value of `y`, with the deviation of the data."""
    times, values = data
    return {
        "contract": sbmlsim_core.FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": 0.5, "lower": 0.01, "upper": 10.0, "scale": "log10"},
            {"id": "y", "start": 2.0, "lower": 0.1, "upper": 10.0, "scale": "log10"},
        ],
        "conditions": [{"id": "decay", "request": request(end=10.0, steps=10)}],
        "curves": [
            {
                "condition": "decay",
                "column": "y",
                "times": times,
                "values": values,
                "noise": [0.02] * len(values),
            }
        ],
    } | over


def test_a_fit_finds_the_parameter_of_the_data(decay, data):
    answer = decay.fit(fit_request(data), threads=2)

    assert answer["contract"] == sbmlsim_core.FIT_CONTRACT
    assert answer["parameters"][0]["id"] == "k"
    assert answer["parameters"][0]["value"] == pytest.approx(0.3, abs=1e-6)
    assert answer["cost"] < 1e-12
    assert len(answer["residuals"]) == 11
    assert answer["evaluations"] > 0 and answer["jacobians"] > 0
    assert answer["seconds"] > 0.0
    assert len(answer["runs"]) == 1
    assert answer["runs"][0]["start"] == [3.0]


def test_an_evolution_of_a_fit_is_the_block_of_its_optimizer(decay, data):
    answer = decay.fit(
        fit_request(
            data,
            optimizer={
                "algorithm": {"differential_evolution": {"seed": 7, "population": 10}},
                "max_iterations": 20,
            },
        ),
        threads=2,
    )

    assert answer["parameters"][0]["value"] == pytest.approx(0.3, abs=1e-6)


def test_a_fit_of_many_starts_keeps_the_best(decay, data):
    answer = decay.fit(fit_request(data, starts=[[0.02], [3.0]]), threads=2)

    assert [run["start"] for run in answer["runs"]] == [[0.02], [3.0]]
    assert answer["cost"] == min(run["cost"] for run in answer["runs"])


def test_a_fit_of_another_contract_raises(decay, data):
    with pytest.raises(ValueError, match="fit/1"):
        decay.fit(fit_request(data, contract="fit/2"))


def test_a_fit_whose_curve_has_no_condition_raises(decay, data):
    broken = fit_request(data)
    broken["curves"][0]["condition"] = "the other dose"

    with pytest.raises(ValueError, match="the other dose"):
        decay.fit(broken)


def test_a_request_which_is_no_fit_raises(decay):
    with pytest.raises(ValueError, match="request"):
        decay.fit({"contract": sbmlsim_core.FIT_CONTRACT, "parameters": 3})


def test_a_fit_whose_model_has_not_the_parameter_raises(decay, data):
    broken = fit_request(data)
    broken["parameters"][0]["id"] = "clearance"

    with pytest.raises(ValueError, match="clearance"):
        decay.fit(broken)


def test_a_fit_runs_without_the_gil(decay, slow):
    # a thread of python which does nothing but count cannot run while a call holds the GIL,
    # so the ticks it makes are the proof that the fit let go of it
    ticks = 0
    stop = threading.Event()

    def tick() -> None:
        nonlocal ticks
        while not stop.is_set():
            ticks += 1

    ticker = threading.Thread(target=tick)
    ticker.start()
    try:
        answer = decay.fit(
            slow | {"optimizer": slow["optimizer"] | {"max_iterations": 4}}, threads=2
        )
    finally:
        stop.set()
        ticker.join()

    assert answer["seconds"] > 0.1, "the fit was too short to tell"
    assert ticks > 1000, f"the fit held the GIL, the ticker made {ticks} ticks"


def test_a_long_fit_is_interrupted(decay, slow):
    # the interrupt has to reach the fit while it runs, which the check of the optimizer per
    # iteration sees; the fit is 200 generations of a condition of 80 ms, so it cannot have
    # ended before the timer either. The elapsed time is the claim of the test, so the
    # interrupt is caught here and not with `pytest.raises` (see `test_batch.py`).
    calibrated = time.perf_counter()
    decay.fit(
        slow | {"optimizer": slow["optimizer"] | {"max_iterations": 1}}, threads=2
    )
    generation = time.perf_counter() - calibrated
    interrupt = threading.Timer(0.05, _thread.interrupt_main)
    switch = sys.getswitchinterval()
    sys.setswitchinterval(1000.0)
    started = time.perf_counter()
    try:
        interrupt.start()
        decay.fit(slow, threads=2)
        for _ in range(1000):
            pass
        interrupted = False
    except KeyboardInterrupt:
        interrupted = True
    finally:
        interrupt.cancel()
        sys.setswitchinterval(switch)
    elapsed = time.perf_counter() - started

    assert interrupted, "the fit was never interrupted"
    assert elapsed < 10 * generation + 1.0, (
        f"the fit of {elapsed:.3f} s was not stopped at its next generation "
        f"({generation:.3f} s)"
    )


def test_a_fit_answers_with_the_identifiability_of_its_optimum(decay, data):
    answer = decay.fit(
        pair_request(
            data,
            identifiability={"walk": {"points": 3, "span": 0.3}, "fisher": True},
        ),
        threads=2,
    )

    assert [profile["id"] for profile in answer["profiles"]] == ["k", "y"]
    k = answer["profiles"][0]
    assert len(k["points"]) == 6
    assert k["identifiable"]
    assert k["lower"] < 0.3 < k["upper"]
    assert k["threshold"] > k["cost"]
    assert answer["fisher"]["parameters"] == ["k", "y"]
    assert all(error > 0.0 for error in answer["fisher"]["errors"])
    assert answer["fisher"]["correlation"][0] == pytest.approx(1.0)


def test_a_fit_without_the_block_of_the_identifiability_answers_without_profiles(
    decay, data
):
    answer = decay.fit(pair_request(data), threads=2)

    assert "profiles" not in answer
    assert "fisher" not in answer


def test_the_problem_of_a_fit_answers_the_residuals_and_the_jacobian(decay, data):
    problem = decay.fit_problem(pair_request(data), threads=2)

    assert problem.parameters == ["k", "y"]
    assert problem.n_residuals == 11
    assert problem.threads == 2
    assert len(problem.start) == 2
    lower, upper = problem.bounds
    assert len(lower) == 2 and len(upper) == 2
    assert all(low < high for low, high in zip(lower, upper, strict=True))

    residuals = problem.residuals(problem.start)
    assert residuals.shape == (11,)
    assert problem.cost(problem.start) == pytest.approx(
        0.5 * float((residuals**2).sum())
    )

    jacobian = problem.jacobian(problem.start)
    assert jacobian.shape == (11, 2)
    # the column of a parameter is the forward difference of the residuals over its step
    step = problem.step(0, problem.start)
    moved = list(problem.start)
    moved[0] += step
    ahead = problem.residuals(moved)
    assert jacobian[:, 0].tolist() == ((ahead - residuals) / step).tolist()


def test_the_problem_of_a_fit_is_the_objective_of_an_optimizer_of_python(decay, data):
    # a plain Gauss-Newton of four steps, which is what a caller whose optimizer stays in
    # python does with the problem: every step is one jacobian of the engine
    problem = decay.fit_problem(pair_request(data), threads=2)
    x = numpy.array(problem.start)
    for _ in range(8):
        residuals = problem.residuals(x)
        jacobian = problem.jacobian(x)
        step, *_ = numpy.linalg.lstsq(jacobian, -residuals, rcond=None)
        x = x + step

    values = problem.values(x)
    assert values[0] == pytest.approx(0.3, rel=1e-4)
    assert values[1] == pytest.approx(1.0, rel=1e-4)
    assert problem.cost(x) < 1e-6


def test_the_problem_of_a_fit_refuses_a_point_which_is_not_its_parameters(decay, data):
    problem = decay.fit_problem(pair_request(data), threads=2)

    with pytest.raises(ValueError, match="2 parameters"):
        problem.residuals([1.0])
    with pytest.raises(ValueError, match="not finite"):
        problem.cost([1.0, float("nan")])
    with pytest.raises(ValueError, match="none of them"):
        problem.step(5, problem.start)
