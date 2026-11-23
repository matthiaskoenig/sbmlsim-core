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
    # the jacobian of a step is the one of the finite differences, which a fit takes by
    # itself, and the answer says so
    assert answer["jacobian"] == "difference_quotient"
    assert answer["simulations"] >= answer["evaluations"]
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
            identifiability={"walk": {"points": 3, "step": 0.1}, "fisher": True},
        ),
        threads=2,
    )

    assert [profile["id"] for profile in answer["profiles"]] == ["k", "y"]
    k = answer["profiles"][0]
    # at most three points on each side, fewer where the walk crosses the threshold
    assert 0 < len(k["points"]) <= 6
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
    problem = decay.fit_problem(pair_request(data, jacobian="sensitivities"), threads=2)

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

    # the jacobian of the forward sensitivities of the condition, one simulation of it
    assert problem.jacobian_of == "sensitivities"
    before = problem.simulations
    jacobian = problem.jacobian(problem.start)
    assert problem.simulations - before == 1
    assert jacobian.shape == (11, 2)
    # which is the derivative of the residuals: a central difference of them, which is
    # accurate to about 1e-9 of itself, is the same jacobian
    for column in range(2):
        step = 1e-6
        below, above = list(problem.start), list(problem.start)
        below[column] -= step
        above[column] += step
        central = (problem.residuals(above) - problem.residuals(below)) / (2.0 * step)
        assert jacobian[:, column] == pytest.approx(central, rel=1e-5, abs=1e-9)


def test_the_jacobian_of_the_differences_is_the_one_of_a_column_at_a_time(decay, data):
    # the finite differences, which a fit of a condition the sensitivities do not carry
    # takes: the column of a parameter is the forward difference over its step
    problem = decay.fit_problem(
        pair_request(data, jacobian="difference_quotient"), threads=2
    )

    assert problem.jacobian_of == "difference_quotient"
    residuals = problem.residuals(problem.start)
    before = problem.simulations
    jacobian = problem.jacobian(problem.start)
    assert problem.simulations - before == 3

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


def test_a_fit_reports_every_run_of_it_as_it_finishes(decay, data):
    reported = []

    answer = decay.fit(
        fit_request(data, starts=[[0.02], [3.0], [9.0], [5.0]]),
        threads=2,
        on_run=lambda index, run: reported.append((index, run["cost"])),
    )

    assert sorted(reported) == [
        (index, run["cost"]) for index, run in enumerate(answer["runs"])
    ]


def test_a_fit_whose_report_raises_is_stopped_by_it(decay, data):
    def refuse(index, run):
        """Refuse the first run of the fit, as a caller which cannot store it would."""
        raise RuntimeError(f"the run {index} is no run of mine")

    with pytest.raises(RuntimeError, match="the run 0 is no run of mine"):
        decay.fit(
            fit_request(data, starts=[[0.02], [3.0], [9.0], [5.0]]),
            threads=1,
            on_run=refuse,
        )


@pytest.fixture(scope="module")
def fast(decay: sbmlsim_core.LoadedModel) -> sbmlsim_core.LoadedModel:
    """A second model, the decay of twice the rate: `y' = -2 k y`."""
    sbml = (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    doubled = sbml.replace(
        "<apply><times/><apply><minus/><ci>k</ci></apply><ci>y</ci></apply>",
        "<apply><times/><apply><minus/><cn>2</cn></apply><ci>k</ci><ci>y</ci></apply>",
    )
    assert doubled != sbml, "the rate rule of the decay model was not doubled"
    compiled = sbmlsim_core.compile_sbml(doubled)
    return sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)


def models_request(
    slow_data: tuple[list[float], list[float]],
    fast_data: tuple[list[float], list[float]],
    **over: Any,
) -> dict[str, Any]:
    """The fit of `k` against the data of both models, each condition on its model."""
    return {
        "contract": sbmlsim_core.FIT_CONTRACT,
        "parameters": [
            {"id": "k", "start": 3.0, "lower": 0.01, "upper": 10.0, "scale": "log10"}
        ],
        "conditions": [
            {"id": "slow", "model": "slow", "request": request(end=10.0, steps=10)},
            {"id": "fast", "model": "fast", "request": request(end=10.0, steps=10)},
        ],
        "curves": [
            {
                "id": "y of the slow decay",
                "condition": "slow",
                "column": "y",
                "times": slow_data[0],
                "values": slow_data[1],
            },
            {
                "id": "y of the fast decay",
                "condition": "fast",
                "column": "y",
                "times": fast_data[0],
                "values": fast_data[1],
            },
        ],
        "optimizer": {"max_iterations": 50},
    } | over


def test_a_fit_of_two_models_finds_the_parameter_of_the_data_of_both(decay, fast, data):
    fast_data = simulate(fast, end=10.0, steps=10)

    answer = sbmlsim_core.fit_models(
        {"slow": decay, "fast": fast},
        models_request(data, fast_data),
        threads=2,
    )

    assert answer["status"] == "SolverConverged"
    assert answer["parameters"][0]["value"] == pytest.approx(0.3, rel=1e-6)
    assert answer["cost"] == pytest.approx(0.0, abs=1e-12)


def test_every_condition_of_a_fit_is_simulated_on_the_model_it_names(decay, fast, data):
    fast_data = simulate(fast, end=10.0, steps=10)
    swapped = {"slow": fast, "fast": decay}

    problem = sbmlsim_core.fit_problem_of_models(
        swapped, models_request(data, fast_data)
    )

    residuals = problem.residuals(problem.start)
    assert numpy.abs(residuals).max() > 1e-3


def test_the_problem_of_a_fit_of_two_models_is_the_one_of_its_data(decay, fast, data):
    fast_data = simulate(fast, end=10.0, steps=10)
    models = {"slow": decay, "fast": fast}
    request_of = models_request(data, fast_data)
    request_of["parameters"][0]["start"] = 0.3

    problem = sbmlsim_core.fit_problem_of_models(models, request_of, threads=2)

    assert problem.n_residuals == 22
    # the start is the value of the data, on a logarithmic scale and back
    assert numpy.abs(problem.residuals(problem.start)).max() < 1e-14


def test_a_condition_which_names_a_model_the_fit_has_not_is_refused(decay, fast, data):
    fast_data = simulate(fast, end=10.0, steps=10)
    request_of = models_request(data, fast_data)
    request_of["conditions"][1]["model"] = "female"

    with pytest.raises(ValueError, match="'female'"):
        sbmlsim_core.fit_models({"slow": decay, "fast": fast}, request_of)


def test_a_condition_of_a_fit_of_several_models_names_one_of_them(decay, fast, data):
    fast_data = simulate(fast, end=10.0, steps=10)
    request_of = models_request(data, fast_data)
    del request_of["conditions"][0]["model"]

    with pytest.raises(ValueError, match="names no model"):
        sbmlsim_core.fit_models({"slow": decay, "fast": fast}, request_of)


def test_a_fit_of_no_model_is_refused(data):
    with pytest.raises(ValueError, match="at least one model"):
        sbmlsim_core.fit_models({}, fit_request(data))


def test_the_models_of_a_fit_are_loaded_models(decay, data):
    with pytest.raises(ValueError, match="no loaded model"):
        sbmlsim_core.fit_models({"decay": "a model"}, fit_request(data))
