"""The fixtures of the oracle tests of sbmlsim-stats, computed once with numpy and scipy.

Every module of the crate has a generator here, which returns its cases, and the script writes
them as `<module>.json` next to itself:

    {"_versions": {...}, "<case>": {"inputs": {...}, "expected": ...}, ...}

The numbers are written by `repr`, so a double is read back as the double it was; the numbers
which JSON cannot hold are the strings "nan", "inf" and "-inf". The fixtures are committed, the
Rust tests read them (`tests/oracle.rs`); the script runs in a throwaway environment:

    uv run --no-project --with numpy --with scipy python crates/stats/oracles/make_oracles.py

A module given as argument writes only its fixture (`... make_oracles.py special`).
"""

import json
import math
import sys
from collections.abc import Callable
from fractions import Fraction
from pathlib import Path
from typing import Any

import numpy as np
import scipy
from scipy import special as sc
from scipy import stats

HERE = Path(__file__).resolve().parent


def encode(value: Any) -> Any:
    """The value with numpy types as python ones and non-finite floats as strings."""
    if isinstance(value, dict):
        return {key: encode(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [encode(item) for item in value]
    if isinstance(value, np.ndarray):
        return encode(value.tolist())
    if isinstance(value, (bool, np.bool_)):
        return bool(value)
    if isinstance(value, (int, np.integer)):
        return int(value)
    if isinstance(value, (float, np.floating)):
        value = float(value)
        if math.isnan(value):
            return "nan"
        if math.isinf(value):
            return "inf" if value > 0 else "-inf"
        return value
    return value


def grid(**axes: list[float]) -> dict[str, list[float]]:
    """Every combination of the values of the axes, an equally long list per axis."""
    names = list(axes)
    mesh = np.meshgrid(
        *(np.asarray(axes[name], dtype=float) for name in names), indexing="ij"
    )
    return {
        name: values.ravel().tolist() for name, values in zip(names, mesh, strict=True)
    }


# --- special --------------------------------------------------------------------------------

AB = [0.5, 1.0, 2.5, 10.0, 100.0]
X = [0.0, 1e-6, 1e-3, 0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99, 0.999, 1 - 1e-6, 1.0]
P = [1e-12, 1e-6, 0.025, 0.5, 0.975, 1 - 1e-9]
DF = [1.0, 2.0, 3.5, 10.0, 30.0, 1e6]
N = [1, 10, 100, 4096]
P_SUCCESS = [0.0, 1e-6, 0.01, 0.3, 0.5, 0.9, 1 - 1e-6, 1.0]


def special() -> dict[str, Any]:
    """The cases of `special`: scipy.special and scipy.stats on grids of their inputs."""
    cases: dict[str, Any] = {}

    inputs = grid(a=AB, b=AB)
    # a parameter large, where the difference of the logarithms of the gamma function cancels;
    # not (0.5, 1e6) or (5e5, 0.5), where betaln of scipy 1.18 is off by 1.5e-10 and 3.5e-11
    # relative (mpmath), the tests take those from mpmath
    for a, b in [(1e3, 1e3), (1e10, 0.5), (2.5, 1e12), (30.0, 4096.0)]:
        inputs["a"].append(a)
        inputs["b"].append(b)
    cases["ln_beta"] = {
        "inputs": inputs,
        "expected": sc.betaln(inputs["a"], inputs["b"]),
    }

    inputs = grid(a=AB, b=AB, x=X)
    cases["beta_inc"] = {
        "inputs": inputs,
        "expected": sc.betainc(inputs["a"], inputs["b"], inputs["x"]),
    }
    # a large and b at most a few: around the mean 1 - b/a, where a continued fraction is slow
    inputs = {"a": [], "b": [], "x": []}
    for a in [1e3, 1e5, 1e7, 1e9, 1e12]:
        for b in [0.5, 1.0, 2.5]:
            for c in [0.01, 0.1, 1.0, 3.0, 10.0, 30.0, 100.0]:
                inputs["a"].append(a)
                inputs["b"].append(b)
                inputs["x"].append(1.0 - c / a)
                # and the mirror, the small parameter first
                inputs["a"].append(b)
                inputs["b"].append(a)
                inputs["x"].append(c / a)
    cases["beta_inc_large"] = {
        "inputs": inputs,
        "expected": sc.betainc(inputs["a"], inputs["b"], inputs["x"]),
    }

    x = [-38.0, -37.5, -30.0, -20.0, -10.0, -8.0, -5.0, -3.0, -2.0, -1.5, -1.0, -0.7]
    x += [
        -0.5,
        -0.1,
        -1e-3,
        -1e-8,
        -1e-300,
        0.0,
        1e-300,
        1e-8,
        1e-3,
        0.1,
        0.5,
        0.7,
        1.0,
    ]
    x += [1.5, 2.0, 3.0, 5.0, 8.0, 8.5, 10.0, 40.0, math.inf, -math.inf, math.nan]
    cases["normal_cdf"] = {"inputs": {"x": x}, "expected": stats.norm.cdf(x)}

    p = [
        0.0,
        1e-300,
        1e-100,
        1e-20,
        *P,
        0.1,
        0.3,
        0.4999,
        0.5001,
        0.7,
        0.9,
        1 - 1e-16,
        1.0,
    ]
    p += [-0.1, 1.1, math.nan]
    cases["normal_ppf"] = {"inputs": {"p": p}, "expected": stats.norm.ppf(p)}

    t = [-1e6, -1e3, -40.0, -10.0, -5.0, -2.0, -1.0, -0.5, -1e-3, 0.0]
    t += [1e-3, 0.5, 1.0, 2.0, 5.0, 10.0, 40.0, 1e3, 1e6]
    inputs = grid(x=t, df=DF)
    cases["t_cdf"] = {
        "inputs": inputs,
        "expected": stats.t.cdf(inputs["x"], inputs["df"]),
    }
    # many degrees of freedom, close to the normal distribution
    inputs = grid(x=[-30.0, -8.0, -2.0, -0.5, 0.5, 2.0, 8.0], df=[1e8, 1e10])
    cases["t_cdf_large_df"] = {
        "inputs": inputs,
        "expected": stats.t.cdf(inputs["x"], inputs["df"]),
    }

    inputs = grid(p=[*P, 0.1, 0.3, 0.7, 0.9], df=DF)
    cases["t_ppf"] = {
        "inputs": inputs,
        "expected": stats.t.ppf(inputs["p"], inputs["df"]),
    }

    inputs: dict[str, list] = {"k": [], "n": [], "p": []}
    for n in N:
        for p_success in P_SUCCESS:
            if n <= 100:
                ks = list(range(n + 1))
            else:
                mean = n * p_success
                sd = math.sqrt(n * p_success * (1 - p_success))
                around = [round(mean + sd * z) for z in np.arange(-8.0, 8.5, 0.5)]
                ks = sorted(
                    {k for k in [0, 1, 2, *around, n - 2, n - 1, n] if 0 <= k <= n}
                )
            for k in ks:
                inputs["k"].append(k)
                inputs["n"].append(n)
                inputs["p"].append(p_success)
    cases["binom_cdf"] = {
        "inputs": inputs,
        "expected": stats.binom.cdf(inputs["k"], inputs["n"], inputs["p"]),
    }

    q = [0.0, *P, 0.1, 0.9, 1.0, -0.1, 1.1, math.nan]
    inputs = {"q": [], "n": [], "p": []}
    for n in N:
        for p_success in P_SUCCESS:
            for quantile in q:
                inputs["q"].append(quantile)
                inputs["n"].append(n)
                inputs["p"].append(p_success)
    cases["binom_ppf"] = {
        "inputs": inputs,
        "expected": stats.binom.ppf(inputs["q"], inputs["n"], inputs["p"]),
    }
    # q at a value of the distribution function, the exact one (of the decimal probability)
    # and the one of scipy, where a rounding decides, and just above it
    inputs = {"q": [], "n": [], "p": []}
    for n in [1, 2, 3, 10, 20]:
        for p_success in ["0.1", "0.2", "0.25", "0.3", "0.5", "0.7", "0.75", "0.9"]:
            exact_p = Fraction(p_success)
            for k in range(n):
                exact = sum(
                    math.comb(n, i) * exact_p**i * (1 - exact_p) ** (n - i)
                    for i in range(k + 1)
                )
                computed = float(stats.binom.cdf(k, n, float(p_success)))
                for quantile in sorted(
                    {float(exact), computed, float(exact) * (1 + 1e-12)}
                ):
                    inputs["q"].append(quantile)
                    inputs["n"].append(n)
                    inputs["p"].append(float(p_success))
    cases["binom_ppf_ties"] = {
        "inputs": inputs,
        "expected": stats.binom.ppf(inputs["q"], inputs["n"], inputs["p"]),
    }
    return cases


GENERATORS: dict[str, Callable[[], dict[str, Any]]] = {
    "special": special,
}


def main(modules: list[str]) -> None:
    """Write the fixtures of `modules`, of every module without one."""
    for module in modules or list(GENERATORS):
        cases = GENERATORS[module]()
        fixture = {"_versions": {"numpy": np.__version__, "scipy": scipy.__version__}}
        fixture.update(cases)
        text = json.dumps(encode(fixture), allow_nan=False, indent=1)
        (HERE / f"{module}.json").write_text(text + "\n")
        print(f"{module}.json: {len(cases)} cases")


if __name__ == "__main__":
    main(sys.argv[1:])
