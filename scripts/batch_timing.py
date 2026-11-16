"""The wall time of `LoadedModel.simulate_many` at one thread against every CPU.

    .venv/bin/python scripts/batch_timing.py [--points 320] [--json FILE]

It imports the built extension, so it runs in the environment the package was installed into
(`uv pip install -e . --group dev`) and not with `uv run`, which would resolve `sbmlsim_core`
from an index. Every result of every thread count, the default of every CPU among them, is
compared bitwise with the one a python loop on a single instance gives, so the numbers are
only reported for a batch which answers what a serial run answers.

Three kinds of point, measured on the decay model of `crates/host/tests/decay.xml`:

- `long integration`: a long time in small steps of the integrator with 11 rows of output, the
  cost of a point of a real scan (tens of milliseconds of integration).
- `large table`: 20001 rows, where the table and not the integration is the cost.
- `short timecourse`: 101 rows over a short time, where the cost of a point is so small that
  the batch itself (the pool, the slices, the objects of python) is what is measured.
"""

import argparse
import json
import os
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import sbmlsim_core

ROOT = Path(__file__).resolve().parents[1]
DECAY = ROOT / "crates/host/tests/decay.xml"


def request(**fields: Any) -> dict[str, Any]:
    """A request of `simulation/1` over `[0, end]` in `steps` equal intervals."""
    steps = fields.pop("steps")
    end = fields.pop("end")
    preinit = fields.pop("preinit", [])
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
        "preinit": preinit,
        "changes": [],
        "steady_state": None,
        "selections": [],
        "settings": settings,
    }


# a point of each cost, by the index of the point (so that no two points of a batch are equal)
KINDS: dict[str, Callable[[int], dict[str, Any]]] = {
    "long integration": lambda n: request(
        end=1e6,
        steps=10,
        maximum_time_step=10.0,
        maximum_num_steps=100_000_000,
        preinit=[("y", 1.0 + n)],
    ),
    "large table": lambda n: request(end=1e4, steps=20_000, preinit=[("y", 1.0 + n)]),
    "short timecourse": lambda n: request(
        end=10.0, steps=100, preinit=[("y", 1.0 + n)]
    ),
}


def model() -> sbmlsim_core.LoadedModel:
    """The decay model of the host tests, compiled and loaded."""
    compiled = sbmlsim_core.compile_sbml(DECAY.read_text(encoding="utf-8"))
    return sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)


def check(serial: list[Any], results: list[Any], what: str) -> None:
    """Exits when a result of the batch is not the one of the serial loop, bitwise."""
    for want, got in zip(serial, results, strict=True):
        if want.values.tobytes() != got.values.tobytes():
            raise SystemExit(f"the batch {what} is not the serial run")


def measure(
    loaded: sbmlsim_core.LoadedModel,
    make: Callable[[int], dict[str, Any]],
    points: int,
    counts: list[int],
) -> dict[str, Any]:
    """The seconds of a python loop and of the batch at every thread count of `counts`."""
    requests = [make(n) for n in range(points)]
    instance = loaded.instance()
    started = time.perf_counter()
    serial = [instance.simulate(r) for r in requests]
    loop = time.perf_counter() - started
    batches: dict[int, float] = {}
    for threads in counts:
        started = time.perf_counter()
        results = loaded.simulate_many(requests, threads=threads)
        batches[threads] = time.perf_counter() - started
        check(serial, results, f"of {threads} threads")
    started = time.perf_counter()
    results = loaded.simulate_many(requests)
    default = time.perf_counter() - started
    check(serial, results, "of every CPU")
    return {"points": points, "loop": loop, "batches": batches, "default": default}


def report(name: str, measured: dict[str, Any]) -> None:
    """Prints the seconds of a kind of point with the speedup over one thread."""
    points, loop = measured["points"], measured["loop"]
    batches, one = measured["batches"], measured["batches"][1]
    print(f"\n{name}, {points} points (one point {loop / points * 1000:.2f} ms serial)")
    print(f"  a python loop on one instance   {loop:7.3f} s")
    for threads, seconds in batches.items():
        speedup = f"speedup {one / seconds:5.2f}x" if seconds else ""
        print(
            f"  simulate_many(threads={threads:<3})        {seconds:7.3f} s   {speedup}"
        )
    print(f"  simulate_many() (every CPU)     {measured['default']:7.3f} s")


def main() -> None:
    """Measures every kind of point and prints the table."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--points", type=int, default=320, help="points per batch")
    parser.add_argument("--json", type=Path, help="write the seconds as JSON")
    args = parser.parse_args()
    cpus = os.process_cpu_count() or 1
    counts = sorted({1, 2, 4, 8, cpus})
    print(f"{cpus} CPUs, sbmlsim_core {sbmlsim_core.__version__}")
    loaded = model()
    measured = {
        name: measure(loaded, make, args.points, counts) for name, make in KINDS.items()
    }
    for name, one in measured.items():
        report(name, one)
    if args.json:
        args.json.write_text(
            json.dumps({"cpus": cpus, "kinds": measured}, indent=2, sort_keys=True),
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
