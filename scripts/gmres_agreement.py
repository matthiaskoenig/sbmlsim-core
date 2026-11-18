"""Check exported benchmark requests against a tighter KLU solution.

Export the model and requests with `sbmlsim-core bench ... --solvers klu,gmres
--export target/bench-gmres`, then run:

    python3 scripts/gmres_agreement.py target/bench-gmres/perfusion \
        --json target/bench-gmres/agreement.json

This compares the selected state amounts, not derived reaction rates. The reference
uses rtol 1e-13 and absolute tolerances 1e-5 times the benchmark tolerances.
The report records accuracy at benchmark tolerances, rtol 1e-10 and rtol 1e-12.
The comparison criterion is 1e-12 + 1e-7 |reference|, as in scripts/agreement.py.
The exit code checks the converged runs at rtol 1e-12; the other runs show how
global error decreases as tolerances tighten.
"""

from __future__ import annotations

import argparse
import copy
import csv
import hashlib
import json
import math
import subprocess
from pathlib import Path


def run(
    cli: Path, model: Path, request: dict, label: str
) -> tuple[list[str], list[list[float]]]:
    """Save the exact request and return its CSV as numbers."""
    request_file = model / f"agreement_{label}.json"
    result_file = model / f"agreement_{label}.csv"
    request_file.write_text(json.dumps(request), encoding="utf-8")
    subprocess.run(
        [str(cli), "simulate", str(model), str(request_file), "-o", str(result_file)],
        check=True,
    )
    with result_file.open(newline="", encoding="utf-8") as stream:
        reader = csv.reader(stream)
        columns = next(reader)
        return columns, [[float(value) for value in row] for row in reader]


def main() -> int:
    """Run the convergence comparison and save its measurements."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--cli", type=Path, default=Path("target/release/sbmlsim-core"))
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    cli = args.cli.resolve()
    base = json.loads((args.model / "klu_analytic_times.json").read_text())
    tolerance = base["settings"]["absolute_tolerance"]
    if "per_state" in tolerance:
        atols = tolerance["per_state"]
    else:
        atols = [tolerance["scalar"]] * (len(base["selections"]) - 1)
    description = json.loads((args.model / "model.json").read_text())
    expected = ["time", *(state["id"] for state in description["states"])]
    if not atols or base["selections"] != expected or len(atols) != len(expected) - 1:
        parser.error("request must select time followed by every model state")

    def request(solver: str, jacobian: str, rtol: float, scale: float) -> dict:
        result = copy.deepcopy(base)
        result["settings"].update(
            linear_solver=solver,
            jacobian=jacobian,
            relative_tolerance=rtol,
            absolute_tolerance={"per_state": [a * scale for a in atols]},
            maximum_num_steps=100000,
        )
        return result

    reference_request = request("klu", "analytic", 1e-13, 1e-5)
    columns, reference = run(cli, args.model, reference_request, "reference")
    if columns != expected or not reference:
        parser.error("reference must contain output rows for every model state")
    results = []
    converged_pass = True
    for accuracy, rtol, scale in [
        ("benchmark", base["settings"]["relative_tolerance"], 1.0),
        ("tight", 1e-10, 1e-2),
        ("converged", 1e-12, 1e-4),
    ]:
        for solver, jacobian in [
            ("klu", "analytic"),
            ("gmres", "analytic"),
            ("gmres", "difference_quotient"),
        ]:
            label = f"{solver}_{jacobian}_{accuracy}"
            names, rows = run(
                cli, args.model, request(solver, jacobian, rtol, scale), label
            )
            assert names == columns and len(rows) == len(reference)
            worst, failing, max_absolute = {}, 0, 0.0
            worst_excess = -1.0
            for row, ref in zip(rows, reference, strict=True):
                assert row[0] == ref[0]
                for j in range(1, len(columns)):
                    error = abs(row[j] - ref[j])
                    allowed = 1e-12 + 1e-7 * abs(ref[j])
                    excess = error / allowed
                    if not math.isfinite(error):
                        raise ValueError(
                            f"nonfinite output: {label}, {columns[j]}, t={row[0]}"
                        )
                    failing += excess > 1.0
                    max_absolute = max(max_absolute, error)
                    if excess > worst_excess:
                        worst_excess = excess
                        worst = {
                            "column": columns[j],
                            "time": row[0],
                            "excess": excess,
                            "value": row[j],
                            "reference": ref[j],
                        }
            result = {
                "label": label,
                "pass": failing == 0,
                "failing_values": failing,
                "max_absolute": max_absolute,
                "worst": worst,
            }
            results.append(result)
            if accuracy == "converged":
                converged_pass &= failing == 0
            print(f"{label}: max excess {worst_excess:.6g}, failing values {failing}")
    report = {
        "model_sha256": hashlib.sha256(
            (args.model / "model.wasm").read_bytes()
        ).hexdigest(),
        "rows": len(reference),
        "states": len(columns) - 1,
        "reference_settings": reference_request["settings"],
        "results": results,
    }
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return int(not converged_pass)


if __name__ == "__main__":
    raise SystemExit(main())
