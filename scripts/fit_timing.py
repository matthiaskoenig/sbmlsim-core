"""The wall time of a fit of a model in Rust against the objective in python.

    .venv/bin/python scripts/fit_timing.py [--conditions 20] [--parameters 10]
        [--threads N] [--rounds 5] [--iterations 10] [--profile] [--json FILE]

It imports the built extension, so it runs in the environment the package was
installed into (`uv pip install -e . --group dev`) and not with `uv run`, which
would resolve `sbmlsim_core` from an index.

`scripts/objective_timing.py` measures what one Jacobian of a fit costs on the
paths the python bindings offer, which all build their tables in python. This
script measures the Jacobian of the same fit of the same model inside the engine
(`LoadedModel.fit`, the contract `fit/1`), where the residuals of a point are
computed on the thread of the point and nothing of a host crosses a thread, and
the fit of that model end to end.

The model is hydrochlorothiazide (28 states) of `bench/sbmlsim.toml` and a
condition an oral dose over a horizon, the ones of `objective_timing.py`, whose
builders this script imports. The data of the fit is the model itself at
parameters a third above the ones it starts from, so the optimum is known and
the residuals are the ones of a fit which has something to find.

The rows:

- `plans, one batch (python)`: the whole Jacobian in one fan out
  (`BatchExecutor.simulate_plans`), the fastest path of the python bindings, with
  the residuals assembled in numpy as a fit of sbmlsim assembles them.
- `fit, in rust`: the same Jacobians inside the engine, measured over a fit of
  `--iterations` iterations and divided by the simulations the fit reports.
"""

import argparse
import json
import os
import statistics
import time
from pathlib import Path
from typing import Any

import numpy
import sbmlsim_core
from objective_timing import (
    DIFF_STEP,
    conditions,
    jacobian_points,
    model,
    observable,
    parameters,
    tolerance,
)

ROOT = Path(__file__).resolve().parents[1]
# the deviation of the parameters of the data from the ones the fit starts at
TRUTH = 1.3
# the standard deviation of a measurement, a share of the largest value of its curve
NOISE = 0.05


def curves(
    executor: sbmlsim_core.BatchExecutor,
    prepared: list[Any],
    truth: list[float],
    column: int,
) -> list[tuple[list[float], list[float]]]:
    """The data of the fit, the model itself at the true parameters.

    The times of a curve are the output times of its own condition, which is what a
    fit resolves them against: it interpolates nothing.
    """
    results = executor.simulate_plans([(one, truth) for one in prepared])
    data = []
    for at, result in enumerate(results):
        if isinstance(result, sbmlsim_core.PointError):
            raise SystemExit(f"the condition {at} does not simulate: {result.message}")
        time = result.columns.index("time")
        data.append(
            (
                [float(value) for value in result.values[:, time]],
                [float(value) for value in result.values[:, column]],
            )
        )
    return data


def residuals_in_python(
    executor: sbmlsim_core.BatchExecutor,
    prepared: list[Any],
    points: list[list[float]],
    data: numpy.typing.NDArray[numpy.float64],
    noise: numpy.typing.NDArray[numpy.float64],
    column: int,
) -> list[Any]:
    """One Jacobian through the python bindings: one batch, the residuals in numpy.

    The plans of the whole Jacobian are one fan out, and the residuals of every
    point are assembled here, which is where a fit of sbmlsim assembles them.
    """
    plans = [(one, values) for values in points for one in prepared]
    results = executor.simulate_plans(plans)
    columns = len(prepared)
    jacobian = numpy.empty((len(points), data.size))
    for at, result in enumerate(results):
        if isinstance(result, sbmlsim_core.PointError):
            raise SystemExit(f"a point of the jacobian failed: {result.message}")
        point, condition = divmod(at, columns)
        simulated = result.values[:, column]
        jacobian[point, condition * data.shape[1] : (condition + 1) * data.shape[1]] = (
            simulated - data[condition]
        ) / noise[condition]
    # the columns of the difference, as a least squares optimizer takes them
    for point in range(1, len(points)):
        jacobian[point] = (jacobian[point] - jacobian[0]) / DIFF_STEP
    return results


def fit_request(
    requests: list[dict[str, Any]],
    names: list[str],
    values: list[float],
    data: list[tuple[list[float], list[float]]],
    column_id: str,
    threads: int,
    iterations: int,
) -> dict[str, Any]:
    """The fit of the model as a request of the contract `fit/1`."""
    return {
        "contract": sbmlsim_core.FIT_CONTRACT,
        "parameters": [
            {
                "id": name,
                "start": value,
                "lower": value / 100.0,
                "upper": value * 100.0,
                "scale": "log10",
            }
            for name, value in zip(names, values, strict=True)
        ],
        "conditions": [
            {"id": f"condition {at}", "request": request}
            for at, request in enumerate(requests)
        ],
        "curves": [
            {
                "id": f"curve {at}",
                "condition": f"condition {at}",
                "column": column_id,
                "times": times,
                "values": curve,
                "noise": [max(NOISE * max(curve), 1e-12)] * len(curve),
            }
            for at, (times, curve) in enumerate(data)
        ],
        "optimizer": {"algorithm": "trust_region", "max_iterations": iterations},
        "threads": threads,
    }


def simulations_of(answer: dict[str, Any], conditions: int, k: int) -> int:
    """The simulations a fit ran: its evaluations and the columns of its Jacobians."""
    return conditions * (1 + answer["evaluations"] + answer["jacobians"] * (k + 1))


def main() -> None:
    """Measures the Jacobian of a fit in python and in rust and prints the table."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--conditions", type=int, default=20)
    parser.add_argument("--parameters", type=int, default=10)
    parser.add_argument("--threads", type=int, default=0)
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--iterations", type=int, default=10)
    parser.add_argument(
        "--profile",
        action="store_true",
        help="also walk a profile of the likelihood of the first parameter",
    )
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    threads = args.threads or len(os.sched_getaffinity(0))

    loaded, description = model()
    atol = tolerance(description)
    column_id = observable(description)
    requests = conditions(args.conditions, ["time", column_id], atol)
    fitted = parameters(description, args.parameters)
    names = [name for name, _ in fitted]
    values = [value for _, value in fitted]
    points = jacobian_points(values)
    truth = [value * TRUTH for value in values]

    with loaded.batch_executor(threads=threads) as executor:
        prepared = [loaded.prepare(request, varying=names) for request in requests]
        data = curves(executor, prepared, truth, 1)
        table = numpy.array([curve for _, curve in data], dtype=float)
        noise = numpy.array(
            [[max(NOISE * max(curve), 1e-12)] * len(curve) for _, curve in data],
            dtype=float,
        )
        request = fit_request(
            requests, names, values, data, column_id, threads, args.iterations
        )
        if args.profile:
            request["identifiability"] = {
                "parameters": [names[0]],
                "walk": {"points": 4, "span": 0.5},
                "fisher": True,
            }

        # one jacobian of the same fit at the same point, inside the engine
        objective = loaded.fit_problem(request, threads=threads)
        start = objective.start

        # the rounds take turns, so a load of the machine is spread over the rows
        python: list[float] = []
        rust: list[float] = []
        for round_at in range(args.rounds + 1):
            started = time.perf_counter()
            residuals_in_python(executor, prepared, points, table, noise, 1)
            took = time.perf_counter() - started
            if round_at:
                python.append(took)
            started = time.perf_counter()
            objective.jacobian(start)
            took = time.perf_counter() - started
            if round_at:
                rust.append(took)

        # and the fit itself once, which has a pool and instances of its own
        started = time.perf_counter()
        answer = loaded.fit(request, threads=threads)
        fit_seconds = time.perf_counter() - started

    k = len(names)
    simulations = simulations_of(answer, args.conditions, k)
    per_jacobian = (k + 1) * args.conditions
    # the fastest round of a path is the one a load of the machine disturbed least
    python_ms = min(python) * 1e3
    rust_ms = min(rust) * 1e3
    spread = (
        f"the spread of the rounds: python {min(python) * 1e3:.2f} to"
        f" {max(python) * 1e3:.2f} ms, rust {min(rust) * 1e3:.2f} to"
        f" {max(rust) * 1e3:.2f} ms, the median of each"
        f" {statistics.median(python) * 1e3:.2f} and"
        f" {statistics.median(rust) * 1e3:.2f} ms"
    )
    print(
        f"{args.conditions} conditions, {k} parameters, {threads} threads,"
        f" {args.rounds} rounds of {per_jacobian} simulations per jacobian"
    )
    print()
    print("| the jacobian of one step | wall time | ms per simulation | of python |")
    print("|---|---:|---:|---:|")
    print(
        f"| plans of one batch, the residuals in python | {python_ms:.2f} ms |"
        f" {python_ms / per_jacobian:.4f} | 1.00x |"
    )
    print(
        f"| the objective in rust | {rust_ms:.2f} ms |"
        f" {rust_ms / per_jacobian:.4f} | {python_ms / rust_ms:.2f}x |"
    )
    print()
    print(spread)
    runs = answer["seconds"]
    print(
        f"the fit itself: {runs:.3f} s, {answer['iterations']} iterations,"
        f" {answer['evaluations']} evaluations, {answer['jacobians']} jacobians,"
        f" {simulations} simulations of {runs * 1e3 / simulations:.4f} ms,"
        f" {answer['status']} ({fit_seconds:.3f} s of the call)"
    )
    found = [parameter["value"] for parameter in answer["parameters"]]
    recovered = [
        abs(value / wanted - 1.0) for value, wanted in zip(found, truth, strict=True)
    ]
    print(
        f"the cost of the optimum: {answer['cost']:.6e}, the parameters within"
        f" {max(recovered) * 100:.1f} % of the ones of the data"
    )
    if args.profile:
        walked = answer["profiles"][0]
        interval = (walked["lower"], walked["upper"])
        print(
            f"the profile of {walked['id']}: {len(walked['points'])} points in"
            f" {walked['seconds']:.3f} s, the interval {interval}"
        )
        fisher = answer["fisher"]
        if fisher["singular"]:
            print(f"the fisher information: {fisher['singular']}")
        else:
            print(f"the errors of the fisher information: {fisher['errors']}")
    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "conditions": args.conditions,
                    "parameters": k,
                    "threads": threads,
                    "python_ms": python_ms,
                    "rust_ms": rust_ms,
                    "fit_seconds": fit_seconds,
                    "simulations": simulations,
                    "answer": {
                        key: answer[key]
                        for key in (
                            "cost",
                            "status",
                            "iterations",
                            "evaluations",
                            "jacobians",
                        )
                    },
                },
                indent=2,
            ),
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
