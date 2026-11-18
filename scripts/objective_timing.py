"""The wall time of one Jacobian of a fit, on the paths the python bindings offer.

    .venv/bin/python scripts/objective_timing.py [--conditions 20]
        [--parameters 10] [--threads N] [--repeats 5] [--json FILE]

It imports the built extension, so it runs in the environment the package was
installed into (`uv pip install -e . --group dev`) and not with `uv run`, which
would resolve `sbmlsim_core` from an index.

One evaluation of the objective of a fit is one simulation per condition, every
one of them at the same parameters. The Jacobian of a step of a least squares
optimizer is that evaluation once per parameter and once at the parameters
themselves, so `(parameters + 1) * conditions` simulations which do not depend
on each other, and the whole cost of a fit of many parameters. The model is
hydrochlorothiazide (28 states) of `bench/sbmlsim.toml`, which
`scripts/prepare_model.py --bench bench/sbmlsim.toml` writes, and a condition is
an oral dose over a horizon, as the studies of a fit of that model are.

The paths, each of which answers with the tables of the first one bitwise (as a
multiset: a batch per condition answers its points in another order):

- `dicts, one instance`: a request of `simulation/1` per point on one kept
  instance, which is what an executor that simulates one point per call does.
- `prepared, one instance`: the same points as prepared requests, serially, so
  that building a request and reading a table is told from the integration.
- `prepared, a batch per condition`: `BatchExecutor.simulate_prepared`, one call
  per condition, which is as far as one batch reached before `simulate_plans`.
- `plans, one batch`: `BatchExecutor.simulate_plans`, the whole Jacobian in one
  fan out over the threads of the pool.
- `dicts, a process pool`: a worker process per thread, each with an instance of
  its own and the conditions of the fit, one task per point and the values
  pickled back, which is the shape of the parallel fit of sbmlsim.
"""

import argparse
import json
import math
import os
import statistics
import time
from collections.abc import Callable, Sequence
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from typing import Any

import sbmlsim_core

ROOT = Path(__file__).resolve().parents[1]
SBML = ROOT / "target/bench-models/sbmlsim-hctz.xml"
ATOL = ROOT / "target/bench-models/sbmlsim-hctz.atol.json"
DOSE = "PODOSE_hctz"
END = 1440.0
STEPS = 100
# the relative step of the finite differences of sbmlsim, the square root of the
# relative tolerance of the integration (`diff_step` of `docs/fitting.md`)
DIFF_STEP = 1e-3
# the parameters a fit of this model varies first: the absorption, the transit,
# the renal function and the water of the body, as the HCTZ example of sbmlsim
# fits them. A larger `--parameters` takes the settable parameters after them.
FIT_PARAMETERS = (
    "Ka_dis_hctz",
    "ti_hctz",
    "KI__HCTZEX_k",
    "KI__urine_volume",
    "f_renal_function",
    "f_GFR_Qki",
    "f_cardiac_function",
    "f_TBW",
    "f_ECF",
)

_WORKER: dict[str, Any] = {}


def tolerance(description: dict[str, Any]) -> dict[str, Any]:
    """Roadrunner's vector of absolute tolerances of the states, or the scalar."""
    if not ATOL.exists():
        return {"scalar": 1e-10}
    atol = json.loads(ATOL.read_text(encoding="utf-8"))
    states = [state["id"] for state in description["states"]]
    if any(state not in atol for state in states):
        return {"scalar": 1e-10}
    return {"per_state": [atol[state] for state in states]}


def request(
    dose: float, end: float, selections: list[str], atol: dict[str, Any]
) -> dict[str, Any]:
    """The request of one condition: an oral dose over a horizon, 101 points."""
    return {
        "contract": sbmlsim_core.SIMULATION_CONTRACT,
        "start": 0.0,
        "end": end,
        "output": {"uniform": {"steps": STEPS}},
        "preinit": [(DOSE, dose)],
        "changes": [],
        "steady_state": None,
        "selections": selections,
        "settings": {
            "relative_tolerance": 1e-6,
            "absolute_tolerance": atol,
            "maximum_num_steps": 20000,
            "maximum_time_step": 0.0,
            "initial_time_step": 0.0,
            "stiff": True,
            "linear_solver": "auto",
            "jacobian": "auto",
        },
    }


def conditions(
    count: int, selections: list[str], atol: dict[str, Any]
) -> list[dict[str, Any]]:
    """One request per condition: doses of 12.5 to 100 mg over 24 or 48 hours."""
    doses = [12.5 * (1 + at % 8) for at in range(count)]
    ends = [END * (1 + at % 2) for at in range(count)]
    return [
        request(dose, end, selections, atol)
        for dose, end in zip(doses, ends, strict=True)
    ]


def parameters(description: dict[str, Any], count: int) -> list[tuple[str, float]]:
    """The parameters of the fit, filled up with the settable ones of the model.

    A parameter is one a request can set, whose value is finite and not zero, so
    that a relative step of it is a step at all.
    """
    settable = {
        parameter["id"]: parameter["initial"]
        for parameter in description["parameters"]
        if parameter["settable"]
        and isinstance(parameter["initial"], float)
        and math.isfinite(parameter["initial"])
        and parameter["initial"] != 0.0
    }
    order = [name for name in FIT_PARAMETERS if name in settable]
    order += [name for name in settable if name not in order]
    if len(order) < count:
        raise SystemExit(f"the model has {len(order)} parameters to vary, not {count}")
    return [(name, settable[name]) for name in order[:count]]


def jacobian_points(values: list[float]) -> list[list[float]]:
    """The parameters themselves and one point per parameter, stepped by `DIFF_STEP`."""
    points = [list(values)]
    for at, value in enumerate(values):
        point = list(values)
        point[at] = value * (1.0 + DIFF_STEP)
        points.append(point)
    return points


def point_request(
    base: dict[str, Any], varying: Sequence[str], values: Sequence[float]
) -> dict[str, Any]:
    """The request of one point: the condition with the parameters of the point."""
    return base | {"preinit": [*base["preinit"], *zip(varying, values, strict=True)]}


def observable(description: dict[str, Any]) -> str:
    """The selection a fit of this model reads: a plasma concentration of the drug."""
    ids = [selection["id"] for selection in description["selections"]]
    for wanted in ("Cve_hctz", "Cve_plasma_hctz", "Car_hctz"):
        if wanted in ids:
            return wanted
    drug = [name for name in ids if name.endswith("_hctz") and name.startswith("C")]
    return drug[0] if drug else ids[0]


def model() -> tuple[sbmlsim_core.LoadedModel, dict[str, Any]]:
    """The model of the benchmark, compiled and loaded, with its description."""
    if not SBML.exists():
        raise SystemExit(
            f"{SBML.relative_to(ROOT)} is missing: run"
            " `uv run scripts/prepare_model.py --bench bench/sbmlsim.toml`"
        )
    compiled = sbmlsim_core.compile_sbml(SBML.read_text(encoding="utf-8"))
    loaded = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)
    return loaded, compiled.description


def _init(sbml: str, requests: list[dict[str, Any]], varying: list[str]) -> None:
    """Loads the model, an instance and the conditions in a worker of the pool."""
    compiled = sbmlsim_core.compile_sbml(sbml)
    loaded = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)
    _WORKER["instance"] = loaded.instance()
    _WORKER["requests"] = requests
    _WORKER["varying"] = varying


def _point(task: tuple[int, tuple[float, ...]]) -> Any:
    """Simulates one point of the Jacobian in a worker and gives its values."""
    at, values = task
    base = _WORKER["requests"][at]
    request = point_request(base, _WORKER["varying"], values)
    return _WORKER["instance"].simulate(request).values


def values_of(results: list[Any]) -> list[bytes]:
    """The bytes of the table of every point, whatever the path answered with."""
    failed = [
        result for result in results if isinstance(result, sbmlsim_core.PointError)
    ]
    if failed:
        raise SystemExit(f"a point of the jacobian failed: {failed[0].message}")
    return [
        (result if hasattr(result, "tobytes") else result.values).tobytes()
        for result in results
    ]


def measure(
    paths: dict[str, Callable[[], list[Any]]], rounds: int
) -> tuple[dict[str, list[float]], dict[str, list[bytes]]]:
    """The seconds of every path in every round, and the tables each answered with.

    A round times every path once, in the order of the paths, so that a load of
    the machine which comes and goes is spread over the paths instead of landing
    on the one which happens to run while it lasts. The first round is a warm up
    and is not reported.
    """
    seconds: dict[str, list[float]] = {name: [] for name in paths}
    values: dict[str, list[bytes]] = {}
    for round_at in range(rounds + 1):
        for name, run in paths.items():
            started = time.perf_counter()
            results = run()
            took = time.perf_counter() - started
            if round_at:
                seconds[name].append(took)
            if name not in values:
                values[name] = values_of(results)
    return seconds, values


def main() -> None:
    """Measures every path on one Jacobian and prints the table."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--conditions", type=int, default=20, help="simulations")
    parser.add_argument("--parameters", type=int, default=10, help="fit parameters")
    parser.add_argument("--threads", type=int, default=0, help="0 is every CPU")
    parser.add_argument("--rounds", type=int, default=9, help="rounds, the median")
    parser.add_argument("--json", type=Path, help="write the seconds as JSON")
    args = parser.parse_args()
    cpus = os.process_cpu_count() or 1
    threads = args.threads or cpus

    loaded, description = model()
    atol = tolerance(description)
    selections = ["time", observable(description)]
    requests = conditions(args.conditions, selections, atol)
    varied = parameters(description, args.parameters)
    names = [name for name, _ in varied]
    points = jacobian_points([value for _, value in varied])
    simulations = len(points) * len(requests)
    print(
        f"{cpus} CPUs, sbmlsim_core {sbmlsim_core.__version__}, hctz, "
        f"{args.conditions} conditions, {args.parameters} parameters, "
        f"{simulations} simulations per jacobian, {threads} threads"
    )
    print(f"the selection is {selections[1]}, the parameters are {', '.join(names)}")

    instance = loaded.instance()
    prepared = [loaded.prepare(base, varying=names) for base in requests]
    tasks = [(at, tuple(point)) for point in points for at in range(len(requests))]

    with (
        loaded.batch_executor(threads=threads) as executor,
        ProcessPoolExecutor(
            max_workers=threads,
            initializer=_init,
            initargs=(SBML.read_text(encoding="utf-8"), requests, names),
        ) as pool,
    ):
        paths: dict[str, Callable[[], list[Any]]] = {
            "dicts, one instance": lambda: [
                instance.simulate(point_request(requests[at], names, values))
                for at, values in tasks
            ],
            "prepared, one instance": lambda: [
                instance.simulate_prepared(prepared[at], list(values))
                for at, values in tasks
            ],
            "prepared, a batch per condition": lambda: [
                result
                for at, plan in enumerate(prepared)
                for result in executor.simulate_prepared(
                    plan, [[*point] for point in points]
                )
            ],
            "plans, one batch": lambda: executor.simulate_plans(
                [(prepared[at], list(values)) for at, values in tasks]
            ),
            "dicts, a process pool": lambda: list(pool.map(_point, tasks, chunksize=1)),
        }
        rounds, values = measure(paths, args.rounds)

    serial_path = "dicts, one instance"
    reference = sorted(values[serial_path])
    for name, answered in values.items():
        if sorted(answered) != reference:
            raise SystemExit(f"the path `{name}` is not `{serial_path}` bitwise")
    measured = {name: statistics.median(times) for name, times in rounds.items()}
    serial = measured[serial_path]
    print(
        f"\n{'path':34} {'median s':>9} {'min s':>8} {'max s':>8}"
        f" {'ms/simulation':>14} {'speedup':>8}"
    )
    for name, seconds in measured.items():
        print(
            f"{name:34} {seconds:9.3f} {min(rounds[name]):8.3f}"
            f" {max(rounds[name]):8.3f} {seconds / simulations * 1000:14.3f}"
            f" {serial / seconds:7.2f}x"
        )
    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "cpus": cpus,
                    "threads": threads,
                    "conditions": args.conditions,
                    "parameters": args.parameters,
                    "simulations": simulations,
                    "seconds": measured,
                    "rounds": rounds,
                },
                indent=2,
                sort_keys=True,
            ),
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
