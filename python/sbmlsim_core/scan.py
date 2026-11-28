"""The scan of sbmlsim planned in Rust (since 0.43.0).

A scan is a simulation and its dimensions with the observables of the run, the request
`scan/1` (sbmlsim's `Scan.to_dict()` and `Observable.to_dict()` with the options of
`Simulator.run`). The planner of the crate `sbmlsim-scan` turns it into the requests of
`simulation/1` of the engine, folds the formulas at the changes, evaluates the observables and
assembles the result `scanresult/1`, as sbmlsim's `Simulator.run` does:

```python
import sbmlsim_core
from sbmlsim_core import scan

compiled = sbmlsim_core.compile_sbml(sbml)
model = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)
result = scan.run(request, model, threads=4)
for variable in result["variables"]:
    print(variable["name"], variable["unit"], variable["values"].shape)
```

`compile_plans` gives the plans `plan/1` of a request instead of running it: every simulation
of the scan compiled against every model, with the values of the dimensions in the units of
the model, for a host which simulates the plans itself.

`run` drives the rounds of the planner on a batch executor of the model; `Scan` is the planner
itself, asked for its rounds (`ask`) and told their answers (`tell`, `tell_observed`), for a
host which simulates elsewhere. A scan with a dimension of models (sbmlsim's
`Dimension(models=...)`) takes the loaded model of every key of the dimension instead of the
model of the run: `scan.run(request, None, models={"fast": fast, "slow": slow})`. The arrays of
the result are arrays of numpy of float64 in the shape of their variable, `NaN` and the
infinities as the values they are. See the page Scans of the documentation for the contracts.
"""

from __future__ import annotations

import contextlib
import json
import math
from collections.abc import Callable, Iterable, Mapping
from typing import Any

from sbmlsim_core._core import (
    PLAN_CONTRACT,
    SCAN_CONTRACT,
    SCAN_RESULT_CONTRACT,
    CompiledModel,
    LoadedModel,
    PointError,
    ScanError,
    ScanPlanner,
    SimulationOutput,
)
from sbmlsim_core._core import compile_plans as _compile_plans

__all__ = [
    "PLAN_CONTRACT",
    "SCAN_CONTRACT",
    "SCAN_RESULT_CONTRACT",
    "Scan",
    "ScanError",
    "compile_plans",
    "request_text",
    "run",
]


def _plain(value: Any) -> Any:
    """A value of numpy as the value of python `json` writes (`tolist`)."""
    tolist = getattr(value, "tolist", None)
    if callable(tolist):
        return tolist()
    raise TypeError(f"an object of the type {type(value).__name__} is no value of JSON")


def _numbers(value: Any) -> Any:
    """A value with every number which is not finite as the text of the request.

    `NaN`, `inf` and `-inf` are `"NaN"`, `"inf"` and `"-inf"`, since JSON has no such numbers;
    arrays and numbers of numpy are their lists and numbers.
    """
    if isinstance(value, float):
        if math.isfinite(value):
            return value
        if math.isnan(value):
            return "NaN"
        return "inf" if value > 0 else "-inf"
    if isinstance(value, str | int | bool) or value is None:
        return value
    if isinstance(value, Mapping):
        return {key: _numbers(item) for key, item in value.items()}
    if isinstance(value, list | tuple):
        return [_numbers(item) for item in value]
    return _numbers(_plain(value))


def request_text(request: Mapping[str, Any] | str) -> str:
    """The JSON text of a request `scan/1`, its mappings in their order.

    A text is the text; a mapping is written with `json.dumps`, which keeps the order of
    every mapping (the pre-initialization of several targets, the simulations of a
    dimension), the order the planner reads them in; arrays and numbers of numpy are written
    as their lists and numbers, and a number which is not finite as `"NaN"`, `"inf"` or
    `"-inf"`, as the planner reads it.
    """
    if isinstance(request, str):
        return request
    return json.dumps(_numbers(request), allow_nan=False)


def compile_plans(
    request: Mapping[str, Any] | str,
    model: CompiledModel | LoadedModel | Mapping[str, Any] | None = None,
    *,
    models: Mapping[str, CompiledModel | LoadedModel | Mapping[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    """The plans of a scan, `plan/1`: every simulation compiled against every model.

    The request is a request `scan/1` (a mapping or its JSON text), of which only `scan`,
    `changes` and `model_changes` decide the plans, so `{"contract": "scan/1", "scan": ...}`
    is one. A model is a `CompiledModel` (nothing has to be loaded to plan), a `LoadedModel`
    or the description `model/2` as a dict; a scan with a dimension of models takes the model of
    every key of the dimension (`models`) instead of the model of the run.

    Returns one plan per simulation and model, every model of a simulation and then the next
    simulation, each `{"simulation", "model", "plan", "values", "at"}` as JSON types: the
    numbers of JSON, `NaN` `None` and the infinities `"inf"` and `"-inf"`, see the page Scans of
    the documentation.

    Raises `ScanError` (`kind` `request` or `units`) for a request which does not fit itself
    or its models.
    """
    text = _compile_plans(
        request_text(request), model, None if models is None else dict(models)
    )
    return json.loads(text)["plans"]


class Scan:
    """A scan planned against a loaded model, asked for its rounds and told their answers.

    ```python
    planner = scan.Scan(request, model)
    while True:
        kind, question = planner.ask()
        if kind == "simulate":
            planner.tell(model.simulate_many(question))
        elif kind == "observe":
            planner.tell_observed(observe(question))
        else:
            break
    result = planner.result()
    ```

    A scan with a dimension of models is planned against the loaded model of every key of the
    dimension (`models`) instead of the model of the run (`model` is `None`), and
    `round_model` is the key of the model of a round of simulations.

    Raises `ScanError` (`kind` `request`, `units` or `unsupported`) for a request which does
    not fit itself or its models; every check which needs no simulation runs here.
    """

    def __init__(
        self,
        request: Mapping[str, Any] | str,
        model: LoadedModel | None,
        *,
        models: Mapping[str, LoadedModel] | None = None,
    ) -> None:
        """Plans the scan of a request (a mapping or its JSON text) against its models."""
        self._planner = ScanPlanner(
            request_text(request), model, None if models is None else dict(models)
        )

    def ask(self) -> tuple[str, Any]:
        """The next round of the scan.

        `("simulate", requests)`: the requests of `simulation/1` (dicts) of a batch, which
        `tell` answers. `("observe", question)`: the observables of the host (sbmlsim's
        `Custom`) of the points of a chunk, `{"observables": [id, ...], "points": [{"index":
        flat index, "time": array, "columns": {symbol: array}}, ...], "scalars": [symbol,
        ...]}` (`scalars` the symbols which are a value per simulation, one value), which
        `tell_observed` answers. `("done", None)`: the scan is done (or failed, which
        `result` raises). A round which was asked and not told is asked again. The requests of
        a round of simulations are of the model `round_model`.
        """
        kind, question = self._planner.ask()
        return kind, question

    @property
    def round_model(self) -> str | None:
        """The model of the last round of simulations `ask` gave.

        Its key in the dimension of models, `None` for the model of the run.
        """
        return self._planner.round_model

    def tell(self, results: Iterable[SimulationOutput | PointError]) -> None:
        """Takes the answers of a round of simulations, one per request in their order.

        The answers are what `simulate_many` of the model or of a batch executor gives.

        Raises `ScanError` of the kind `point` when a point of a scan of `on_error: raise`
        failed, and of the kind `request` for answers which do not fit the round.
        """
        self._planner.tell(results)

    def tell_observed(self, values: Mapping[str, Any]) -> None:
        """Takes the answer of a round of the observables of the host.

        The answer is `{"values": {id: [value of every point]}, "errors": [...]}`. The
        value of a point is a number for an observable of the kind `scalar` and a value per
        time point for one of the kind `timecourse`, in the order of the points of the
        question; `errors` (optional) are the functions which failed, `{"observable": id,
        "index": flat index, "message": "ValueError: ..."}`, whose points fail alone.
        """
        self._planner.tell_observed(values)

    def finished_models(self) -> list[str]:
        """The keys of the models of a dimension of models whose points have all run.

        A host which holds something per model, e.g. an executor, may drop it.
        """
        return self._planner.finished_models()

    @property
    def progress(self) -> tuple[int, int]:
        """The points of the block which ran and the points of the block."""
        return self._planner.progress

    def result(self) -> dict[str, Any]:
        """The result `scanresult/1` once `ask` says the scan is done; a scan gives it once.

        A dict of the fields of the contract: `sizes` (`[{"name", "size"}]`), `dims`,
        `layout` (`grid` or `ragged`), `coords` (`[{"name", "dims", "unit", "values"}]`, the
        values `{"numbers": array}`, `{"labels": [...]}` or `{"indices": array}`),
        `variables` (`[{"name", "dims", "shape", "unit", "values"}]`, the values an array of
        float64 of the shape), `units` (pairs of the name and the unit, in the order of
        sbmlsim's attribute), `changes` (an array), `errors` (`[{"index", "point",
        "message"}]` of a scan of `on_error: flag`, else `None`) and `block` (of a run of
        `points`, else `None`).

        Raises the `ScanError` of a scan which failed, and one of the kind `request` for a
        scan which is not done.
        """
        return self._planner.result()


def run(
    request: Mapping[str, Any] | str,
    model: LoadedModel | None,
    *,
    models: Mapping[str, LoadedModel] | None = None,
    threads: int | None = None,
    observe: Callable[[dict[str, Any]], Mapping[str, Any]] | None = None,
    on_batch: Callable[[int, int], None] | None = None,
) -> dict[str, Any]:
    """Runs a scan on a batch executor of the model and gives its result `scanresult/1`.

    The rounds of simulations run on `model.batch_executor(threads=threads)` without their
    requests becoming objects of python; `threads=None` is the default of the executor (every
    CPU of the process, at most eight). Every worker of the executor simulates on an instance
    of its own, which it keeps across the rounds, and a request carries everything of its
    simulation, so the result does not depend on `threads`. `observe(question)` computes the observables of the
    host (sbmlsim's `Custom`) and answers as `Scan.tell_observed` takes it; a scan which has
    such observables needs it. `on_batch(done, total)` is called with the points of the
    block which ran whenever a batch of them finished, e.g. for a progress bar.

    A scan with a dimension of models takes the loaded model of every key of the dimension
    (`models`) instead of the model of the run (`model` is `None`); the requests of a model run
    on a batch executor of that model, opened at the first round of the model and closed once
    every point of the model ran, so at most the executors of the models whose rounds
    alternate (a dimension of models after a dimension of values) are open at once.

    Raises `ScanError`, see `Scan`.
    """
    loaded_models = {} if models is None else dict(models)
    planner = ScanPlanner(request_text(request), model, loaded_models or None)
    executors: dict[str, Any] = {}
    with contextlib.ExitStack() as stack:
        executor = (
            None
            if model is None
            else stack.enter_context(model.batch_executor(threads=threads))
        )
        while True:
            kind, question = planner.advance(
                executor, on_batch, executors, observe is not None
            )
            if kind == "model":
                for key in planner.finished_models():
                    finished = executors.pop(key, None)
                    if finished is not None:
                        finished.close()
                # closed at the end as well, each on its own (closing twice does nothing)
                executors[question] = stack.enter_context(
                    loaded_models[question].batch_executor(threads=threads)
                )
                continue
            if kind == "done":
                break
            assert observe is not None  # the planner raises a round without it
            planner.tell_observed(observe(question))
    return planner.result()
