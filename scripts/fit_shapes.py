"""The shapes of a fit which do not fill the pool of its batch by themselves.

    .venv/bin/python scripts/fit_shapes.py [--conditions 2,4,20] [--starts 4]
        [--profiles 3] [--parameters 5] [--iterations 10] [--rounds 3]
        [--threads N] [--json FILE]

It imports the built extension, so it runs in the environment the package was
installed into (`uv pip install -e . --group dev`) and not with `uv run`, which
would resolve `sbmlsim_core` from an index.

`scripts/fit_timing.py` measures one jacobian of a fit, which is the width of one
evaluation: one simulation per condition, `(parameters + 1)` of them per jacobian,
in one fan out over the pool. This script measures the shapes around it, which are
the ones of the issue: a multi start fit and the profiles of an identifiability,
whose runs do not depend on each other and run next to each other on the same pool
(`crates/fit`, the starts of `fit_with`, the sides of a profile and the profiles of
the parameters).

A fit of few conditions is narrower than the machine, so the table reports the
threads a shape keeps busy (the CPU time of the process over the wall time of the
call) next to its wall time: that is what the parallelism of the shapes is for. The
numbers of `bench/OPTIMIZATION.md` are the output of this script, the fastest of
`--rounds` rounds of every shape.

The model is hydrochlorothiazide (28 states) of `bench/sbmlsim.toml` and a condition
an oral dose over a horizon, the ones of `objective_timing.py`, whose builders this
script imports.
"""

import argparse
import json
import os
import time
from pathlib import Path
from typing import Any

import sbmlsim_core
from fit_timing import TRUTH, curves, fit_request
from objective_timing import conditions, model, observable, parameters, tolerance


def shape(
    loaded: sbmlsim_core.LoadedModel,
    description: dict[str, Any],
    n_conditions: int,
    n_parameters: int,
    starts: int,
    profiles: int,
    iterations: int,
    threads: int,
) -> tuple[float, float]:
    """The wall time and the CPU time of one fit of a shape, in seconds.

    `starts` runs of the fit, and the profiles of the first `profiles` parameters of
    the best of them, which is what an identifiability of the fit asks for.
    """
    atol = tolerance(description)
    column = observable(description)
    requests = conditions(n_conditions, ["time", column], atol)
    fitted = parameters(description, n_parameters)
    names = [name for name, _ in fitted]
    values = [value for _, value in fitted]
    truth = [value * TRUTH for value in values]
    with loaded.batch_executor(threads=threads) as executor:
        prepared = [loaded.prepare(request, varying=names) for request in requests]
        data = curves(executor, prepared, truth, 1)
    request = fit_request(requests, names, values, data, column, threads, iterations)
    if starts > 1:
        # the same fit from several points of the space, which is a multi start fit
        request["starts"] = [
            [value * (1.0 + 0.1 * start) for value in values] for start in range(starts)
        ]
    if profiles > 0:
        request["identifiability"] = {
            "parameters": names[:profiles],
            "walk": {"points": 4, "span": 0.5},
            "fisher": False,
        }
    began, cpu = time.perf_counter(), time.process_time()
    loaded.fit(request, threads=threads)
    return (time.perf_counter() - began, time.process_time() - cpu)


def main() -> None:
    """Measures every shape and prints the table."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--conditions",
        default="2,4,20",
        help="the widths of one evaluation to measure, comma separated",
    )
    parser.add_argument("--starts", type=int, default=4)
    parser.add_argument("--profiles", type=int, default=3)
    parser.add_argument("--parameters", type=int, default=5)
    parser.add_argument("--iterations", type=int, default=10)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--threads", type=int, default=0)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    threads = args.threads or len(os.sched_getaffinity(0))
    widths = [int(width) for width in args.conditions.split(",")]

    loaded, description = model()
    # a round of the model before the measurement, which instantiates the pool and
    # lets the first fit pay for the engine of every thread
    shape(loaded, description, 2, args.parameters, 1, 0, 2, threads)

    rows: list[dict[str, Any]] = []
    for width in widths:
        for what, (starts, profiles) in [
            ("one fit", (1, 0)),
            (f"{args.starts} starts", (args.starts, 0)),
            (f"the profiles of {args.profiles} parameters", (1, args.profiles)),
        ]:
            # the fastest round is the one a load of the machine disturbed least
            rounds = [
                shape(
                    loaded,
                    description,
                    width,
                    args.parameters,
                    starts,
                    profiles,
                    args.iterations,
                    threads,
                )
                for _ in range(args.rounds)
            ]
            wall, cpu = min(rounds, key=lambda round: round[0])
            rows.append(
                {
                    "conditions": width,
                    "shape": what,
                    "seconds": wall,
                    "cpu_seconds": cpu,
                    "threads_busy": cpu / wall,
                }
            )

    print(
        f"{args.parameters} parameters, {args.iterations} iterations,"
        f" {threads} threads, {args.rounds} rounds"
    )
    print()
    print("| the shape | wall time | threads busy |")
    print("|---|---:|---:|")
    for row in rows:
        print(
            f"| {row['conditions']} conditions, {row['shape']} |"
            f" {row['seconds'] * 1e3:.1f} ms | {row['threads_busy']:.1f} of {threads} |"
        )
    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "threads": threads,
                    "parameters": args.parameters,
                    "iterations": args.iterations,
                    "rounds": args.rounds,
                    "shapes": rows,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
