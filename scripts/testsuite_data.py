# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Write the result of a run of the SBML Test Suite as the data of the documentation.

`sbmlsim-core testsuite testdata/cases --json run.json` writes the result of every
case; this turns it into `docs/data/testsuite.json`, which the grid of the page
[Test suite](../docs/testsuite.md) reads. The file is committed and is written by
nothing at build time, as the tables of the benchmarks are.

    uv run scripts/testsuite_data.py run.json --baseline testdata/testsuite_baseline.json
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

# the status of a case as the grid shows it, in the order of the legend
KINDS: dict[str, str] = {
    "pass": "pass",
    "tolerance": "fail",
    "simulation_error": "fail",
    "missing_variable": "fail",
    "repeat": "fail",
    "not_compiled": "unsupported",
}


def _rounded(value: float | None) -> float | None:
    """Round a difference to two digits, which is all the tooltip shows.

    Args:
        value: The largest relative difference of a case, or `None`.

    Returns:
        The rounded value, or `None`.
    """
    return None if value is None else float(f"{value:.2g}")


def build(run: dict[str, Any], baseline: dict[str, Any]) -> dict[str, Any]:
    """Build the data of the grid from a run and the baseline.

    Args:
        run: What `--json` of the test suite runner wrote.
        baseline: The baseline, which carries the reason of every expected failure.

    Returns:
        The commit, the counts and one entry per case: its identifier, its kind,
        its status and, where there is one, its difference and its reason.
    """
    reasons = {
        cid: entry.get("reason", "")
        for cid, entry in baseline.get("expected_failures", {}).items()
    }
    cases = []
    for result in run["results"]:
        status = result["status"]
        case: dict[str, Any] = {
            "id": result["cid"],
            "kind": KINDS.get(status, "fail"),
            "status": status,
        }
        # a field which says nothing is left out, the page reads it as absent
        difference = _rounded(result.get("max_difference"))
        if difference is not None:
            case["difference"] = difference
        reason = reasons.get(result["cid"]) or result.get("message") or ""
        if reason:
            case["reason"] = reason
        cases.append(case)
    cases.sort(key=lambda case: case["id"])
    counts = dict.fromkeys(("pass", "fail", "unsupported"), 0)
    for case in cases:
        counts[case["kind"]] += 1
    return {
        "suite_commit": baseline["suite_commit"],
        "n_cases": run["n_cases"],
        "counts": counts,
        "cases": cases,
    }


def main() -> int:
    """Run the command line tool.

    Returns:
        The exit code, 0.
    """
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("run", type=Path, help="the JSON of `testsuite --json`")
    parser.add_argument(
        "--baseline", type=Path, default=Path("testdata/testsuite_baseline.json")
    )
    parser.add_argument("--out", type=Path, default=Path("docs/data/testsuite.json"))
    arguments = parser.parse_args()

    data = build(
        json.loads(arguments.run.read_text(encoding="utf-8")),
        json.loads(arguments.baseline.read_text(encoding="utf-8")),
    )
    arguments.out.parent.mkdir(parents=True, exist_ok=True)
    arguments.out.write_text(
        json.dumps(data, separators=(",", ":")) + "\n", encoding="utf-8"
    )
    counts = data["counts"]
    print(
        f"{arguments.out}: {data['n_cases']} cases, "
        f"{counts['pass']} pass, {counts['fail']} fail, "
        f"{counts['unsupported']} unsupported"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
