# /// script
# requires-python = ">=3.14,<3.15"
# dependencies = [
#     "sbmlsim>=0.24.0",
#     "libroadrunner",
#     "numpy",
# ]
#
# [tool.uv.sources]
# libroadrunner = { index = "roadrunner-dev" }
#
# [[tool.uv.index]]
# name = "roadrunner-dev"
# url = "https://matthiaskoenig.github.io/roadrunner-wheels/simple/"
# explicit = true
# ///
"""The simulations of the benchmark of sbmlsim-core with roadrunner, and the ratios.

    uv run scripts/roadrunner_reference.py bench/models.toml --json target/bench/roadrunner.json
    uv run scripts/roadrunner_reference.py bench/models.toml --compare target/bench/roadrunner.json \
        target/bench/engine.json target/bench/node.json target/bench/chrome.json

Every model of the benchmark file (prepared with `scripts/prepare_model.py --bench`) is
simulated as `sbmlsim-core bench` simulates it: from 0 to its end, its uniform output points,
its pre-initialization (the initial values `init(x)`, then `resetAll`, which evaluates the
initial assignments with them, as the engine and sbmlsim apply it; checked: the state at the
start is the engine's for every model, `--export`), BDF, its relative tolerance and its absolute
tolerance (roadrunner scales it per state, checked equal to the vector of `<sbml>.atol.json`
which the engine integrates with), at most 20000 steps, LLJIT at the optimization level `O2`
(`DEFAULT`, as sbmlsim loads models), in a process of its own pinned with `taskset` to one CPU
(`--cpu`, a performance core). roadrunner dense is the libroadrunner of this environment (dense,
CVODE's difference quotient); roadrunner KLU is the `roadrunner-sparsedq` build (`--sparse-build`,
KLU with its colored sparse difference quotient; the released libroadrunner has no KLU). A
measurement is the wall time of `np.array(r.simulate(0, end, points))` (and of the steps of the
integrator for a model with `steps = true`, `variable_step_size`), the median of `--reps` after
`--warmups`; the two builds take turns over `--rounds` rounds and a time is the median of the
round medians, as in the spike (spike/RESULTS.md) and in `sbmlsim-core bench`.

With `--compare`, the first file is the result of this script and the others are results of the
engine (`sbmlsim-core bench --json`, `js/bench.mjs --json`, `bench/web`); the tables of
bench/RESULTS.md are printed in Markdown: the time of every model, solver and host, and its
ratio to roadrunner (dense against roadrunner dense, KLU against roadrunner KLU).
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path
from statistics import median

SPARSE_BUILD = Path.home() / "rr-enzyme/build-sparsedq/lib/site-packages"


def read_models(toml: Path) -> dict[str, dict]:
    """Get the models of a benchmark file with their SBML paths resolved."""
    config = tomllib.loads(toml.read_text(encoding="utf-8"))
    models = {}
    for name, model in config["models"].items():
        model = dict(model)
        model["sbml"] = str((toml.parent / model["sbml"]).resolve())
        models[name] = model
    return models


def worker(build: str, model: dict, warmups: int, reps: int) -> None:
    """Time roadrunner on one model; print one JSON list of rows."""
    import numpy as np
    import roadrunner

    roadrunner.Logger.setLevel(roadrunner.Logger.LOG_ERROR)
    options = roadrunner.LoadSBMLOptions()
    options.setLLVMBackend(roadrunner.LoadSBMLOptions.LLJIT)
    options.setLLJitOptimizationLevel(roadrunner.LoadSBMLOptions.DEFAULT)  # O2, as sbmlsim
    t0 = time.perf_counter()
    r = roadrunner.RoadRunner(model["sbml"], options)
    load = time.perf_counter() - t0
    # the state vector (`getRatesOfChange` is in its order)
    states = [r.model.getStateVectorId(i) for i in range(len(r.getRatesOfChange()))]
    # the pre-initialization as the engine and sbmlsim apply it: the initial values, which every
    # `resetAll` keeps and evaluates the initial assignments with (setting one regenerates the
    # model and its selections, so it is set once, before the selections)
    for key, value in model.get("preinit", []):
        r.setValue(f"init({key})", value)
    r.timeCourseSelections = ["time", *states]
    atol_file = Path(model["sbml"]).with_suffix(".atol.json")
    atol_expected = json.loads(atol_file.read_text()) if atol_file.exists() else None
    # (host, linear solver, Jacobian): the libroadrunner of this environment dense (CVODE's
    # difference quotient) and, where it has them, with KLU and its sparse difference quotient
    # (the defaults of libroadrunner 2.12); the sparsedq build with KLU
    if build == "sparsedq":
        configs = [("roadrunner KLU", "klu", "sparse_difference_quotient")]
    else:
        configs = [("roadrunner dense", "dense", "difference_quotient")]
        if "linear_solver" in r.integrator.getSettings():
            configs.append((f"roadrunner {roadrunner.__version__} KLU", "klu", "sparse_difference_quotient"))
    outputs = ["times", "steps"] if model.get("steps") else ["times"]
    rows = []
    for host, solver, jacobian in configs:
        if "linear_solver" in r.integrator.getSettings():
            r.integrator.setValue("linear_solver", "dense")
            r.integrator.setValue("jacobian", jacobian)
            r.integrator.setValue("linear_solver", solver)
        r.integrator.setValue("stiff", True)
        r.integrator.setValue("relative_tolerance", model["relative_tolerance"])
        r.integrator.setValue("absolute_tolerance", model["absolute_tolerance"])
        r.integrator.setValue("maximum_num_steps", 20000)
        for output in outputs:
            r.integrator.setValue("variable_step_size", output == "steps")
            times = []
            result = None
            for i in range(warmups + reps):
                r.resetAll()
                t0 = time.perf_counter()
                if output == "steps":
                    result = np.array(r.simulate(0, model["end"]))
                else:
                    result = np.array(r.simulate(0, model["end"], model["points"]))
                dt = time.perf_counter() - t0
                if i >= warmups:
                    times.append(dt)
            s = r.integrator.getStatistics()
            r.resetAll()
            initial = {sid: float(r.getValue(sid)) for sid in states}
            assert list(r.timeCourseSelections) == ["time", *states] or result.shape[1] == len(states) + 1
            if "linear_solver" in r.integrator.getSettings():
                assert (r.integrator.getValue("linear_solver"), r.integrator.getValue("jacobian")) == (solver, jacobian)
            vector = r.integrator.getAbsoluteToleranceVector()
            atol_matches = None
            if atol_expected is not None:
                atol_matches = bool(
                    len(vector) == len(atol_expected)
                    and all(atol_expected[sid] == float(v) for sid, v in zip(states, vector))
                )
            rows.append(
                {
                    "host": host,
                    "roadrunner": roadrunner.__version__,
                    "build": build,
                    "solver": solver,
                    "jacobian": jacobian,
                    "output": output,
                    "load_s": load,
                    "median_s": median(times),
                    "n_rows": int(result.shape[0]),
                    "states": len(states),
                    "stats": {
                        "steps": s["num_steps"],
                        "rhs_evals": s["num_rhs_evals"],
                        "lin_rhs_evals": s["num_lin_rhs_evals"],
                        "jacobian_evals": s["num_jac_evals"],
                        "linear_setups": s["num_lin_solv_setups"],
                        "nonlinear_iterations": s["num_nonlin_solv_iters"],
                        "nonlinear_convergence_failures": s["num_nonlin_solv_conv_fails"],
                        "error_test_failures": s["num_err_test_fails"],
                        "reinits": s.get("num_reinits"),
                    },
                    "atol_matches": atol_matches,
                    # the state at the start (amounts), by the ids of the state vector
                    "initial": initial,
                }
            )
    print(json.dumps(rows))


def run_worker(build: str, name: str, args: argparse.Namespace) -> list[dict]:
    """Run a worker process of a build, pinned to the CPU."""
    env = dict(os.environ)
    if build == "sparsedq":
        env["PYTHONPATH"] = str(args.sparse_build)
    cmd = ["taskset", "-c", args.cpu, sys.executable, __file__, "--worker", build, name]
    cmd += [str(args.models), "--warmups", str(args.warmups), "--reps", str(args.reps)]
    out = subprocess.run(cmd, check=False, capture_output=True, text=True, env=env)
    if out.returncode:
        raise RuntimeError(f"{build} {name}: {out.stderr[-3000:]}")
    return json.loads(out.stdout.strip().splitlines()[-1])


def initial_state_problems(name: str, configs: list[dict], args: argparse.Namespace) -> list[str]:
    """Compare the state at the start of roadrunner with the one of the engine.

    The engine simulates the request of the export (`sbmlsim-core bench --export`, the dense
    configuration of the output of times) with the command line; the value of every state of
    roadrunner's state vector after the pre-initialization and the reset (an amount for a species)
    must be the one of the first row of the engine within 1e-12 relative (1e-300 absolute), so
    both simulate the same problem (the pre-initialization and the initial assignments).
    """
    if args.skip_initial_check:
        return []
    model_dir = args.export / name
    request = model_dir / "dense_analytic_times.json"
    if not request.exists() or not args.cli.exists():
        return [f"{name}: no {request} or {args.cli} to check the initial state (--skip-initial-check)"]
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "out.csv"
        subprocess.run([str(args.cli), "simulate", str(model_dir), str(request), "-o", str(out)], check=True)
        header, first = out.read_text().splitlines()[:2]
    engine = dict(zip(header.split(","), map(float, first.split(",")), strict=True))
    problems = []
    for config in configs:
        if config["output"] != "times":
            continue
        for sid, value in config["initial"].items():
            other = engine.get(sid)
            if other is None or abs(other - value) > 1e-12 * abs(value) + 1e-300:
                problems.append(f"{name}: {config['host']}: the state {sid} starts at {value!r}, the engine's at {other!r}")
    if not problems:
        print(f"{name}: roadrunner and the engine start from the same state", file=sys.stderr)
    return problems


def measure(args: argparse.Namespace) -> int:
    """Measure every model with both builds; write the JSON."""
    models = read_models(args.models)
    only = set(args.only.split(",")) if args.only else None
    builds = ["release", "sparsedq"] if args.sparse_build.exists() else ["release"]
    if "sparsedq" not in builds:
        print(f"warning: no roadrunner-sparsedq build at {args.sparse_build}", file=sys.stderr)
    import roadrunner

    results = {
        "host": "roadrunner",
        "roadrunner": roadrunner.__version__,
        "sparse_build": str(args.sparse_build),
        "rounds": args.rounds,
        "warmups": args.warmups,
        "reps": args.reps,
        "models": [],
    }
    problems: list[str] = []
    for name in models:
        if only and name not in only:
            continue
        rounds: dict[tuple[str, str], list[dict]] = {}
        for k in range(args.rounds):
            batch = [row for build in builds for row in run_worker(build, name, args)]
            for row in batch:
                rounds.setdefault((row["host"], row["output"]), []).append(row)
            print(
                f"{name}: round {k + 1}/{args.rounds}: "
                + ", ".join(f"{r['host']} {r['output']} {r['median_s'] * 1e3:.4g} ms" for r in batch),
                file=sys.stderr,
                flush=True,
            )
        configs = []
        for rows in rounds.values():
            configs.append(
                {
                    **rows[-1],
                    "median_s": median(r["median_s"] for r in rows),
                    "round_medians_s": [r["median_s"] for r in rows],
                }
            )
        problems += initial_state_problems(name, configs, args)
        mismatch = [c["host"] for c in configs if c["atol_matches"] is False]
        if mismatch:
            print(f"warning: {name}: the absolute tolerances differ from the file ({mismatch})", file=sys.stderr)
        results["models"].append({"name": name, "configs": configs})
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(results, indent=1), encoding="utf-8")
    for model in results["models"]:
        for c in model["configs"]:
            s = c["stats"]
            print(
                f"{model['name']:<14} {c['host']:<17} {c['output']:<6} {c['median_s'] * 1e3:10.4f} ms"
                f"  steps {s['steps']:6d}  rhs {s['rhs_evals'] + s['lin_rhs_evals']:7d}  jac {s['jacobian_evals']:5d}"
                f"  reinits {s['reinits']}  load {c['load_s']:.3f} s"
            )
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    return 1 if problems else 0


def fmt(seconds: float) -> str:
    """Format a time in milliseconds with 3 significant digits."""
    ms = seconds * 1e3
    return f"{ms:.3g}" if ms < 1000 else f"{ms:.0f}"


def compare(args: argparse.Namespace) -> int:
    """Print the tables of the times and the ratios against roadrunner in Markdown.

    The KLU baseline is the sparsedq build, else the KLU of the libroadrunner of this environment
    (`roadrunner <version> KLU`). A configuration without a baseline, or with a ratio which is no
    number, is an error (exit code 1), so no ratio passes the check of 2x unseen.
    """
    reference = json.loads(args.compare[0].read_text())
    engines = [json.loads(p.read_text()) for p in args.compare[1:]]
    rr = {(m["name"], c["host"], c["output"]): c for m in reference["models"] for c in m["configs"]}

    def baseline(name: str, solver: str, output: str) -> dict | None:
        if solver == "dense":
            return rr.get((name, "roadrunner dense", output))
        found = rr.get((name, "roadrunner KLU", output))
        if found is None:
            found = next(
                (c for (n, h, o), c in rr.items() if n == name and o == output and c["solver"] == "klu"), None
            )
        return found

    print(
        "| model | states | solver | output | roadrunner ms | "
        + " | ".join(f"{e['host']} ms | ratio" for e in engines)
        + " |"
    )
    print("|---|---:|---|---|---:|" + "---:|---:|" * len(engines))
    worst = []
    errors = []
    names = [m["name"] for m in engines[0]["models"]]
    for name in names:
        for output in ("times", "steps"):
            for solver in ("dense", "klu"):
                ref = baseline(name, solver, output)
                cells = []
                states = None
                for e in engines:
                    model = next((m for m in e["models"] if m["name"] == name), None)
                    config = model and next(
                        (
                            c
                            for c in model["configs"]
                            if c["solver"] == solver and c["jacobian"] == "analytic" and c["output"] == output
                        ),
                        None,
                    )
                    if config is None:
                        cells += ["-", "-"]
                        continue
                    states = model["states"]
                    ratio = config["median_s"] / ref["median_s"] if ref else float("nan")
                    if not math.isfinite(ratio):
                        errors.append(f"{name} {solver} {output} {e['host']}: no roadrunner reference")
                    elif ratio > 2:
                        worst.append((name, solver, output, e["host"], ratio))
                    cells += [fmt(config["median_s"]), f"{ratio:.2f}"]
                if states is None:
                    continue
                reference_ms = f"{fmt(ref['median_s'])} ({ref['host']})" if ref else "-"
                print(f"| {name} | {states} | {solver} | {output} | {reference_ms} | " + " | ".join(cells) + " |")
    print()
    for name, solver, output, host, ratio in worst:
        print(f"above 2x: {name} {solver} {output} {host} {ratio:.2f}")
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


def main() -> int:
    """Run the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("models", type=Path, help="the benchmark file (bench/models.toml)")
    parser.add_argument("--json", type=Path, help="write the results as JSON")
    parser.add_argument("--only", help="only these models (comma separated)")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--reps", type=int, default=20)
    parser.add_argument("--cpu", default="2", help="the CPU the workers are pinned to")
    parser.add_argument("--sparse-build", type=Path, default=SPARSE_BUILD)
    parser.add_argument("--compare", type=Path, nargs="+", help="print the tables of results")
    parser.add_argument(
        "--export",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "target/bench/export",
        help="the export of `sbmlsim-core bench --export`, whose initial states are checked",
    )
    parser.add_argument(
        "--cli", type=Path, default=Path(__file__).resolve().parent.parent / "target/release/sbmlsim-core"
    )
    parser.add_argument("--skip-initial-check", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--worker", nargs=2, metavar=("BUILD", "MODEL"), help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.worker:
        build, name = args.worker
        worker(build, read_models(args.models)[name], args.warmups, args.reps)
        return 0
    if args.compare:
        return compare(args)
    return measure(args)


if __name__ == "__main__":
    sys.exit(main())
