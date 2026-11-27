"""The fixtures of the oracle tests of sbmlsim-stats, computed once with numpy and scipy.

Every module of the crate has a generator here, which returns its cases, and the script writes
them as `<module>.json` next to itself:

    {"_versions": {...}, "<case>": {"inputs": {...}, "expected": ...}, ...}

The numbers are written by `repr`, so a double is read back as the double it was; the numbers
which JSON cannot hold are the strings "nan", "inf" and "-inf", and a call which raised is
`{"error": "<the message>"}`. The fixtures are committed, the Rust tests read them
(`tests/oracle.rs`), and the tests of the python bindings read the ones of the functions they
bind (`python/tests/test_stats.py`). The script runs in a throwaway environment, and the
modules take two:

- `special` and `linalg` call numpy and scipy only. Without an argument the script writes
  these two:

      uv run --no-project --with numpy --with scipy \
          python crates/stats/oracles/make_oracles.py

- `metrics` calls the functions of sbmlsim itself (`sbmlsim.fit.metrics` and
  `sbmlsim.fit.evaluation`), so its environment has sbmlsim as well, with the python and
  the package index of libroadrunner sbmlsim needs:

      uv run --no-project --python 3.14 \
          --index https://matthiaskoenig.github.io/roadrunner-wheels/simple/ \
          --index-strategy unsafe-best-match \
          --with sbmlsim==0.53.0 --with numpy --with scipy \
          python crates/stats/oracles/make_oracles.py metrics

Modules given as arguments write only their fixtures (`... make_oracles.py special`); a
module of sbmlsim in an environment without it stops before any fixture is written.
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


# --- linalg ---------------------------------------------------------------------------------


def _factor(r: np.ndarray) -> np.ndarray:
    """The factor of `sbmlsim.uncertainty.draws._factor`: Cholesky, else the eigen one."""
    try:
        return np.linalg.cholesky(r)
    except np.linalg.LinAlgError:
        values, vectors = np.linalg.eigh(r)
        return vectors * np.sqrt(np.clip(values, 0.0, None))


def _symmetric_case(a: np.ndarray, *, full: bool = True) -> dict[str, Any]:
    """The case of a symmetric matrix: numpy's eigh, pinv, cholesky and draws' factor.

    The eigenvalues are written largest first and the eigenvectors in the same order, the
    columns of a row-major matrix. Not `full` keeps only the eigenvalues of a large matrix,
    whose tests check the other outputs by identities (`A A+ A = A`, `L L^T = A`).
    The Cholesky factor is null for a matrix which is not positive definite and left out of
    a singular one, where the rounding decides whether LAPACK finds a positive last pivot.
    """
    k = a.shape[0]
    assert np.array_equal(a, a.T)
    values, vecs = np.linalg.eigh(a)
    expected: dict[str, Any] = {"eigenvalues": values[::-1]}
    if not full:
        return {"inputs": {"k": k, "a": a.ravel()}, "expected": expected}
    # np.linalg.pinv of sbmlsim.fit.fisher, its default cut 1e-15 times the largest
    expected["pinv"] = np.linalg.pinv(a).ravel()
    expected["vectors"] = vecs[:, ::-1].ravel()
    smallest = float(values[0])
    if smallest > 1e-8 * float(np.abs(values).max()):
        expected["cholesky"] = np.linalg.cholesky(a).ravel()
    elif smallest < -1e-8 * float(np.abs(values).max()):
        expected["cholesky"] = None
    f = _factor(a)
    expected["factor_product"] = (f @ f.T).ravel()
    return {"inputs": {"k": k, "a": a.ravel()}, "expected": expected}


def linalg() -> dict[str, Any]:
    """The cases of `linalg`: numpy.linalg on small, singular, indefinite and large matrices."""
    rng = np.random.default_rng(20261127)
    cases: dict[str, Any] = {}

    cases["one"] = _symmetric_case(np.array([[2.5]]))
    cases["two"] = _symmetric_case(np.array([[2.0, 1.0], [1.0, 3.0]]))
    # rank 2 of 3, distinct eigenvalues: B B^T of integers, exactly symmetric
    b = np.array([[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]])
    cases["rank_deficient"] = _symmetric_case(b @ b.T)
    x = rng.standard_normal((10, 15))
    a = x @ x.T / 15.0 + 0.1 * np.eye(10)
    cases["spd_10"] = _symmetric_case((a + a.T) / 2.0)
    # integers, so the 200 x 200 matrix is short in the fixture; a Wishart matrix of 210
    # degrees of freedom, its condition about 1e4
    x = rng.integers(-9, 10, size=(200, 210)).astype(float)
    cases["spd_200"] = _symmetric_case(x @ x.T, full=False)
    y = rng.standard_normal((5, 5))
    a = (y + y.T) / 2.0
    assert np.linalg.eigvalsh(a)[0] < 0.0
    cases["indefinite_5"] = _symmetric_case(a)

    j = rng.standard_normal((40, 6))
    cases["gram"] = {
        "inputs": {"n": 40, "k": 6, "j": j.ravel()},
        "expected": {
            "gram": (j.T @ j).ravel(),
            "pinv": np.linalg.pinv(j.T @ j).ravel(),
        },
    }
    # a parameter which the data do not tell from another one: a column twice
    j = rng.standard_normal((30, 4))
    j[:, 3] = j[:, 1]
    cases["gram_rank_deficient"] = {
        "inputs": {"n": 30, "k": 4, "j": j.ravel()},
        "expected": {
            "gram": (j.T @ j).ravel(),
            "pinv": np.linalg.pinv(j.T @ j).ravel(),
        },
    }
    return cases


# --- metrics --------------------------------------------------------------------------------


def outcome(function: Callable[..., Any], *args: Any, **kwargs: Any) -> Any:
    """The value of a call, `{"error": <message>}` for one which raised a ValueError."""
    try:
        return function(*args, **kwargs)
    except ValueError as error:
        return {"error": str(error)}


def _with_specials(values: np.ndarray, rng: np.random.Generator) -> np.ndarray:
    """The values with a not a number, a zero, a negative and an infinite one at random places."""
    values = values.copy()
    places = rng.choice(values.size, size=4, replace=False)
    values[places] = [np.nan, 0.0, -1.5, np.inf]
    return values


def large_residuals(n: int) -> np.ndarray:
    """Get the residuals `0..n` of a formula of integers and exact operations.

    `(((i * 2654435761) % 1000003) / 1000003 - 0.5) * 2**(i % 31 - 15)`: the tests of the
    Rust and of the python bindings compute them bit for bit as here instead of reading
    them, and their squares over 60 binary orders of magnitude make a sum in blocks of 8192
    differ from the pairwise one of the whole array.
    """
    i = np.arange(n, dtype=np.int64)
    a = (i * 2654435761) % 1000003
    return (a.astype(float) / 1000003.0 - 0.5) * np.ldexp(
        1.0, (i % 31 - 15).astype(int)
    )


def metrics() -> dict[str, Any]:
    """The cases of `metrics`: the functions of `sbmlsim.fit.metrics` and `.evaluation`.

    The kernels of the metrics on residuals (`sse`, `mse`, `rmse`, the AIC and the BIC of the
    MSE, the NRMSE of `evaluation._rms`), the R², the fold criteria, the uncertainty criterion,
    the fold errors and the agreement of a subset and the criterion of Guest et al., on random
    inputs of a seed and on the edges: empty, one value, zeros, not a number, infinite,
    negative, the bounds of a criterion and the arguments the python refuses.
    """
    import pandas as pd
    from sbmlsim.fit import evaluation
    from sbmlsim.fit import metrics as m

    rng = np.random.default_rng(20261127)
    cases: dict[str, Any] = {}

    # the metrics of residuals, k = 3 fitted parameters
    residuals: dict[str, np.ndarray] = {
        "empty": np.array([]),
        "one": np.array([0.7]),
        "zeros": np.zeros(5),
        "negative_zero": np.array([-0.0]),
        "not_a_number": np.array([1.0, np.nan, 2.0]),
        "infinite": np.array([1.0, np.inf]),
        # the squares overflow to inf and underflow to 0, an MSE of 0
        "overflow": np.array([1e200, 1e200]),
        "underflow": np.array([1e-200, 1e-200, 1e-200]),
        # an array of integers, which the python converts to doubles
        "integers": np.arange(-3, 6),
    }
    for n in (2, 7, 8, 9, 100, 1000):
        # magnitudes over a few decades, so an order of the sum other than numpy's shows
        residuals[f"random_{n}"] = rng.standard_normal(n) * np.exp(
            3.0 * rng.standard_normal(n)
        )
    for label, x in residuals.items():
        mse = outcome(m.mse, x)
        expected: dict[str, Any] = {
            "sse": m.sse(x),
            "mse": mse,
            "rmse": outcome(m.rmse, x),
            "aic": outcome(m.aic, x, 3),
            "bic": outcome(m.bic, x, 3),
            "nrmse": evaluation._rms(pd.Series(x, dtype=float)),
        }
        cases[f"residuals/{label}"] = {"inputs": {"x": x, "k": 3}, "expected": expected}

    # residuals of more than a block of the pairwise sum and of more than the 8192 values
    # of a buffer of numpy, an odd number of them too: the inputs are the formula of
    # `large_residuals`, the sums numpy's, which a sum in blocks of 8192 would not give
    for n in (20000, 100003):
        x = large_residuals(n)
        squares = np.square(x)
        blocked = 0.0
        for start in range(0, n, 8192):
            blocked += float(np.sum(squares[start : start + 8192]))
        assert blocked != m.sse(x), "the case does not tell the sums apart"
        cases[f"large_residuals/{n}"] = {
            "inputs": {"n": n, "k": 3},
            "expected": {
                "sse": m.sse(x),
                "mse": m.mse(x),
                "nrmse": evaluation._rms(pd.Series(x, dtype=float)),
                "first": x[:4],
            },
        }

    # the functions of the MSE over a grid of their arguments, the refused ones included
    mses = [-1.0, -1e-05, -1e300, -np.inf, -0.0, 0.0, 1e-300, 0.5, 2.0, np.inf, np.nan]
    ns = [-1, 0, 1, 10, 1000]
    ks = [0, 3, -2]
    grid_mse, grid_n, grid_k = (
        list(axis)
        for axis in zip(*((a, b, c) for a in mses for b in ns for c in ks), strict=True)
    )
    cases["from_mse"] = {
        "inputs": {"mse": grid_mse, "n": grid_n, "k": grid_k},
        "expected": {
            "rmse": [outcome(m.rmse_from_mse, a) for a in grid_mse],
            "aic": [
                outcome(m.aic_from_mse, a, b, c)
                for a, b, c in zip(grid_mse, grid_n, grid_k, strict=True)
            ],
            "bic": [
                outcome(m.bic_from_mse, a, b, c)
                for a, b, c in zip(grid_mse, grid_n, grid_k, strict=True)
            ],
        },
    }

    # R²
    y = rng.lognormal(0.0, 1.0, 25)
    pairs: dict[str, tuple[np.ndarray, np.ndarray]] = {
        "random": (y, y * rng.lognormal(0.0, 0.2, 25)),
        "negative": (y, y[::-1].copy()),
        "perfect": (y, y.copy()),
        "empty": (np.array([]), np.array([])),
        "one": (np.array([2.0]), np.array([1.0])),
        "two": (np.array([2.0, 3.0]), np.array([2.5, 2.5])),
        "constant": (np.full(4, 3.0), np.array([1.0, 2.0, 3.0, 4.0])),
        "not_a_number": (np.array([1.0, np.nan, 3.0]), np.array([1.0, 2.0, 3.0])),
        "lengths": (np.array([1.0, 2.0, 3.0]), np.array([1.0, 2.0])),
        "integers": (np.array([1, 2, 4, 7]), np.array([2, 2, 3, 9])),
    }
    for label, (y_obs, y_pred) in pairs.items():
        cases[f"r_squared/{label}"] = {
            "inputs": {"y": y_obs, "f": y_pred},
            "expected": outcome(m.r_squared, y_obs, y_pred),
        }

    # the fold criteria: the counts of `fold_counts` and the acceptance of `Fold`
    y = _with_specials(rng.lognormal(0.0, 1.0, 40), rng)
    f = _with_specials(y * rng.lognormal(0.0, 0.5, 40), rng)
    folds: dict[str, tuple[np.ndarray, np.ndarray, float]] = {
        "random_1.25": (y, f, 1.25),
        "random_1.5": (y, f, 1.5),
        "random_2": (y, f, 2.0),
        # the bounds, inside: twofold exactly, and just outside
        "bounds": (
            np.array([3.0, 3.0, 0.1, 0.1, 7.0, 7.0]),
            np.array([6.0, 1.5, 0.2, 0.05, 14.000000000000002, 3.4999999999999996]),
            2.0,
        ),
        "empty": (np.array([]), np.array([]), 2.0),
        "fold_one": (y, f, 1.0),
        "fold_half": (y, f, 0.5),
        "fold_not_a_number": (y, f, float("nan")),
        "lengths": (np.array([1.0, 2.0]), np.array([1.0]), 2.0),
        "integers": (np.array([1, 2, 3, 4, 0, -2]), np.array([2, 2, 9, 1, 1, 1]), 2.0),
    }
    for label, (y_obs, y_pred, fold) in folds.items():
        counts = outcome(m.fold_counts, y_obs, y_pred, fold=fold)
        if isinstance(counts, dict):
            accepted = None
        else:
            points = pd.DataFrame({"DV": y_obs, "IPRED": y_pred})
            accepted = np.asarray(evaluation.Fold(fold).accepted(points))
        cases[f"fold/{label}"] = {
            "inputs": {"y": y_obs, "f": y_pred, "fold": fold},
            "expected": {"counts": counts, "accepted": accepted},
        }

    # the uncertainty criterion of `Uncertainty`
    y = rng.normal(10.0, 2.0, 40)
    f = y + rng.normal(0.0, 2.0, 40)
    sd = np.abs(rng.normal(1.5, 0.5, 40))
    sd[[0, 1, 2]] = [np.nan, 0.0, -1.0]
    y[3], f[4], f[5], sd[6] = np.nan, np.inf, np.nan, np.inf
    sds: dict[str, tuple[np.ndarray, np.ndarray, np.ndarray, float]] = {
        "random_1": (y, f, sd, 1.0),
        "random_2": (y, f, sd, 2.0),
        "random_half": (y, f, sd, 0.5),
        # |y - f| = k sd exactly, inside
        "bounds": (
            np.array([1.0, 1.0]),
            np.array([3.0, 3.5]),
            np.array([1.0, 1.0]),
            2.0,
        ),
        "empty": (np.array([]), np.array([]), np.array([]), 2.0),
    }
    for label, (y_obs, y_pred, y_sd, k) in sds.items():
        points = pd.DataFrame({"DV": y_obs, "IPRED": y_pred, "DV_SD": y_sd})
        cases[f"sd/{label}"] = {
            "inputs": {"y": y_obs, "f": y_pred, "sd": y_sd, "k": k},
            "expected": np.asarray(evaluation.Uncertainty(k).accepted(points)),
        }

    # the fold errors and the agreement of a subset, of the points with a ratio
    def valid(n: int) -> tuple[np.ndarray, np.ndarray]:
        y = rng.lognormal(0.0, 1.0, n)
        return y, y * rng.lognormal(0.1, 0.3, n)

    def invalid() -> tuple[np.ndarray, np.ndarray]:
        """Three points without a ratio."""
        return np.array([0.0, np.nan, 2.0]), np.array([1.0, 1.0, -1.0])

    ratio_sets: dict[str, tuple[np.ndarray, np.ndarray]] = {}
    for n in (1, 2, 9, 10, 50, 1000):
        y, f = valid(n)
        y_bad, f_bad = invalid()
        ratio_sets[f"random_{n}"] = (
            np.concatenate([y, y_bad]),
            np.concatenate([f, f_bad]),
        )
    ratio_sets["none"] = invalid()
    ratio_sets["empty"] = (np.array([]), np.array([]))
    y, f = valid(11)
    # a ratio which overflows to inf and one which underflows to 0
    ratio_sets["overflow"] = (np.append(y, 1e-300), np.append(f, 1e300))
    ratio_sets["underflow"] = (np.append(y, 1e300), np.append(f, 1e-300))
    with np.errstate(all="ignore"):
        for label, (y_obs, y_pred) in ratio_sets.items():
            points = pd.DataFrame({"DV": y_obs, "IPRED": y_pred})
            cases[f"fold_errors/{label}"] = {
                "inputs": {"y": y_obs, "f": y_pred},
                "expected": {
                    "gmfe": evaluation.gmfe(points),
                    "afe": evaluation.afe(points),
                    "subset_agreement": evaluation.subset_agreement(points),
                },
            }

    # the limits of Guest et al.
    ratio = np.array(
        [
            0.01,
            0.1,
            0.5,
            0.8,
            0.999,
            1.0,
            1.001,
            1.25,
            2.0,
            5.0,
            10.0,
            100.0,
            1e6,
            1e300,
        ]
    )
    # the edges: tiny, zero, negative and not finite ratios
    ratio = np.append(
        ratio, [1e-300, 5e-324, 0.0, -0.0, -1.0, -0.5, np.nan, np.inf, -np.inf]
    )
    with np.errstate(all="ignore"):
        low, high = evaluation.guest_limits(ratio)
    cases["guest_limits"] = {
        "inputs": {"ratio": ratio},
        "expected": {"low": low, "high": high},
    }

    obs = 1.0 + rng.lognormal(0.0, 1.0, 30)
    obs[:10] = 1.0 / obs[:10]
    pred = obs * rng.lognormal(0.0, 0.4, 30)
    pred[[3, 7]] = [np.nan, np.inf]
    limits = np.array([0.5, 2.0, 4.0, 1.0])
    low, high = evaluation.guest_limits(limits)
    guests: dict[str, tuple[Any, Any]] = {
        "random": (obs, pred),
        "at_the_limits": (
            np.concatenate([limits, limits]),
            np.concatenate([low, high]),
        ),
        "outside": (limits, high * (1.0 + 1e-9)),
        "all_not_finite": (np.array([1.5, 2.0]), np.array([np.nan, -np.inf])),
        "empty": (np.array([]), np.array([])),
        "observed_zero": (np.array([1.0, 0.0]), np.array([1.0, 1.0])),
        "observed_not_a_number": (np.array([np.nan, 2.0]), np.array([1.0, 1.0])),
        "observed_negative": (np.array([-1.5, 2.0]), np.array([1.0, 1.0])),
        # the numbers of a message as python writes them
        "observed_repr": (
            np.array([-1e-05, 1e16, 123456789012345.67, 0.0001, -np.inf, 1e-300]),
            np.ones(6),
        ),
        "lengths": (np.array([1.0, 2.0]), np.array([1.0])),
        "integers": (np.array([2, 1, 3]), np.array([2, 1, 9])),
        # a scalar, which the python writes as one in the message (the python bindings only)
        "scalar": (-1.0, 1.0),
    }
    for label, (observed, predicted) in guests.items():
        cases[f"within_guest/{label}"] = {
            "inputs": {"observed": observed, "predicted": predicted},
            "expected": outcome(evaluation.within_guest, observed, predicted),
        }
    assert m.sse([]) == 0.0
    assert evaluation.MIN_POINTS_SUBSET_AGREEMENT == 10
    return cases


GENERATORS: dict[str, Callable[[], dict[str, Any]]] = {
    "special": special,
    "linalg": linalg,
    "metrics": metrics,
}

#: the modules whose fixture is of the functions of sbmlsim, whose version it records
OF_SBMLSIM = {"metrics"}


def main(modules: list[str]) -> None:
    """Write the fixtures of `modules`, without one of the modules of numpy and scipy only.

    The modules of sbmlsim (`OF_SBMLSIM`) are written only when they are named, and only in
    an environment with sbmlsim: the script stops before it writes a fixture otherwise.
    """
    modules = modules or [module for module in GENERATORS if module not in OF_SBMLSIM]
    unknown = [module for module in modules if module not in GENERATORS]
    if unknown:
        sys.exit(
            f"no module {', '.join(unknown)}, the modules are {', '.join(GENERATORS)}"
        )
    sbmlsim_version = ""
    if any(module in OF_SBMLSIM for module in modules):
        try:
            import sbmlsim
        except ImportError:
            sys.exit(
                f"the fixtures of {', '.join(sorted(OF_SBMLSIM & set(modules)))} call "
                "sbmlsim, which this environment does not have: run the script in the "
                "environment of sbmlsim of the docstring of this script"
            )
        sbmlsim_version = sbmlsim.__version__
    for module in modules:
        cases = GENERATORS[module]()
        versions = {"numpy": np.__version__, "scipy": scipy.__version__}
        if module in OF_SBMLSIM:
            versions["sbmlsim"] = sbmlsim_version
        fixture = {"_versions": versions}
        fixture.update(cases)
        text = json.dumps(encode(fixture), allow_nan=False, indent=1)
        (HERE / f"{module}.json").write_text(text + "\n")
        print(f"{module}.json: {len(cases)} cases")


if __name__ == "__main__":
    main(sys.argv[1:])
