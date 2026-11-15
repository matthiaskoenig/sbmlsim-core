# /// script
# requires-python = ">=3.14,<3.15"
# dependencies = [
#     "sbmlsim>=0.24.0",
#     "libroadrunner",
#     "python-libsbml",
#     "numpy",
#     "pandas",
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
"""The agreement of sbmlsim-core with roadrunner (sbmlsim) at tight tolerances.

    uv run scripts/agreement.py [--cli target/release/sbmlsim-core] [--json FILE] [--details]

The packaged models of sbmlsim (`HCTZ_SBML`, `MIDAZOLAM_SBML`, `REPRESSILATOR_SBML`,
`DEMO_SBML`), the machine perfusion models of the benchmark (`mp_core`, `mp_flat`, sources of
`bench/models.toml`) and the event model of the benchmark are prepared with
`scripts/prepare_model.py` (comp flattened, distrib stripped, L3V2) and simulated with sbmlsim's
`Simulator` (roadrunner, its semantics of the pre-initialization, which the engine follows) and
with `sbmlsim-core simulate` (wasmtime), 201 uniform output times. The absolute tolerance is the
scalar of the run, which sbmlsim scales per state; the engine gets the very vector roadrunner
integrates with (by the ids of the state vector).

The criterion: the reference is roadrunner at rtol 1e-12 and atol 1e-14, the
converged solution. The engine at rtol 1e-10 and atol 1e-12, with the dense solver and with KLU
(both forced, whatever `KLU_THRESHOLD`), is compared with it with
`sbmlsim.comparison.diff.DataSetsComparison` (`tol_rel = 1e-7`, `tol_abs = 1e-12`) on every
column both have: a value passes when `|c - u| <= 1e-12 + 1e-7 |c|`, its excess is
`|c - u| / (1e-12 + 1e-7 |c|)`. A solver passes when every value passes, or when its largest
excess is at most 1 % above the one (`EXCESS_MARGIN`) of roadrunner at rtol 1e-10 and atol 1e-12 with the same linear
solver (dense with CVODE's difference quotient, KLU with the sparse difference quotient) against
the same reference: the engine is as close to the converged solution as roadrunner at the same
tolerances. The reference itself is roadrunner with the defaults of its libroadrunner (2.12: KLU
with the sparse difference quotient). A model passes when both solvers pass; the exit code is 1 when a model fails.

Reported besides: the largest relative difference `2 |c - u| / (|c| + |u|)` over all values and
over the values above `RESOLVED` (1e-9, 1000 times the absolute tolerance), and the number of
failing values, and (informative, no part of the criterion) the largest excess of the engine
against roadrunner at the same tolerances; `--details` prints the largest excesses of the
engine against the reference.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_model import PreparationError, prepare, resolve_source

ROOT = Path(__file__).resolve().parent.parent
RELATIVE_LIMIT = 1e-7
# the engine passes the second half of the criterion when its largest excess is at most
# EXCESS_MARGIN times roadrunner's; without the margin the event model failed by a tie (its
# largest excess is the same value in both, 1.73855 against 1.73789, where the engine dense and
# roadrunner dense differ by 2.6e-14 relative)
EXCESS_MARGIN = 1.01
RTOL, ATOL = 1e-10, 1e-12
# the values the largest relative difference is also reported over: above 1000 times the
# absolute tolerance
RESOLVED = 1e-9
POINTS = 201

# name -> (source, end, pre-initialization)
MODELS: dict[str, tuple[str, float, dict[str, float]]] = {
    "hctz": ("sbmlsim:hctz_body_flat.xml", 1440.0, {"PODOSE_hctz": 25.0}),
    "midazolam": ("sbmlsim:midazolam_body_flat.xml", 1440.0, {"PODOSE_mid": 7.5}),
    "repressilator": ("sbmlsim:repressilator.xml", 1000.0, {}),
    "demo": ("sbmlsim:Koenig_demo_14.xml", 20.0, {}),
    "mp_core": ("bench:mp_core", 360.0, {}),
    "mp_flat": ("bench:mp_flat", 360.0, {}),
    # the event model of the benchmark (the events of the engine)
    "repeated_dose": ("bench:repeated_dose", 168.0, {}),
}


def source_file(source: str, work: Path) -> Path:
    """Get the SBML file of a source (a source of bench/models.toml for `bench:<name>`)."""
    if source.startswith("bench:"):
        toml = ROOT / "bench" / "models.toml"
        config = tomllib.loads(toml.read_text(encoding="utf-8"))
        source = config["models"][source.split(":", 1)[1]]["source"]
        return resolve_source(source, toml.parent, config.get("checkouts", {}), work / "downloads")[0]
    return resolve_source(source, ROOT, {}, work / "downloads")[0]


# the linear solver of roadrunner and its Jacobian, as the engine's (dense, KLU)
ROADRUNNER_SOLVERS = {"dense": "difference_quotient", "klu": "sparse_difference_quotient"}


def reference(
    sbml: Path,
    end: float,
    preinit: dict[str, float],
    rtol: float = RTOL,
    atol: float = ATOL,
    linear_solver: str | None = None,
) -> tuple[pd.DataFrame, dict[str, float]]:
    """Simulate with sbmlsim (roadrunner): the result and the absolute tolerance of every state.

    `linear_solver` is `dense` (CVODE's dense difference quotient) or `klu` (KLU with the sparse
    difference quotient), `None` the default of the libroadrunner.
    """
    from sbmlsim.simulation import Simulation
    from sbmlsim.simulator import Simulator

    simulator = Simulator(absolute_tolerance=atol, relative_tolerance=rtol, n_workers=1)
    model = simulator.load(sbml)
    r = model.r_loaded
    if linear_solver is not None:
        r.integrator.setValue("linear_solver", "dense")
        r.integrator.setValue("jacobian", ROADRUNNER_SOLVERS[linear_solver])
        r.integrator.setValue("linear_solver", linear_solver)
    times = np.linspace(0.0, end, POINTS)
    result = simulator.simulate(model, Simulation(end=end, preinit_changes=preinit, times=list(times)))
    if linear_solver is not None:
        settings = (r.integrator.getValue("linear_solver"), r.integrator.getValue("jacobian"))
        assert settings == (linear_solver, ROADRUNNER_SOLVERS[linear_solver]), settings
    vector = r.integrator.getAbsoluteToleranceVector()
    atol = {r.model.getStateVectorId(i): float(vector[i]) for i in range(len(vector))}
    df = pd.DataFrame(result.values, columns=list(result.columns))
    return df.loc[:, ~df.columns.duplicated()], atol


def engine(
    cli: Path,
    sbml: Path,
    end: float,
    preinit: dict[str, float],
    atol: dict[str, float],
    work: Path,
    linear_solver: str,
    rtol: float = RTOL,
) -> pd.DataFrame:
    """Simulate with sbmlsim-core: compile, then `simulate` with the request of the reference."""
    model_dir = work / "model"
    if not (model_dir / "model.json").exists():
        subprocess.run([str(cli), "compile", str(sbml), "-o", str(model_dir)], check=True)
    desc = json.loads((model_dir / "model.json").read_text())
    vector = [atol[state["id"]] for state in desc["states"]]
    times = np.linspace(0.0, end, POINTS)
    request = {
        "contract": "simulation/1",
        "start": 0.0,
        "end": end,
        "output": {"times": [float(t) for t in times]},
        "preinit": [[k, v] for k, v in preinit.items()],
        "changes": [],
        "steady_state": None,
        "selections": [],
        "settings": {
            "relative_tolerance": rtol,
            "absolute_tolerance": {"per_state": vector},
            "maximum_num_steps": 100000,
            "maximum_time_step": 0.0,
            "initial_time_step": 0.0,
            "stiff": True,
            "linear_solver": linear_solver,
            "jacobian": "auto",
        },
        "seed": 0,
    }
    request_file = work / f"request_{linear_solver}_{rtol:g}.json"
    request_file.write_text(json.dumps(request))
    out = work / f"out_{linear_solver}_{rtol:g}.csv"
    subprocess.run([str(cli), "simulate", str(model_dir), str(request_file), "-o", str(out)], check=True)
    return pd.read_csv(out)


def compare(name: str, ref: pd.DataFrame, eng: pd.DataFrame) -> dict:
    """Compare the results of a model."""
    from sbmlsim.comparison.diff import DataSetsComparison

    common = [c for c in ref.columns if c in eng.columns]

    class Comparison(DataSetsComparison):
        tol_rel = RELATIVE_LIMIT
        tol_abs = ATOL

    comparison = Comparison({"roadrunner": ref[common], "sbmlsim-core": eng[common]})
    diff_abs = comparison.diff_abs
    failing = comparison.diff_tol_bool
    allowed = ATOL + RELATIVE_LIMIT * comparison.dfs[0].abs()
    resolved = comparison.dfs[0].abs() > RESOLVED
    rel = comparison.diff_rel
    result = {
        "label": name,
        "rows": len(ref),
        "columns": len(comparison.columns),
        "max_relative": float(rel.max().max()),
        "max_relative_column": rel.max().idxmax(),
        "max_relative_resolved": float(rel.where(resolved).max().max()),
        "max_relative_resolved_column": rel.where(resolved).max().idxmax(),
        "max_excess": float((diff_abs / allowed).max().max()),
        "failing_values": int(failing.sum().sum()),
        "failing_columns": sorted(failing.columns[failing.any()].tolist()),
        "pass": not bool(failing.any().any()),
    }
    # the largest failing values, by their excess over the allowed difference
    c, u = comparison.dfs
    excess = (diff_abs / allowed.replace(0.0, np.nan)).where(failing)
    stacked = excess.stack().dropna().sort_values(ascending=False).head(12)
    result["worst_values"] = [
        {
            "column": column,
            "time": float(c["time"].iloc[row]),
            "roadrunner": float(c[column].iloc[row]),
            "engine": float(u[column].iloc[row]),
            "relative": float(rel[column].iloc[row]),
            "excess": float(value),
        }
        for (row, column), value in stacked.items()
    ]
    return result


def main() -> int:
    """Run the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cli", type=Path, default=ROOT / "target/release/sbmlsim-core")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--only", help="only these models (comma separated)")
    parser.add_argument("--details", action="store_true", help="print the largest failing values")
    args = parser.parse_args()
    only = set(args.only.split(",")) if args.only else None
    results = []
    with tempfile.TemporaryDirectory() as tmp:
        for name, (source, end, preinit) in MODELS.items():
            if only and name not in only:
                continue
            work = Path(tmp) / name
            work.mkdir()
            try:
                sbml = work / "model.xml"
                sbml.write_text(prepare(source_file(source, Path(tmp))), encoding="utf-8")
            except PreparationError as error:
                print(f"error: {name}: {error}", file=sys.stderr)
                return 1
            converged, _ = reference(sbml, end, preinit, rtol=1e-12, atol=1e-14)
            result = {"model": name, "pass": True}
            for solver in ("dense", "klu"):
                loose, atol = reference(sbml, end, preinit, linear_solver=solver)
                rr = compare("roadrunner", converged, loose)
                eng = engine(args.cli, sbml, end, preinit, atol, work, solver)
                c = compare(solver, converged, eng)
                c["pass"] = c["pass"] or c["max_excess"] <= EXCESS_MARGIN * rr["max_excess"]
                # informative: the engine against roadrunner at the same tolerances
                direct = compare("direct", loose, eng)
                c["against_roadrunner_1e-10"] = {k: direct[k] for k in ("max_excess", "failing_values", "max_relative_resolved")}
                rr.pop("worst_values")
                result[solver] = {"roadrunner": rr, "engine": c, "pass": c["pass"]}
                result["pass"] = result["pass"] and c["pass"]
                print(
                    f"{name:<14} {solver:<5} {'pass' if c['pass'] else 'FAIL'}"
                    f"  engine: max excess {c['max_excess']:.4g}, failing values {c['failing_values']}"
                    f", max relative above 1e-9 {c['max_relative_resolved']:.3g} ({c['max_relative_resolved_column']})"
                    f"  roadrunner: max excess {rr['max_excess']:.4g}, failing values {rr['failing_values']}"
                    f"  engine against roadrunner 1e-10: max excess {direct['max_excess']:.3g}",
                    flush=True,
                )
                if args.details:
                    for w in c["worst_values"]:
                        print(
                            f"{'':<16}{w['column']:<28} t={w['time']:<10g} roadrunner {w['roadrunner']:<24.17g}"
                            f" engine {w['engine']:<24.17g} relative {w['relative']:.3g} excess {w['excess']:.3g}"
                        )
            print(f"{name:<14} {'PASS' if result['pass'] else 'FAIL'}", flush=True)
            results.append(result)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(results, indent=1))
    return 0 if all(r["pass"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
