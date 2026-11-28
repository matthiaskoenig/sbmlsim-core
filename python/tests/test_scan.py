"""Tests of `sbmlsim_core.scan`, the planner of a scan driven from python.

Every oracle fixture of `testdata/scan` (written by sbmlsim from its own runs of the engine
core) runs end to end through `scan.run`: the request of the fixture, the model compiled again
from its SBML, the simulations on a batch executor and the observables of the host in python,
compared with the `result` of the fixture under the rule of `crates/scan/tests/fixtures.rs`:
the native values by their bits, the values interpolated onto a grid and the observables
within 4 units in the last place, `NaN` equal to `NaN`.
"""

import json
import math
from collections.abc import Callable
from functools import cache
from pathlib import Path
from typing import Any

import numpy as np
import pytest
import sbmlsim_core
from sbmlsim_core import scan

ROOT = Path(__file__).resolve().parents[2]
TESTDATA = ROOT / "testdata" / "scan"
CASES = sorted(path.stem for path in TESTDATA.glob("*.json"))

#: The units in the last place a derived value (a grid, an observable) may differ by.
ULPS = 4.0


def fixture(case: str) -> dict[str, Any]:
    """The fixture of a case, its mappings in the order of their text."""
    return json.loads((TESTDATA / f"{case}.json").read_text())


@cache
def model(name: str) -> sbmlsim_core.LoadedModel:
    """The model of a fixture, compiled again from its SBML and loaded once."""
    compiled = sbmlsim_core.compile_sbml((TESTDATA / "models" / name).read_text())
    return runtime().load(compiled.wasm, compiled.description)


@cache
def runtime() -> sbmlsim_core.Runtime:
    """The runtime of the tests."""
    return sbmlsim_core.Runtime()


def auc_of_c(time: np.ndarray, values: dict[str, np.ndarray]) -> float:
    """`scripts.scan_fixtures:auc_of_c` of sbmlsim."""
    return float(np.trapezoid(values["[C]"], time))


def doubled_c(time: np.ndarray, values: dict[str, np.ndarray]) -> np.ndarray:
    """`scripts.scan_fixtures:doubled_c` of sbmlsim."""
    return 2.0 * values["[C]"]


FUNCTIONS: dict[str, Callable[..., Any]] = {
    "scripts.scan_fixtures:auc_of_c": auc_of_c,
    "scripts.scan_fixtures:doubled_c": doubled_c,
}


def observer(request: dict[str, Any]) -> Callable[[dict[str, Any]], dict[str, Any]]:
    """The host of the custom observables of a request, as sbmlsim calls them."""
    functions = {
        o["id"]: FUNCTIONS[o["function"]]
        for o in request["observables"]
        if o["type"] == "Custom"
    }

    def observe(question: dict[str, Any]) -> dict[str, Any]:
        values = {
            oid: [functions[oid](p["time"], p["columns"]) for p in question["points"]]
            for oid in question["observables"]
        }
        return {"values": values}

    return observe


def number(value: Any) -> float:
    """A number of a fixture: `null` is NaN, `"inf"` and `"-inf"` the infinities."""
    if value is None:
        return float("nan")
    if isinstance(value, str):
        return float(value)
    return float(value)


def numbers(values: list[Any]) -> np.ndarray:
    return np.array([number(v) for v in values], dtype=np.float64)


def ulps(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    """The distance of the values in units in the last place of the larger.

    It is 0 for equal bits and two NaN, infinite for a NaN or an infinity against another
    value.
    """
    a = np.asarray(a, dtype=np.float64).ravel()
    b = np.asarray(b, dtype=np.float64).ravel()
    same = (a.view(np.uint64) == b.view(np.uint64)) | (np.isnan(a) & np.isnan(b))
    with np.errstate(invalid="ignore", over="ignore"):
        larger = np.maximum(np.abs(a), np.abs(b))
        d = np.abs(a - b) / np.spacing(larger)
    d[~(np.isfinite(a) & np.isfinite(b))] = np.inf
    d[same] = 0.0
    return d


def compare(
    case: str, result: dict[str, Any], expected: dict[str, Any], request: dict
) -> None:
    """Compares a result with the `result` of its fixture, see the module."""
    observed = [o["id"] for o in request["observables"]]
    gridded = request["time"] is not None

    def derived(name: str) -> bool:
        return gridded or any(name == i or name.startswith(f"{i}.") for i in observed)

    assert result["contract"] == "scanresult/1"
    assert [(s["name"], s["size"]) for s in result["sizes"]] == [
        tuple(s) for s in expected["sizes"]
    ]
    attrs = expected["attrs"]
    assert result["dims"] == attrs["dims"]
    assert [c["name"] for c in result["coords"]] == expected["order"]["coords"]
    assert [v["name"] for v in result["variables"]] == expected["order"]["variables"]
    for coord in result["coords"]:
        want = expected["coords"].get(coord["name"])
        if want is None:
            continue
        assert coord["dims"] == want["dims"], coord["name"]
        assert coord["unit"] == want["unit"], coord["name"]
        ((kind, values),) = coord["values"].items()
        if kind == "numbers":
            assert isinstance(values, np.ndarray) and values.dtype == np.float64
            assert ulps(values, numbers(want["values"])).max(initial=0.0) == 0.0
        elif kind == "indices":
            assert values.tolist() == want["values"]
        else:
            assert values == want["values"], coord["name"]
    for variable in result["variables"]:
        want = expected["variables"].get(variable["name"])
        if want is None:
            continue
        name = variable["name"]
        values = variable["values"]
        assert isinstance(values, np.ndarray) and values.dtype == np.float64, name
        assert variable["dims"] == want["dims"], name
        assert list(variable["shape"]) == want["shape"], name
        assert values.shape == tuple(want["shape"]), name
        assert variable["unit"] == want["unit"], name
        distance = ulps(values, numbers(want["values"])).max(initial=0.0)
        allowed = ULPS if derived(name) else 0.0
        assert distance <= allowed, f"{case}: {name} differs by {distance} ulps"
    assert [tuple(pair) for pair in result["units"]] == list(attrs["units"].items())
    assert ulps(result["changes"], numbers(attrs["changes"])).max(initial=0.0) == 0.0
    texts = None
    if result["errors"] is not None:
        texts = [
            f"{e['point']}: {e['message']}" if e["point"] else e["message"]
            for e in result["errors"]
        ]
    assert texts == attrs.get("errors")
    block = result["block"]
    if "block" in attrs:
        want = attrs["block"]
        assert block is not None
        assert (block["start"], block["stop"]) == (want["start"], want["stop"])
        assert list(block["shape"]) == want["shape"]
        assert block["dims"] == want["dims"]
        assert [tuple(pair) for pair in block["coords"]] == list(want["coords"].items())
    else:
        assert block is None


@pytest.mark.parametrize("case", CASES)
def test_fixtures(case: str) -> None:
    f = fixture(case)
    request = f["request"]
    loaded = model(f["model"])
    if f["error"] is not None:
        with pytest.raises(sbmlsim_core.ScanError) as error:
            scan.run(request, loaded, observe=observer(request))
        assert f["error"]["type"] == "ScanError"
        assert error.value.kind == "point"
        assert str(error.value) == f["error"]["message"]
        assert isinstance(error.value.point, int)
        return
    result = scan.run(request, loaded, observe=observer(request))
    compare(case, result, f["result"], request)


def test_there_are_fixtures() -> None:
    assert len(CASES) == 25


def same_result(a: dict[str, Any], b: dict[str, Any]) -> None:
    """Two results are the same, every array by its bits."""
    assert [v["name"] for v in a["variables"]] == [v["name"] for v in b["variables"]]
    for x, y in zip(a["variables"], b["variables"], strict=True):
        assert x["values"].tobytes() == y["values"].tobytes(), x["name"]
    assert a["changes"].tobytes() == b["changes"].tobytes()
    assert a["errors"] == b["errors"]


@pytest.mark.parametrize("case", ["scan_2d_values", "failing_point", "pk_oral"])
def test_the_threads_change_no_bit(case: str) -> None:
    f = fixture(case)
    loaded = model(f["model"])
    one = scan.run(f["request"], loaded, threads=1)
    four = scan.run(f["request"], loaded, threads=4)
    same_result(one, four)


def test_ask_and_tell_is_the_run() -> None:
    f = fixture("custom_observable")
    request = f["request"]
    loaded = model(f["model"])
    planner = scan.Scan(request, loaded)
    observe = observer(request)
    rounds = {"simulate": 0, "observe": 0}
    while True:
        kind, question = planner.ask()
        if kind == "done":
            assert question is None
            break
        rounds[kind] += 1
        if kind == "simulate":
            assert all(r["contract"] == "simulation/1" for r in question)
            planner.tell(loaded.simulate_many(question, threads=2))
        else:
            assert set(question) == {"observables", "points", "scalars"}
            assert question["scalars"] == []
            point = question["points"][0]
            assert isinstance(point["time"], np.ndarray)
            assert all(isinstance(c, np.ndarray) for c in point["columns"].values())
            planner.tell_observed(observe(question))
    assert rounds["simulate"] >= 1 and rounds["observe"] >= 1
    same_result(planner.result(), scan.run(request, loaded, observe=observe))


def test_a_request_as_json_text_is_the_dict() -> None:
    f = fixture("scan_simulations")
    loaded = model(f["model"])
    same_result(
        scan.run(json.dumps(f["request"]), loaded), scan.run(f["request"], loaded)
    )


def test_numpy_values_in_a_request_are_numbers() -> None:
    f = fixture("scan_2d_values")
    request = json.loads(json.dumps(f["request"]))
    dimension = request["scan"]["dimensions"][0]
    target = next(iter(dimension["values"]))
    values = dimension["values"][target]
    if isinstance(values, dict):
        values["value"] = np.array(values["value"])
    else:
        dimension["values"][target] = np.array(values)
    loaded = model(f["model"])
    same_result(scan.run(request, loaded), scan.run(f["request"], loaded))


def test_on_batch_reports_the_points_which_ran() -> None:
    f = fixture("points_block")
    calls: list[tuple[int, int]] = []
    scan.run(
        f["request"],
        model(f["model"]),
        on_batch=lambda done, total: calls.append((done, total)),
    )
    assert calls, "no batch was reported"
    assert calls[-1][0] == calls[-1][1] == 3
    assert [d for d, _ in calls] == sorted(d for d, _ in calls)


def test_a_request_which_does_not_fit_the_model_is_a_scan_error() -> None:
    f = fixture("steps_plain")
    request = json.loads(json.dumps(f["request"]))
    request["scan"]["dimensions"] = [
        {
            "id": "d",
            "kind": "values",
            "labels": [0],
            "values": {"no_such_entity": [1.0]},
            "simulations": {},
            "models": {},
            "at": None,
            "design": None,
            "coordinates": {},
        }
    ]
    with pytest.raises(sbmlsim_core.ScanError) as error:
        scan.Scan(request, model(f["model"]))
    assert error.value.kind == "request"
    assert error.value.point is None
    assert "no_such_entity" in str(error.value)
    assert isinstance(error.value, sbmlsim_core.SbmlsimCoreError)


def test_custom_observables_need_an_observe_function() -> None:
    f = fixture("custom_observable")
    with pytest.raises(sbmlsim_core.ScanError) as error:
        scan.run(f["request"], model(f["model"]))
    assert error.value.kind == "request"
    assert "observe" in str(error.value)


def test_a_failed_host_function_fails_its_point_alone() -> None:
    f = fixture("custom_observable")
    request = json.loads(json.dumps(f["request"]))
    request["on_error"] = "flag"
    observe = observer(request)

    def failing(question: dict[str, Any]) -> dict[str, Any]:
        answer = observe(question)
        first = question["points"][0]["index"]
        answer["errors"] = [
            {
                "observable": question["observables"][0],
                "index": first,
                "message": "ValueError: no",
            }
        ]
        return answer

    result = scan.run(request, model(f["model"]), observe=failing)
    status = next(v for v in result["variables"] if v["name"] == "status")
    assert status["values"].tolist() == [1.0, 0.0]
    assert result["errors"][0]["message"].endswith(": ValueError: no")


def test_a_scan_told_out_of_turn_is_a_request_error() -> None:
    f = fixture("steps_plain")
    planner = scan.Scan(f["request"], model(f["model"]))
    with pytest.raises(sbmlsim_core.ScanError) as error:
        planner.tell([])
    assert error.value.kind == "request"
    with pytest.raises(sbmlsim_core.ScanError) as error:
        planner.result()
    assert error.value.kind == "request"


def fold_model(old: str = "", new: str = "") -> sbmlsim_core.LoadedModel:
    """The model of the fold fixtures, with a text of its SBML replaced."""
    sbml = (TESTDATA / "models" / "scan_fold.xml").read_text().replace(old, new)
    compiled = sbmlsim_core.compile_sbml(sbml)
    return runtime().load(compiled.wasm, compiled.description)


def models_request(keys: list[str]) -> dict[str, Any]:
    """A scan of the fold model over `kb` and a dimension of models of `keys`."""
    request = json.loads(json.dumps(fixture("failing_point_raise")["request"]))
    request["scan"]["dimensions"] = [
        {
            "id": "rate",
            "kind": "values",
            "labels": [0, 1],
            "values": {"kb": [0.0, 0.1]},
            "simulations": {},
            "models": {},
            "at": None,
            "design": None,
            "coordinates": {},
        },
        {
            "id": "model",
            "kind": "models",
            "labels": keys,
            "values": {},
            "simulations": {},
            "models": dict.fromkeys(keys, "<sbml/>"),
            "at": None,
            "design": None,
            "coordinates": {},
        },
    ]
    return request


def test_a_dimension_of_models_runs_every_model() -> None:
    fast = fold_model()
    slow = fold_model('id="k" value="0.2"', 'id="k" value="0.4"')
    models = {"slow": slow, "fast": fast}
    request = models_request(["fast", "slow"])
    result = scan.run(request, None, models=models, threads=2)
    assert result["dims"] == ["rate", "model"]
    alone = {}
    for key, loaded in models.items():
        single = json.loads(json.dumps(request))
        single["scan"]["dimensions"] = single["scan"]["dimensions"][:1]
        alone[key] = scan.run(single, loaded)
    for variable in result["variables"]:
        for m, key in enumerate(["fast", "slow"]):
            want = next(
                v for v in alone[key]["variables"] if v["name"] == variable["name"]
            )
            got = variable["values"][:, m]
            assert got.tobytes() == want["values"].tobytes(), (variable["name"], key)
    planner = scan.Scan(request, None, models=models)
    rounds = []
    while True:
        kind, question = planner.ask()
        if kind == "done":
            break
        key = planner.round_model
        assert key is not None
        rounds.append(key)
        planner.tell(models[key].simulate_many(question))
    assert rounds == ["fast", "slow"]
    same_result(planner.result(), result)


def test_a_dimension_of_models_needs_its_models() -> None:
    request = models_request(["fast", "slow"])
    with pytest.raises(sbmlsim_core.ScanError, match="replaces the model of the run"):
        scan.run(request, fold_model(), models={"fast": fold_model()})
    with pytest.raises(
        sbmlsim_core.ScanError, match=r"the host gave the models \['fast'\]"
    ):
        scan.run(request, None, models={"fast": fold_model()})
    with pytest.raises(sbmlsim_core.ScanError, match="needs a model"):
        scan.run(fixture("failing_point_raise")["request"], None)
    with pytest.raises(TypeError, match="LoadedModel"):
        scan.run(request, None, models={"fast": fold_model(), "slow": 1.0})  # ty: ignore[invalid-argument-type]


def test_the_executors_of_the_models_are_opened_and_closed_by_the_rounds() -> None:
    from sbmlsim_core._core import ScanPlanner

    fast = fold_model()
    slow = fold_model('id="k" value="0.2"', 'id="k" value="0.4"')
    request = models_request(["fast", "slow"])
    # the dimension of models first: every point of `fast`, then every point of `slow`
    request["scan"]["dimensions"].reverse()
    planner = ScanPlanner(
        scan.request_text(request), None, {"fast": fast, "slow": slow}
    )
    assert planner.advance(None, None, {}) == ("model", "fast")
    assert planner.finished_models() == []
    with fast.batch_executor(threads=1) as one:
        assert planner.advance(None, None, {"fast": one}) == ("model", "slow")
        assert planner.finished_models() == ["fast"]
    with slow.batch_executor(threads=1) as two:
        assert planner.advance(None, None, {"slow": two}) == ("done", None)
    assert planner.finished_models() == ["fast", "slow"]
    lazy = scan.run(request, None, models={"fast": fast, "slow": slow})
    same_result(planner.result(), lazy)


@pytest.mark.parametrize("case", CASES)
def test_the_plans_of_a_fixture(case: str) -> None:
    written = fixture(case)
    sbml = (TESTDATA / "models" / written["model"]).read_text()
    compiled = sbmlsim_core.compile_sbml(sbml)
    plans = scan.compile_plans(written["request"], compiled)
    assert scan.PLAN_CONTRACT == "plan/1"
    assert plans
    # a compiled model, a loaded model and a description plan alike
    assert scan.compile_plans(written["request"], model(written["model"])) == plans
    assert scan.compile_plans(written["request"], compiled.description) == plans
    dimensions = written["request"]["scan"]["dimensions"]
    for entry in plans:
        assert set(entry) == {"simulation", "model", "plan", "values", "at"}
        assert entry["model"] is None
        assert len(entry["values"]) == len(entry["at"]) == len(dimensions)
        plan = entry["plan"]
        assert plan["output"] in {"integrator", "times"}
        assert plan["start"] < plan["end"]
        for dimension, values in zip(dimensions, entry["values"], strict=True):
            if dimension["kind"] == "values":
                assert [v["target"] for v in values] == list(dimension["values"])


def test_the_plans_of_steps_and_of_values() -> None:
    written = fixture("scan_at")
    request = written["request"]
    plans = scan.compile_plans(request, model(written["model"]))
    plan = plans[0]["plan"]
    simulation = request["scan"]["simulation"]
    if simulation["steps"] is not None:
        assert len(plan["times"]) == simulation["steps"] + 1
    dimension = request["scan"]["dimensions"][0]
    assert plans[0]["at"] == [dimension["at"]]
    target = next(iter(dimension["values"]))
    assert plans[0]["values"][0][0]["target"] == target


def test_the_plans_of_a_dimension_of_models() -> None:
    request = models_request(["fast", "slow"])
    request["model_changes"] = {"slow": {"k": 0.4}}
    plans = scan.compile_plans(
        request, models={"fast": fold_model(), "slow": fold_model()}
    )
    assert [entry["model"] for entry in plans] == ["fast", "slow"]
    assert plans[1]["plan"]["preinit"] == [
        {"target": "k", "kind": "parameter", "value": 0.4}
    ]
    assert plans[0]["values"][0] == [{"target": "kb", "values": [0.0, 0.1]}]


def test_plans_which_do_not_fit_the_model_are_a_scan_error() -> None:
    written = fixture("steps_plain")
    request = json.loads(json.dumps(written["request"]))
    request["scan"]["simulation"]["preinit_changes"] = {"nothing": 1.0}
    with pytest.raises(
        sbmlsim_core.ScanError, match="'nothing' is not a parameter"
    ) as err:
        scan.compile_plans(request, model(written["model"]))
    assert err.value.kind == "request"
    with pytest.raises(TypeError, match="no CompiledModel, LoadedModel or description"):
        scan.compile_plans(request, 1.0)  # ty: ignore[invalid-argument-type]


def test_a_value_which_is_not_finite_is_a_number_of_the_plan() -> None:
    written = fixture("steps_plain")
    request = json.loads(json.dumps(written["request"]))
    simulation = request["scan"]["simulation"]
    target = "KM"
    simulation["preinit_changes"] = {target: "inf"}
    simulation["changes"] = [{"times": [1.0], "values": {target: "-inf"}}]
    plan = scan.compile_plans(request, model(written["model"]))[0]["plan"]
    assert plan["preinit"] == [{"target": target, "kind": "parameter", "value": "inf"}]
    assignment = plan["events"][0]["assignments"][0]
    assert assignment == {"target": target, "kind": "parameter", "value": "-inf"}


def test_a_float_which_is_not_finite_is_written_as_the_planner_reads_it() -> None:
    text = scan.request_text(
        {"a": [1.0, math.nan, math.inf, -math.inf], "b": np.array([np.inf, 2.0])}
    )
    assert json.loads(text) == {"a": [1.0, "NaN", "inf", "-inf"], "b": ["inf", 2.0]}
    written = fixture("steps_plain")
    request = json.loads(json.dumps(written["request"]))
    request["scan"]["simulation"]["preinit_changes"] = {"KM": math.inf}
    plan = scan.compile_plans(request, model(written["model"]))[0]["plan"]
    assert plan["preinit"] == [{"target": "KM", "kind": "parameter", "value": "inf"}]


def test_a_failed_fold_fails_its_point_alone() -> None:
    """The segment which folds the change of the point `kb = 2` blows up before the change."""
    request = json.loads(json.dumps(fixture("failing_point")["request"]))
    request["scan"]["simulation"]["changes"] = [
        {"times": [1.0], "values": {"C": "[C] * [C] / 4"}}
    ]
    result = scan.run(request, fold_model())
    status = next(v for v in result["variables"] if v["name"] == "status")
    assert status["values"].tolist() == [0.0, 1.0, 0.0]
    assert [e["index"] for e in result["errors"]] == [1]
    # the other points are the ones of a scan without the failing point
    request["scan"]["dimensions"][0]["labels"] = [0, 2]
    request["scan"]["dimensions"][0]["values"] = {"kb": [0.0, 0.1]}
    alone = scan.run(request, fold_model())
    for variable in result["variables"]:
        if variable["name"] == "status":
            continue
        other = next(v for v in alone["variables"] if v["name"] == variable["name"])
        assert np.array_equal(
            variable["values"][[0, 2]], other["values"], equal_nan=True
        ), variable["name"]
    request["on_error"] = "raise"
    request["scan"]["dimensions"][0]["labels"] = [0, 1, 2]
    request["scan"]["dimensions"][0]["values"] = {"kb": [0.0, 2.0, 0.1]}
    with pytest.raises(sbmlsim_core.ScanError) as error:
        scan.run(request, fold_model())
    assert error.value.kind == "point"
    assert error.value.point == 1


def test_the_question_names_the_values_per_simulation() -> None:
    f = fixture("custom_observable")
    request = json.loads(json.dumps(f["request"]))
    request["observables"] = [
        {"type": "Formula", "id": "cmax", "formula": "max([C])", "unit": None},
        {
            "type": "Custom",
            "id": "scaled",
            "function": "f",
            "unit": "dimensionless",
            "symbols": ["cmax", "[C]"],
            "kind": "timecourse",
        },
    ]
    planner = scan.Scan(request, model(f["model"]))
    while True:
        kind, question = planner.ask()
        if kind == "observe":
            break
        assert kind == "simulate"
        planner.tell(model(f["model"]).simulate_many(question))
    assert question["scalars"] == ["cmax"]
    point = question["points"][0]
    assert point["columns"]["cmax"].shape == (1,)
    assert point["columns"]["[C]"].shape == point["time"].shape
