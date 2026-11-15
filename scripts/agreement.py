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

    uv run scripts/agreement.py [--cli target/release/sbmlsim-core] [--json FILE]

The packaged models of sbmlsim (`HCTZ_SBML`, `MIDAZOLAM_SBML`, `REPRESSILATOR_SBML`,
`DEMO_SBML`) and the machine perfusion models of the benchmark (`mp_core`, `mp_flat`, sources
of `bench/models.toml`) are prepared with `scripts/prepare_model.py` (comp flattened, distrib
stripped, L3V2) and simulated with both: sbmlsim's `Simulator` (roadrunner, its semantics of the
pre-initialization, which the engine follows) and `sbmlsim-core simulate` (wasmtime), at the
relative tolerance 1e-10 and the absolute tolerance 1e-12, which sbmlsim scales per state; the
engine gets the very vector roadrunner integrates with (by the ids of the state vector). The
output is 201 uniform times. Every column both have (the selections roadrunner makes by default)
is compared with `sbmlsim.comparison.diff.DataSetsComparison` (roadrunner the reference) and a
model fails when a value differs by more than 1e-7 relative, `|c - u| <= 1e-7 |c| + abs_tol`,
`abs_tol` the absolute tolerance of the integrator scaled by the largest magnitude of the column
(a value at the level of the absolute tolerance is not resolved by either integrator). The
largest relative difference `2 |c - u| / (|c| + |u|)` is reported over all values and over the
values above 1e-6 of the largest magnitude of their column.
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
    # the event model of the benchmark (not one of the brief, the events of the engine)
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


def reference(
    sbml: Path, end: float, preinit: dict[str, float], rtol: float = RTOL, atol: float = ATOL
) -> tuple[pd.DataFrame, dict[str, float]]:
    """Simulate with sbmlsim (roadrunner): the result and the absolute tolerance of every state."""
    from sbmlsim.simulation import Simulation
    from sbmlsim.simulator import Simulator

    simulator = Simulator(absolute_tolerance=atol, relative_tolerance=rtol, n_workers=1)
    model = simulator.load(sbml)
    times = np.linspace(0.0, end, POINTS)
    result = simulator.simulate(model, Simulation(end=end, preinit_changes=preinit, times=list(times)))
    r = model.r_loaded
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
    rtol: float = RTOL,
) -> pd.DataFrame:
    """Simulate with sbmlsim-core: compile, then `simulate` with the request of the reference."""
    model_dir = work / "model"
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
            "linear_solver": "auto",
            "jacobian": "auto",
        },
        "seed": 0,
    }
    (work / "request.json").write_text(json.dumps(request))
    out = work / f"out_{rtol:g}.csv"
    subprocess.run([str(cli), "simulate", str(model_dir), str(work / "request.json"), "-o", str(out)], check=True)
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
        "model": name,
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


def convergence(
    cli: Path, sbml: Path, end: float, preinit: dict[str, float], ref: pd.DataFrame, eng: pd.DataFrame, work: Path
) -> dict[str, dict]:
    """Compare both at the tolerances of the agreement with both at rtol 1e-12 and atol 1e-14."""
    tight, atol = reference(sbml, end, preinit, rtol=1e-12, atol=1e-14)
    eng_tight = engine(cli, sbml, end, preinit, atol, work, rtol=1e-12)
    pairs = {
        "roadrunner 1e-10 / roadrunner 1e-12": (tight, ref),
        "sbmlsim-core 1e-10 / roadrunner 1e-12": (tight, eng),
        "sbmlsim-core 1e-12 / roadrunner 1e-12": (tight, eng_tight),
        "sbmlsim-core 1e-10 / sbmlsim-core 1e-12": (eng_tight, eng),
    }
    out = {}
    for label, (a, b) in pairs.items():
        c = compare(label, a, b)
        c.pop("worst_values")
        out[label] = c
    return out


def main() -> int:
    """Run the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cli", type=Path, default=ROOT / "target/release/sbmlsim-core")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--only", help="only these models (comma separated)")
    parser.add_argument("--details", action="store_true", help="print the largest failing values")
    parser.add_argument(
        "--convergence",
        action="store_true",
        help="also compare both at 1e-10 with both at rtol 1e-12, atol 1e-14 (the integration error)",
    )
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
            ref, atol = reference(sbml, end, preinit)
            eng = engine(args.cli, sbml, end, preinit, atol, work)
            result = compare(name, ref, eng)
            results.append(result)
            print(
                f"{name:<14} {'PASS' if result['pass'] else 'FAIL'}  columns {result['columns']:4d}"
                f"  max relative {result['max_relative']:.3g} ({result['max_relative_column']})"
                f"  above 1e-9 {result['max_relative_resolved']:.3g} ({result['max_relative_resolved_column']})"
                f"  max |c-u| / (atol + 1e-7 |c|) {result['max_excess']:.3g}"
                f"  failing values {result['failing_values']}",
                flush=True,
            )
            if args.convergence:
                result["convergence"] = convergence(args.cli, sbml, end, preinit, ref, eng, work)
                for label, c in result["convergence"].items():
                    print(
                        f"    {label:<34} max relative above 1e-9 {c['max_relative_resolved']:.3g}"
                        f" ({c['max_relative_resolved_column']})  max excess {c['max_excess']:.3g}"
                        f"  failing values {c['failing_values']}"
                    )
            if args.details:
                for w in result["worst_values"]:
                    print(
                        f"    {w['column']:<28} t={w['time']:<10g} roadrunner {w['roadrunner']:<24.17g}"
                        f" engine {w['engine']:<24.17g} relative {w['relative']:.3g} excess {w['excess']:.3g}"
                    )
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(results, indent=1))
    return 0 if all(r["pass"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
