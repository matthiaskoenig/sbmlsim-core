"""The fixture of the statistics in JavaScript, `stats.json`.

The calls of `sbmlsim_core.stats` and what python gives for them, natively, which
`js/test/stats.test.mjs` holds `compiler.wasm` to bit for bit.

The calls are built from the inputs of the oracle fixtures of the crate
(`crates/stats/oracles/<module>.json`), with the draws, the group means and the errors of every
kind on top. A call is `{"name", "function", "args", "kwargs"}` and either `"result"` or
`"error": {"kind", "message"}`; the required arguments of python are `args`, every optional or
keyword-only one is in `kwargs`, as the JavaScript functions take them (positional, then an
object of options). A double which JSON cannot hold is `{"$": "nan"}`, `{"$": "inf"}` or
`{"$": "-inf"}`, an integer past the safe integers of JavaScript `{"bigint": "<digits>"}`
(a `BigInt` there), every other number is written by `repr`, so it reads back as the double it
was, `-0.0` included. The fixture is committed; after a change of the crate, write it again
with the python package of this checkout:

    .venv/bin/python js/test/fixtures/make_stats.py
"""

import json
import math
from collections.abc import Callable
from pathlib import Path
from typing import Any

import numpy as np
from sbmlsim_core import stats

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
ORACLES = ROOT / "crates" / "stats" / "oracles"


def decode(value: Any) -> Any:
    """The inputs of an oracle fixture with its strings of non-finite floats as floats."""
    if isinstance(value, dict):
        return {key: decode(item) for key, item in value.items()}
    if isinstance(value, list):
        return [decode(item) for item in value]
    if value in ("nan", "inf", "-inf"):
        return float(value)
    return value


def encode(value: Any) -> Any:
    """A value of python as JSON: arrays as nested lists, non-finite floats tagged."""
    if isinstance(value, np.ndarray):
        return encode(value.tolist())
    if isinstance(value, (np.floating, np.integer, np.bool_)):
        return encode(value.item())
    if isinstance(value, dict):
        return {key: encode(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [encode(item) for item in value]
    if isinstance(value, int) and not isinstance(value, bool) and abs(value) > 2**53:
        return {"bigint": str(value)}
    if isinstance(value, float) and not math.isfinite(value):
        return {"$": "nan" if math.isnan(value) else ("inf" if value > 0 else "-inf")}
    return value


CALLS: list[dict[str, Any]] = []


def call(name: str, function: str, *args: Any, **kwargs: Any) -> None:
    """Calls the function of `sbmlsim_core.stats` and records the call with its outcome."""
    entry: dict[str, Any] = {
        "name": name,
        "function": function,
        "args": encode(list(args)),
        "kwargs": encode(kwargs),
    }
    try:
        entry["result"] = encode(getattr(stats, function)(*args, **kwargs))
    except (ValueError, OverflowError, ZeroDivisionError, IndexError) as error:
        entry["error"] = {"kind": type(error).__name__, "message": str(error)}
    CALLS.append(entry)


def oracle(module: str) -> dict[str, Any]:
    """The cases of an oracle fixture by their names, their inputs decoded."""
    fixture = json.loads((ORACLES / f"{module}.json").read_text())
    return {
        name: decode(case["inputs"])
        for name, case in fixture.items()
        if not name.startswith("_") and isinstance(case, dict) and "inputs" in case
    }


def cases(module: str, prefix: str) -> list[tuple[str, dict[str, Any]]]:
    """The cases of a module whose names start with the prefix and a slash."""
    return [
        (name, inputs)
        for name, inputs in oracle(module).items()
        if name == prefix or name.startswith(prefix + "/")
    ]


def matrix(values: list[float], rows: int, columns: int) -> list[list[float]]:
    """Row-major values as the rows of a matrix."""
    return [values[i * columns : (i + 1) * columns] for i in range(rows)]


def metrics() -> None:
    """The metrics of a fit and the criteria of its evaluation."""
    for name, x in cases("metrics", "residuals"):
        residuals, k = x["x"], x["k"]
        call(name, "sse", residuals)
        call(name, "mse", residuals)
        call(name, "nrmse", residuals)
        try:
            mse = stats.mse(residuals)
        except ValueError:
            continue
        call(name, "rmse_from_mse", mse)
        call(name, "aic_from_mse", mse, len(residuals), k)
        call(name, "bic_from_mse", mse, len(residuals), k)
    for name, x in cases("metrics", "from_mse"):
        for i, (mse, n, k) in enumerate(zip(x["mse"], x["n"], x["k"], strict=True)):
            if i % 7:
                continue
            call(f"{name}/{i}", "rmse_from_mse", mse)
            call(f"{name}/{i}", "aic_from_mse", mse, n, k)
            call(f"{name}/{i}", "bic_from_mse", mse, n, k)
    for name, x in cases("metrics", "r_squared"):
        call(name, "r_squared", x["y"], x["f"])
    for name, x in cases("metrics", "fold"):
        call(name, "fold_counts", x["y"], x["f"], fold=x["fold"])
        call(name, "fold_accepted", x["y"], x["f"], fold=x["fold"])
    for name, x in cases("metrics", "sd"):
        call(name, "sd_accepted", x["y"], x["f"], x["sd"], k=x["k"])
    for name, x in cases("metrics", "fold_errors"):
        call(name, "gmfe", x["y"], x["f"])
        call(name, "afe", x["y"], x["f"])
        call(name, "subset_agreement", x["y"], x["f"])
    for name, x in cases("metrics", "guest_limits"):
        call(name, "guest_limits", x["ratio"])
        call(f"{name}/matrix", "guest_limits", matrix(x["ratio"][:20], 4, 5))
        call(f"{name}/scalar", "guest_limits", 2.0)
    for name, x in cases("metrics", "within_guest"):
        call(name, "within_guest", x["observed"], x["predicted"])
    call("within_guest/shapes", "within_guest", [1.0, 2.0], [1.0])
    call(
        "within_guest/matrix_not_positive",
        "within_guest",
        [[1.0, -2.0], [3.0, 4.0]],
        [[1.0, 2.0], [3.0, 4.0]],
    )


def fisher() -> None:
    """The Fisher information and the posterior."""
    for name, x in cases("fisher", "fisher"):
        jacobian = matrix(x["j"], x["rows"], x["k"])
        call(name, "gram", jacobian)
        information = stats.gram(jacobian).tolist()
        call(
            name,
            "fisher_statistics",
            information,
            x["cost"],
            x["n"],
            likelihood=x["likelihood"],
        )
        se = stats.fisher_statistics(
            information, x["cost"], x["n"], likelihood=x["likelihood"]
        )["standard_errors"]
        kwargs = {"level": 1.0 - x["alpha"]}
        if x["dof"] is not None:
            kwargs["dof"] = x["dof"]
        call(name, "fisher_intervals", x["values"], se, x["scales"], **kwargs)
        call(name, "fisher_relative_errors", x["values"], se, x["scales"])
    for name, x in cases("fisher", "posterior"):
        precision = matrix(x["precision"], x["k"], x["k"])
        call(
            name,
            "posterior_laplace",
            precision,
            pids=x["pids"],
            opid=x["opid"],
            rank_tolerance=x["rank_tolerance"],
        )
        call(f"{name}/no_pids", "posterior_laplace", precision)
        for scale in x["scales"]:
            call(name, "log_sd", scale, 0.3)
        call(name, "lognormal_cv", 0.3)
    for name, x in cases("fisher", "rank_correlation"):
        r = matrix(x["r"], x["k"], x["k"])
        call(name, "rank_correlation", r)
        call(name, "normal_correlation", r)
    call("gram/vector", "gram", [1.0, 2.0])
    call("fisher_statistics/not_square", "fisher_statistics", [[1.0, 2.0]], 1.0, 3)
    call("fisher_intervals/scale", "fisher_intervals", [1.0], [0.1], ["LOGX"])
    call("log_sd/scale", "log_sd", "ln", 0.3)
    call("rank_correlation/vector", "rank_correlation", [1.0, 0.5])


def uncertainty() -> None:
    """The statistics of the draws."""
    qs = [0.0, 0.05, 0.5, 0.95, 1.0]
    for name, x in cases("uncertainty", "statistics"):
        call(name, "mean_se", x["x"])
        call(name, "sd_se", x["x"])
        call(name, "quantiles", x["x"], qs)
        for q in (0.05, 0.5, 0.95):
            call(name, "quantile", x["x"], q)
            call(name, "quantile_ci", x["x"], q)
    for name, x in cases("uncertainty", "quantile_ci"):
        call(name, "quantile_ci", x["x"], x["q"], level=x["level"])
    for name, x in cases("uncertainty", "quantiles"):
        call(name, "quantiles", x["x"], x["qs"])
    for name, x in cases("uncertainty", "first_order"):
        kwargs = {} if x["n_bins"] is None else {"n_bins": x["n_bins"]}
        call(name, "first_order_given_data", x["x"], x["y"], **kwargs)
    for name, x in cases("uncertainty", "precise"):
        call(name, "precise", x["values"], x["rel_tol"])
    for name, x in cases("uncertainty", "interval"):
        call(
            name,
            "interval_parameters",
            x["low"],
            x["high"],
            level=x["level"],
            lognormal=x["lognormal"],
        )


def draws() -> None:
    """The draws and their correlations."""
    # a matrix of no rows is `[]` in JSON, of the shape (0,) and not (0, 0), as in JavaScript
    for name, x in cases("draws", "nearest"):
        if "m" not in x:
            continue
        k = x["k"]
        m = matrix(x["m"], k, k) if k else []
        call(
            name,
            "nearest_correlation",
            m,
            max_iterations=x["max_iterations"],
            tol=x["tol"],
        )
    for name, x in cases("draws", "is_psd"):
        k = x["k"]
        m = matrix(x["m"], k, k) if k else []
        call(name, "is_psd", m, rtol=x["rtol"])
    for name, x in cases("draws", "validate"):
        if x["shape"] == [0, 0]:
            continue
        m = np.asarray(x["matrix"], dtype=float).reshape(x["shape"])
        kwargs: dict[str, Any] = {"kind": x["kind"]}
        if x["units"] is not None:
            # the oracle counts the units, which is all the validation reads of them
            kwargs["units"] = ["mg"] * x["units"]
        call(name, "validate_covariance", x["targets"], m, **kwargs)
    for name, x in cases("draws", "rho"):
        call(
            name,
            "normal_score_rho",
            x["m"],
            tuple(x["a"]),
            tuple(x["b"]),
            scale=x["scale"],
        )
    call(
        "rho/family", "normal_score_rho", 0.1, ("gamma", 1.0, 1.0), ("normal", 1.0, 1.0)
    )
    for name, x in cases("draws", "copula"):
        u = matrix(x["u"], x["n"], x["d"])
        call(name, "copula", u, matrix(x["r"], x["d"], x["d"]))
    call("copula/vector", "copula", [0.5, 0.5], [[1.0]])
    call("copula/size", "copula", [[0.5, 0.5]], [[1.0]])
    for design in ("sobol", "lhs", "random"):
        for d, n, seed in ((3, 16, 1), (2, 7, 12345), (0, 4, 1), (2, 0, 1)):
            call(
                f"unit_points/{design}/{d}x{n}",
                "unit_points",
                d,
                n,
                design=design,
                seed=seed,
            )
        call(
            f"unit_points/{design}/start",
            "unit_points",
            2,
            8,
            design=design,
            seed=3,
            start=5,
        )
        call(
            f"unit_points/{design}/negative_d",
            "unit_points",
            -1,
            4,
            design=design,
            seed=1,
        )
        call(
            f"unit_points/{design}/negative_n",
            "unit_points",
            2,
            -4,
            design=design,
            seed=1,
        )
        call(
            f"unit_points/{design}/negative_start",
            "unit_points",
            2,
            4,
            design=design,
            seed=1,
            start=-2,
        )
    call("unit_points/default", "unit_points", 2, 8, seed=7)
    call("unit_points/large_seed", "unit_points", 2, 4, design="random", seed=2**64 - 1)
    call("unit_points/negative_seed", "unit_points", 2, 4, seed=-1)
    call("unit_points/design", "unit_points", 2, 4, design="halton", seed=1)
    call("unit_points/sobol_dimensions", "unit_points", 3000, 1, seed=1)
    u = stats.unit_points(2, 64, seed=1)
    call("copula/sobol", "copula", u, [[1.0, 0.8], [0.8, 1.0]])


def testing() -> None:
    """The probability statements."""
    for name, x in cases("testing", "sequence"):
        # the sequences of a formula of the oracles are not repeated here
        if "satisfied" not in x or len(x["satisfied"]) > 400:
            continue
        satisfied = x["satisfied"]
        p0, margin, alpha = x["p0"], x["margin"], x["alpha"]
        sides = {
            side: stats.p_values(satisfied, p0, side, margin=margin)
            for side in ("greater", "less", "equivalent")
        }
        for side in sides:
            call(name, "p_values", satisfied, p0, side, margin=margin)
        call(name, "confidence_sequence", satisfied, level=1.0 - alpha)
        call(name, "decide", sides["greater"], alpha)
        call(
            name,
            "decision",
            sides["greater"],
            sides["less"],
            sides["equivalent"],
            alpha,
        )
        estimate = float(np.mean(satisfied)) if satisfied else 0.5
        call(name, "needed", estimate, len(satisfied), p0, margin, alpha)
    for name, x in cases("testing", "sequence_alpha"):
        call(name, "confidence_sequence", x["satisfied"], level=1.0 - x["alpha"])
    for name, x in cases("testing", "needed"):
        if name.startswith("needed_error"):
            call(
                name, "needed", x["estimate"], x["n"], x["p0"], x["margin"], x["alpha"]
            )
            continue
        columns = zip(
            x["estimate"], x["n"], x["p0"], x["margin"], x["alpha"], strict=True
        )
        for i, (estimate, n, p0, margin, alpha) in enumerate(columns):
            if i % 5 == 0:
                call(f"{name}/{i}", "needed", estimate, n, p0, margin, alpha)
    for name, x in cases("testing", "family_level"):
        for level, m in zip(x["level"], x["m"], strict=True):
            call(f"{name}/{level}/{m}", "family_level", level, m)
    for name, x in cases("testing", "bands"):
        distribution = matrix(x["distribution"], x["rows"], x["cols"])
        call(name, "bands", distribution, x["qs"])
    call("p_values/side", "p_values", [True, False], 0.5, "more")
    call("p_values/numbers", "p_values", [1.0, 0.0, float("nan"), -0.0], 0.5, "greater")
    call("bands/vector", "bands", [1.0, 2.0], [0.5])
    rng = np.random.default_rng(1)
    predictions = rng.lognormal(size=(12, 3))
    predictions[2, 1] = np.nan
    for geometric in (False, True):
        for size, groups in ((1, 5), (4, 6), (3, 0)):
            call(
                f"group_means/{geometric}/{size}x{groups}",
                "group_means",
                predictions,
                size,
                groups,
                geometric=geometric,
                seed=11,
            )
    call("group_means/negative", "group_means", predictions, -2, 3, seed=1)
    call("group_means/vector", "group_means", [1.0, 2.0], 2, 3, seed=1)
    call("group_means/no_points", "group_means", np.zeros((4, 0)), 2, 3, seed=1)


def errors() -> None:
    """An error of every kind which is not in the cases of the oracles above."""
    call("sd_se/infinite_variance", "sd_se", [1e154, -1e154, 3e154])
    call(
        "interval_parameters/level_zero",
        "interval_parameters",
        1.0,
        2.0,
        level=0.0,
        lognormal=False,
    )
    call("first_order/index", "first_order_given_data", [1.0], [1.0, 2.0, 3.0])
    call(
        "interval_parameters/overflow",
        "interval_parameters",
        1.0,
        1e308,
        lognormal=True,
    )


GENERATORS: list[Callable[[], None]] = [
    metrics,
    fisher,
    uncertainty,
    draws,
    testing,
    errors,
]


def main() -> None:
    """Writes `stats.json` next to this script."""
    for generator in GENERATORS:
        generator()
    names = [entry["name"] + "/" + entry["function"] for entry in CALLS]
    fixture = {
        "_comment": "written by make_stats.py, see there",
        "calls": CALLS,
    }
    text = json.dumps(fixture, separators=(",", ":"), allow_nan=False)
    (HERE / "stats.json").write_text(text + "\n")
    print(f"{len(CALLS)} calls ({len(set(names))} distinct), {len(text) / 1024:.0f} kB")


if __name__ == "__main__":
    main()
