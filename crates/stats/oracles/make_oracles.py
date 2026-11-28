"""The fixtures of the oracle tests of sbmlsim-stats, computed once with numpy and scipy.

Every module of the crate has a generator here, which returns its cases, and the script writes
them as `<module>.json` next to itself:

    {"_versions": {...}, "<case>": {"inputs": {...}, "expected": ...}, ...}

The numbers are written by `repr`, so a double is read back as the double it was; the numbers
which JSON cannot hold are the strings "nan", "inf" and "-inf", and a call which raised is
`{"error": "<the message>"}`. The fixtures are committed, the Rust tests read them
(`tests/oracle_<module>.rs`), and the tests of the python bindings read the ones of the
functions they bind (`python/tests/test_stats.py`). The script runs in a throwaway environment, and the
modules take two:

- `special` and `linalg` call numpy and scipy only. Without an argument the script writes
  these two:

      uv run --no-project --with numpy --with scipy \
          python crates/stats/oracles/make_oracles.py

- `metrics`, `fisher`, `uncertainty`, `draws` and `testing` call the functions of sbmlsim
  itself (`sbmlsim.fit.metrics`, `sbmlsim.fit.evaluation`, `sbmlsim.fit.fisher`,
  `sbmlsim.fit.posterior`, `sbmlsim.uncertainty.statistics`, `_precise` of
  `sbmlsim.uncertainty.propagation`, `Uncertain.interval`,
  `sbmlsim.uncertainty.covariance`, `sbmlsim.uncertainty.draws` and
  `sbmlsim.uncertainty.testing`), so their environment has sbmlsim as well, with the
  python and the package index of libroadrunner sbmlsim needs:

      uv run --no-project --python 3.14 \
          --index https://matthiaskoenig.github.io/roadrunner-wheels/simple/ \
          --index-strategy unsafe-best-match \
          --with sbmlsim==0.53.0 --with numpy --with scipy \
          python crates/stats/oracles/make_oracles.py metrics fisher uncertainty draws testing

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


# --- fisher ---------------------------------------------------------------------------------


def _fisher_case(
    j: np.ndarray,
    *,
    cost: float,
    likelihood: bool,
    scales: list[str],
    values: np.ndarray,
    n: int | None = None,
    alpha: float = 0.95,
) -> tuple[dict[str, Any], Any]:
    """The case of a jacobian: the statistics of `FisherInformation` of its `J' J`.

    The input `information` is the matrix of numpy (`jac.T @ jac` of `fisher_information`),
    `j` the jacobian it is of; `dof` the degrees of freedom of the intervals, `null` for a
    likelihood, whose intervals are the ones of the normal distribution.
    """
    from sbmlsim.fit.fisher import DEFAULT_RANK_TOLERANCE, FisherInformation
    from sbmlsim.fit.options import ObjectiveType, ParameterScaleType

    rows, k = j.shape
    n = rows if n is None else n
    objective = ObjectiveType.LIKELIHOOD if likelihood else ObjectiveType.LEAST_SQUARES
    matrix = j.T @ j
    fisher = FisherInformation(
        opid="op",
        sid="estimate",
        pids=[f"p{i}" for i in range(k)],
        values=np.asarray(values, dtype=float),
        scale=ParameterScaleType.LINEAR,
        matrix=matrix,
        cost=cost,
        n=n,
        alpha=alpha,
        scales=[ParameterScaleType[name] for name in scales],
        objective=objective,
    )
    assert fisher.rank_tolerance == DEFAULT_RANK_TOLERANCE == 1e-8
    with np.errstate(all="ignore"):
        lower, upper = fisher.confidence_intervals()
        case = {
            "inputs": {
                "rows": rows,
                "k": k,
                "j": j.ravel(),
                "information": matrix.ravel(),
                "cost": cost,
                "n": n,
                "likelihood": likelihood,
                "scales": scales,
                "values": values,
                "alpha": alpha,
                "dof": None if likelihood else n - k,
            },
            "expected": {
                "gram": matrix.ravel(),
                "sigma2": fisher.sigma2,
                "spectrum": fisher.eigenvalues,
                "rank": fisher.rank,
                "condition": fisher.condition_number,
                "covariance": fisher.covariance.ravel(),
                "standard_errors": fisher.standard_errors,
                "correlation": fisher.correlation.to_numpy().ravel(),
                "lower": lower,
                "upper": upper,
                "relative_errors": fisher.relative_errors,
            },
        }
    return case, fisher


class _Problem:
    """The parts of an `OptimizationProblem` `posterior_laplace` reads, of a Fisher information.

    No parameter has a prior, so the precision is the information divided by the variance of
    the residuals, and the bounds are wide.
    """

    def __init__(self, fisher: Any) -> None:
        from types import SimpleNamespace

        self.fisher = fisher
        self.opid = fisher.opid
        self.pids = list(fisher.pids)
        self.objective = fisher.objective
        self.parameters = [
            SimpleNamespace(pid=pid, prior=None, target_id=pid) for pid in self.pids
        ]
        self.scales_initialized = fisher.parameter_scales
        self.parameter_units = dict.fromkeys(self.pids)
        self.bounds = (
            np.array([1e-12 if s.is_log else -1e12 for s in self.scales_initialized]),
            np.full(len(self.pids), 1e12),
        )

    def initialize(self, settings: Any) -> None:
        """Nothing to initialize."""

    def bind_parameter_set(self, parameter_set: Any) -> Any:
        """The values of the information."""
        from types import SimpleNamespace

        return SimpleNamespace(x=lambda pids: self.fisher.values)

    def to_scale(self, values: Any) -> np.ndarray:
        """The values on the scales of the information."""
        return self.fisher.to_scale(values)

    def from_scale(self, values: Any) -> np.ndarray:
        """The values of the scales of the information in the units of the model."""
        return self.fisher.from_scale(values)


def _posterior_case(fisher: Any, *, matrix: np.ndarray | None = None) -> dict[str, Any]:
    """The case of the Laplace approximation of a Fisher information without priors.

    `posterior_laplace` of sbmlsim on a problem of the information (`_Problem`); the input
    `precision` is the one it decomposes, the information divided by the variance of the
    residuals (`matrix` instead of the information, unsymmetric, to see it symmetrized). The
    expected covariance or the message of the ValueError, and of the posterior its
    correlation, the rank and the normal correlation, the standard deviation of the logarithm
    (`_log_sd`), the coefficient of variation of a log-normal marginal (`distributions`, `nan`
    of a normal one) and the intervals of two levels.
    """
    from types import SimpleNamespace

    from sbmlsim.fit.fisher import DEFAULT_RANK_TOLERANCE
    from sbmlsim.fit.options import ParameterScaleType
    from sbmlsim.fit.posterior import _log_sd, posterior_laplace

    if matrix is not None:
        fisher.matrix = matrix
    precision = np.asarray(fisher.matrix, dtype=float) / fisher.sigma2
    names = [s.name for s in fisher.parameter_scales]
    case: dict[str, Any] = {
        "inputs": {
            "k": fisher.k,
            "precision": precision.ravel(),
            "rank_tolerance": DEFAULT_RANK_TOLERANCE,
            "opid": fisher.opid,
            "pids": fisher.pids,
            "scales": names,
            "values": fisher.values,
        }
    }
    # duck types of the problem, the settings it does not read and the parameter set
    problem: Any = _Problem(fisher)
    settings: Any = None
    parameter_set: Any = SimpleNamespace(sid=fisher.sid)
    try:
        posterior = posterior_laplace(problem, settings, parameter_set, fisher=fisher)
    except ValueError as error:
        case["expected"] = {"error": str(error)}
        return case
    sd = posterior.standard_deviations
    distributions: Any = posterior.distributions()
    cv = [
        distributions[pid].cv if scale is not ParameterScaleType.LINEAR else math.nan
        for pid, scale in zip(posterior.pids, posterior.scales, strict=True)
    ]
    lower95, upper95 = posterior.intervals(0.95)
    lower50, upper50 = posterior.intervals(0.5)
    case["expected"] = {
        "covariance": posterior.covariance.ravel(),
        "standard_deviations": sd,
        "correlation": posterior.correlation_matrix.ravel(),
        "rank_correlation": posterior.rank_correlation().ravel(),
        "normal_correlation": posterior.normal_correlation().matrix.ravel(),
        "log_sd": [
            _log_sd(scale, float(s))
            for scale, s in zip(posterior.scales, sd, strict=True)
        ],
        "cv": cv,
        "lower95": lower95,
        "upper95": upper95,
        "lower50": lower50,
        "upper50": upper50,
    }
    return case


def large_jacobian(rows: int, k: int) -> np.ndarray:
    """Get the jacobian `rows x k` of a formula of integers, of the digits -9 to 9.

    With `x = i * k + c`, `J[i, c] = (((x * x % 1000003) * 2654435761 + x) % 1000003) % 19 - 9`
    (the square, as a congruence linear in `x` gives a jacobian of rank 198): the tests of
    the Rust and of the python bindings compute it bit for bit as here instead of reading
    it, and its `J' J` is of integers below 2**53, so exactly numpy's in every order of the
    sum.
    """
    x = np.arange(rows * k, dtype=np.int64).reshape(rows, k)
    return ((((x * x) % 1000003 * 2654435761 + x) % 1000003) % 19 - 9).astype(float)


#: the rows of the covariance and the correlation of the case `fisher_large/200` stored
LARGE_ROWS = [0, 1, 99, 199]


def fisher() -> dict[str, Any]:
    """The cases of `fisher` and `posterior`: `FisherInformation` and `posterior_laplace`.

    The statistics of the Fisher information (`sigma2`, the eigenvalues, the rank, the
    condition number, the covariance, the standard errors, the correlation, the intervals
    and the relative errors) of random jacobians of a least squares fit and of a likelihood
    on the three scales, of rank deficient ones (a column a multiple of another, a column of
    zeros, eight parameters with two such columns), without degrees of freedom, of a perfect
    fit and of a cost which is not a number or overflows the intervals; the Laplace
    approximation of the posterior of some of them, of a direction which the data do not
    constrain and of matrices whose correlation the rank correlation clips.
    """
    from sbmlsim.fit.options import ObjectiveType, ParameterScaleType
    from sbmlsim.fit.posterior import ParameterPosterior

    rng = np.random.default_rng(20261127)
    cases: dict[str, Any] = {}
    fishers: dict[str, Any] = {}

    def add(label: str, j: np.ndarray, **kwargs: Any) -> None:
        cases[f"fisher/{label}"], fishers[label] = _fisher_case(j, **kwargs)

    def positive(k: int) -> np.ndarray:
        return np.exp(rng.standard_normal(k))

    j = rng.standard_normal((50, 4)) * np.array([1.0, 3.0, 0.2, 10.0])
    cost = float(0.5 * np.sum(rng.standard_normal(50) ** 2))
    mixed = ["LINEAR", "LOG", "LOG10", "LOG10"]
    add(
        "least_squares",
        j,
        cost=cost,
        likelihood=False,
        scales=["LINEAR"] * 4,
        values=rng.standard_normal(4),
    )
    add(
        "likelihood_log",
        j,
        cost=cost,
        likelihood=True,
        scales=["LOG"] * 4,
        values=positive(4),
    )
    add(
        "mixed_scales",
        j,
        cost=cost,
        likelihood=False,
        scales=mixed,
        values=positive(4),
        alpha=0.9,
    )
    add(
        "likelihood_log10",
        rng.standard_normal((30, 3)),
        cost=12.5,
        likelihood=True,
        scales=["LOG10"] * 3,
        values=positive(3),
        alpha=0.68,
    )
    # a parameter the data tell only together with another one, of a clear direction
    deficient = j.copy()
    deficient[:, 3] = 2.0 * deficient[:, 1]
    add(
        "rank_deficient",
        deficient,
        cost=cost,
        likelihood=False,
        scales=mixed,
        values=positive(4),
    )
    # a parameter the data do not depend on
    zero = j.copy()
    zero[:, 2] = 0.0
    add(
        "zero_column",
        zero,
        cost=cost,
        likelihood=False,
        scales=mixed,
        values=positive(4),
    )
    # eight parameters, two of them a combination of others
    eight = rng.standard_normal((50, 8))
    eight[:, 5] = eight[:, 0] - eight[:, 2]
    eight[:, 7] = 0.5 * eight[:, 3]
    add(
        "rank_deficient_8",
        eight,
        cost=cost,
        likelihood=False,
        scales=["LOG"] * 8,
        values=positive(8),
    )
    # as many and more parameters than data points
    square = rng.standard_normal((4, 4))
    add(
        "dof_zero",
        square,
        cost=1.5,
        likelihood=False,
        scales=mixed,
        values=positive(4),
    )
    add(
        "dof_negative",
        square[:3],
        cost=1.5,
        likelihood=False,
        scales=mixed,
        values=positive(4),
    )
    add(
        "dof_negative_likelihood",
        square,
        n=2,
        cost=1.5,
        likelihood=True,
        scales=mixed,
        values=positive(4),
    )
    add(
        "perfect_fit",
        j,
        cost=0.0,
        likelihood=False,
        scales=mixed,
        values=positive(4),
    )
    add(
        "cost_not_a_number",
        j,
        cost=math.nan,
        likelihood=False,
        scales=mixed,
        values=positive(4),
    )
    # standard errors of about 1e149, past the largest double on a logarithmic scale
    add(
        "overflow",
        j,
        cost=1e300,
        likelihood=False,
        scales=mixed,
        values=positive(4),
    )
    # values a logarithm cannot take, and a linear value of zero
    add(
        "values_not_positive",
        j,
        cost=cost,
        likelihood=False,
        scales=["LINEAR", "LOG", "LOG10", "LOG"],
        values=np.array([0.0, 0.0, -2.0, -0.5]),
    )
    add(
        "one",
        rng.standard_normal((10, 1)),
        cost=2.0,
        likelihood=False,
        scales=["LOG10"],
        values=positive(1),
    )

    def with_objective(label: str, objective: ObjectiveType) -> Any:
        fisher = fishers[label]
        fisher.objective = objective
        return fisher

    cases["posterior/likelihood_log"] = _posterior_case(fishers["likelihood_log"])
    cases["posterior/likelihood_log10"] = _posterior_case(fishers["likelihood_log10"])
    cases["posterior/least_squares_mixed"] = _posterior_case(fishers["mixed_scales"])
    cases["posterior/least_squares_linear"] = _posterior_case(fishers["least_squares"])
    # an information which is not exactly symmetric, symmetrized by the python
    fisher = _fisher_case(
        rng.standard_normal((40, 5)),
        cost=4.0,
        likelihood=True,
        scales=["LOG", "LINEAR", "LOG10", "LOG", "LINEAR"],
        values=positive(5),
    )[1]
    skew = rng.standard_normal((5, 5)) * 1e-6
    cases["posterior/unsymmetric"] = _posterior_case(
        fisher, matrix=fisher.matrix + skew * np.abs(fisher.matrix)
    )
    cases["posterior/weak_combination"] = _posterior_case(
        with_objective("rank_deficient", ObjectiveType.LIKELIHOOD)
    )
    small = j.copy()
    small[:, 0] *= 1e-5
    cases["posterior/weak_small"] = _posterior_case(
        _fisher_case(
            small, cost=cost, likelihood=True, scales=mixed, values=positive(4)
        )[1]
    )
    cases["posterior/weak_zero"] = _posterior_case(
        _fisher_case(
            np.zeros((5, 3)),
            cost=1.0,
            likelihood=True,
            scales=["LOG"] * 3,
            values=positive(3),
        )[1]
    )

    # 200 parameters of 260 data points, identifiable, its condition about 141: the jacobian
    # of `large_jacobian`, the covariance and the correlation of a few rows
    from sbmlsim.fit.fisher import FisherInformation

    rows, k = 260, 200
    large = large_jacobian(rows, k)
    information = large.T @ large
    assert np.array_equal(information, information.T)
    fisher = FisherInformation(
        opid="op",
        sid="estimate",
        pids=[f"p{i}" for i in range(k)],
        values=np.ones(k),
        scale=ParameterScaleType.LINEAR,
        matrix=information,
        cost=100.0,
        n=rows,
    )
    assert fisher.rank == k
    cases["fisher_large/200"] = {
        "inputs": {
            "rows": rows,
            "k": k,
            "cost": 100.0,
            "n": rows,
            "rows_stored": LARGE_ROWS,
        },
        "expected": {
            "sigma2": fisher.sigma2,
            "spectrum": fisher.eigenvalues,
            "rank": fisher.rank,
            "condition": fisher.condition_number,
            "standard_errors": fisher.standard_errors,
            "covariance_rows": fisher.covariance[LARGE_ROWS].ravel(),
            "correlation_rows": fisher.correlation.to_numpy()[LARGE_ROWS].ravel(),
        },
    }

    # the rank correlation of correlations past one and of not a number (a negative variance)
    covariances = {
        "past_one": np.array([[1.0, 2.0, 0.5], [2.0, 1.0, -3.0], [0.5, -3.0, 4.0]]),
        "negative_variance": np.array(
            [[1.0, 0.3, 0.1], [0.3, -2.0, 0.2], [0.1, 0.2, 0.0]]
        ),
    }
    for label, covariance in covariances.items():
        posterior = ParameterPosterior(
            opid="op",
            sid="estimate",
            pids=["a", "b", "c"],
            values=np.ones(3),
            scales=[ParameterScaleType.LINEAR] * 3,
            covariance=covariance,
            units=[None] * 3,
            targets=["a", "b", "c"],
            lower_bounds=np.zeros(3),
            upper_bounds=np.full(3, 10.0),
            objective=ObjectiveType.LIKELIHOOD,
        )
        with np.errstate(all="ignore"):
            cases[f"rank_correlation/{label}"] = {
                "inputs": {"k": 3, "r": posterior.correlation_matrix.ravel()},
                "expected": posterior.rank_correlation().ravel(),
            }
    return cases


# --- uncertainty ------------------------------------------------------------------------------


def raised(function: Callable[..., Any], *args: Any, **kwargs: Any) -> Any:
    """The value of a call, `{"error": <message>, "exception": <class>}` for one which raised.

    The statistics of the uncertainty raise more than a ValueError on the edges: an
    OverflowError of a power or an exponential past the largest double, a
    ZeroDivisionError and an IndexError of numpy, whose class the bindings raise as well.
    """
    try:
        return function(*args, **kwargs)
    except (ValueError, OverflowError, ZeroDivisionError, IndexError) as error:
        return {"error": str(error), "exception": type(error).__name__}


def paired_draws(n: int) -> tuple[np.ndarray, np.ndarray]:
    """Get the draws `x` and `y` of `n` pairs of the formula of `large_residuals`.

    `x = large_residuals(n)` and `y = x * x + large_residuals(n + 13)[13:]`, of exact
    operations of doubles: the tests compute them bit for bit as here.
    """
    x = large_residuals(n)
    return x, x * x + large_residuals(n + 13)[13:]


#: the probabilities of the quantiles and of their intervals of every input of the statistics
QUANTILE_QS = [0.05, 0.5, 0.95]
#: the probabilities of the quantiles of every input, the ends included
QUANTILES_QS = [0.0, 0.05, 0.25, 0.5, 0.95, 1.0]


def uncertainty() -> dict[str, Any]:
    """The cases of `uncertainty`: `sbmlsim.uncertainty.statistics` and its siblings.

    The mean, the standard deviation and the quantiles of draws with their errors
    (`mean_se`, `sd_se`, `quantile_ci`) and `np.nanquantile`, of 0, 1, 2, 7, 8 and 1000
    draws (the last of the formula of `large_residuals`), with not a number, infinite
    draws and without variance; the edges of `quantile_ci` (the probabilities 0 and 1,
    the levels 0 and 1 and the arguments numpy and scipy refuse); the first-order index
    from given data (`first_order_given_data`) of ties, too few pairs, an output
    without variance and of bins of every number; the precision of an output
    (`propagation._precise`); and the parameters of the distribution of a reported
    interval (`Uncertain.interval`, the median and the coefficient of variation of a
    log-normal, the mean and the standard deviation of a normal).
    """
    from sbmlsim.uncertainty import statistics as st
    from sbmlsim.uncertainty.propagation import _precise
    from sbmlsim.uncertainty.quantity import Uncertain

    rng = np.random.default_rng(20261127)
    cases: dict[str, Any] = {}

    def statistics_of(x: np.ndarray) -> dict[str, Any]:
        with np.errstate(all="ignore"):
            return {
                "mean_se": raised(st.mean_se, x),
                "sd_se": raised(st.sd_se, x),
                "quantile_ci": [raised(st.quantile_ci, x, q) for q in QUANTILE_QS],
                "quantiles": np.nanquantile(x, QUANTILES_QS)
                if np.any(~np.isnan(x))
                else [math.nan] * len(QUANTILES_QS),
            }

    draws: dict[str, np.ndarray] = {
        "empty": np.array([]),
        "one": np.array([0.7]),
        "two": np.array([0.7, -1.2]),
        "not_a_number_only": np.array([np.nan, np.nan]),
        "zero_variance": np.full(9, 2.5),
        "with_not_a_number": np.array([1.0, np.nan, 2.0, 4.0, np.nan, 8.0]),
        "infinite": np.array([1.0, np.inf, 2.0, 3.0]),
        "both_infinities": np.array([np.inf, 1.0, 2.0, -np.inf]),
        "infinite_only": np.array([np.inf, np.inf]),
        "one_infinite": np.array([np.inf]),
        # the fourth power of the standard deviation past the largest double, which
        # python refuses with an OverflowError, and a variance which overflows
        "overflow_power": np.array([0.0, 1e100, -1e100]),
        "overflow_variance": np.array([1e300, -1e300, 1e300]),
        "ties": np.array([3.0, 1.0, 3.0, 2.0, 3.0, 1.0, 2.0, 3.0]),
        "negative_zero": np.array([-0.0, 0.0, -0.0]),
    }
    for n in (7, 8):
        draws[f"random_{n}"] = rng.standard_normal(n) * np.exp(rng.standard_normal(n))
    draws["random_8_not_a_number"] = draws["random_8"].copy()
    draws["random_8_not_a_number"][[2, 5]] = np.nan
    for label, x in draws.items():
        cases[f"statistics/{label}"] = {
            "inputs": {"x": x},
            "expected": statistics_of(x),
        }
    for n in (1000, 100003):
        x = large_residuals(n)
        cases[f"statistics_formula/{n}"] = {
            "inputs": {"n": n, "first": x[:4]},
            "expected": statistics_of(x),
        }

    # the edges of the probability and of the level of the interval of a quantile
    edges = {
        "q0": (0.0, 0.95),
        "q1": (1.0, 0.95),
        "q_above": (1.5, 0.95),
        "q_below": (-0.1, 0.95),
        "q_not_a_number": (math.nan, 0.95),
        "level0": (0.5, 0.0),
        "level1": (0.5, 1.0),
        "level_above": (0.5, 1.5),
        "level_not_a_number": (0.5, math.nan),
        "level_minus_one": (0.5, -1.0),
        "level_negative": (0.5, -0.5),
        "level50": (0.3, 0.5),
        "level99": (0.99, 0.99),
    }
    for size in (0, 3, 10):
        x = rng.standard_normal(size)
        for label, (q, level) in edges.items():
            cases[f"quantile_ci/{size}_{label}"] = {
                "inputs": {"x": x, "q": q, "level": level},
                "expected": raised(st.quantile_ci, x, q, level),
            }

    # the quantiles numpy refuses, of probabilities outside [0, 1] and not a number
    x = np.array([1.0, 2.0, 3.0])
    for label, qs in {
        "above": [0.5, 1.5],
        "below": [-0.1],
        "not_a_number": [math.nan],
    }.items():
        cases[f"quantiles/{label}"] = {
            "inputs": {"x": x, "qs": qs},
            "expected": raised(np.nanquantile, x, qs),
        }

    # the first-order index from given data
    def first_order(label: str, x: Any, y: Any, n_bins: int | None = None) -> None:
        with np.errstate(all="ignore"):
            expected = raised(st.first_order_given_data, x, y, n_bins)
        cases[f"first_order/{label}"] = {
            "inputs": {"x": x, "y": y, "n_bins": n_bins},
            "expected": expected,
        }

    for n in (7, 8, 9, 20, 100):
        x = rng.standard_normal(n)
        first_order(f"random_{n}", x, 2.0 * x + rng.standard_normal(n))
    # an input without an effect, ties of the input which the stable sort keeps in order
    x = np.round(rng.standard_normal(50), 1)
    first_order("no_effect", x, rng.standard_normal(50))
    first_order("ties", x, x * x + 0.1 * rng.standard_normal(50))
    first_order("ties_bins_3", x, x * x + 0.1 * rng.standard_normal(50), 3)
    first_order("negative_zero_ties", np.array([0.0, -0.0] * 6), np.arange(12.0))
    x = rng.standard_normal(30)
    y = x + rng.standard_normal(30)
    for bins in (1, 2, 7, 29, 30, 31, 0):
        first_order(f"bins_{bins}", x, y, bins)
    first_order("bins_0_too_few", x[:7], y[:7], 0)
    first_order("no_variance", x, np.full(30, 4.0))
    first_order("no_variance_bins_0", x, np.full(30, 4.0), 0)
    with_gaps_x, with_gaps_y = x.copy(), y.copy()
    with_gaps_x[[1, 4]] = [np.nan, np.inf]
    with_gaps_y[[2, 9, 11]] = [np.nan, -np.inf, np.inf]
    first_order("not_finite_pairs", with_gaps_x, with_gaps_y)
    first_order("variance_overflows", x, y * 1e300)
    first_order("lengths", x, y[:29])
    first_order("lengths_one_x", x[:1], y)
    first_order("lengths_one_y", x, y[:1])
    first_order("lengths_one_and_empty", x[:1], y[:0])
    first_order("lengths_empty_and_one", x[:0], y[:1])
    first_order("empty", x[:0], y[:0])
    for n in (1000, 100003):
        x, y = paired_draws(n)
        cases[f"first_order_formula/{n}"] = {
            "inputs": {"n": n, "first_x": x[:4], "first_y": y[:4]},
            "expected": st.first_order_given_data(x, y),
        }

    # the precision of the draws of an output
    def precise(label: str, values: np.ndarray, rel_tol: float) -> None:
        with np.errstate(all="ignore"):
            expected = raised(_precise, values, rel_tol)
        cases[f"precise/{label}"] = {
            "inputs": {"values": values, "rel_tol": rel_tol},
            "expected": expected,
        }

    precise("seven", rng.standard_normal(7), 0.1)
    precise("seven_and_not_a_number", np.append(rng.standard_normal(7), np.nan), 0.1)
    precise("constant", np.full(10, 3.0), 0.1)
    precise("random_8", rng.standard_normal(8), 10.0)
    precise("random_50", rng.standard_normal(50), 0.5)
    precise("random_50_strict", rng.standard_normal(50), 0.1)
    precise("infinite", np.append(rng.standard_normal(20), np.inf), 0.1)
    precise("overflow", np.array([0.0, 1e100, -1e100] * 3), 0.1)
    # enough draws for both ends of the intervals of the 5 % and the 95 % quantile
    values = rng.standard_normal(200)
    values[[3, 50, 120]] = np.nan
    precise("random_200_not_a_number", values, 0.3)
    for n in (1000, 100003):
        values = large_residuals(n)
        for rel_tol in (0.05, 0.5):
            cases[f"precise_formula/{n}_{rel_tol}"] = {
                "inputs": {"n": n, "rel_tol": rel_tol},
                "expected": _precise(values, rel_tol),
            }

    # the parameters of the distribution of a reported interval
    def interval(low: float, high: float, level: float, lognormal: bool) -> Any:
        u = Uncertain.interval(
            low, high, level=level, shape="log-normal" if lognormal else "normal"
        )
        d = u.distribution
        return [d.median, d.cv] if lognormal else [d.mean, d.sd]

    intervals = {
        "positive": (1.0, 2.0, 0.95),
        "positive_90": (0.3, 7.5, 0.9),
        "positive_50": (10.0, 11.0, 0.5),
        "wide": (1e-6, 1e6, 0.99),
        "narrow": (1.0, 1.0 + 1e-12, 0.95),
        "negative": (-1.0, 2.0, 0.9),
        "infinite": (1.0, math.inf, 0.95),
        "minus_infinite": (-math.inf, 1.0, 0.95),
        "subnormal": (5e-324, 1e-323, 0.95),
        "empty": (2.0, 1.0, 0.9),
        "point": (1.0, 1.0, 0.9),
        "not_a_number": (math.nan, 2.0, 0.9),
        "level0": (1.0, 2.0, 0.0),
        "level1": (1.0, 2.0, 1.0),
        "level_not_a_number": (1.0, 2.0, math.nan),
        # 0.5 + level / 2 is 0.5: a quantile of zero, a division by it
        "level_tiny": (1.0, 2.0, 1e-17),
        # the variance of the logarithm past the exponential of the largest double
        "exp_overflow": (1e-300, 1e300, 1e-10),
        "zero": (0.0, 1.0, 0.95),
        # neighbouring doubles of one logarithm: a log-normal without a spread
        "logarithms_equal": (1e300, math.nextafter(1e300, math.inf), 0.95),
    }
    for label, (low, high, level) in intervals.items():
        for lognormal in (False, True):
            name = "lognormal" if lognormal else "normal"
            cases[f"interval/{label}_{name}"] = {
                "inputs": {
                    "low": low,
                    "high": high,
                    "level": level,
                    "lognormal": lognormal,
                },
                "expected": raised(interval, low, high, level, lognormal),
            }
    return cases


# --- draws -----------------------------------------------------------------------------------


def large_correlation(k: int) -> np.ndarray:
    """Get the symmetric `k x k` matrix of a unit diagonal of a formula of integers.

    `r_ij = (((i + j) * 7 + i * j) % 19 - 9) / 10` off the diagonal, a coefficient of
    -0.9 to 0.9 which is not positive semi-definite; a division by ten is rounded the same
    by every language, so the tests compute it bit for bit as here.
    """
    i, j = np.indices((k, k))
    r = (((i + j) * 7 + i * j) % 19 - 9) / 10.0
    np.fill_diagonal(r, 1.0)
    return r


def _validated(
    targets: list[str],
    matrix: Any,
    *,
    kind: str = "covariance",
    units: list[str | None] | None = None,
) -> dict[str, Any]:
    """The case of the validation of a `Covariance`: null if it is valid, else its error."""
    from sbmlsim.uncertainty.covariance import Covariance

    array = np.asarray(matrix, dtype=float)
    try:
        Covariance(targets, array, units=units, kind=kind)
        expected: Any = None
    except ValueError as error:
        expected = {"error": str(error)}
    return {
        "inputs": {
            "targets": targets,
            "shape": list(array.shape),
            "matrix": array.ravel(),
            "kind": kind,
            "units": None if units is None else len(units),
        },
        "expected": expected,
    }


def _rho(
    m: float, a: tuple[str, float, float], b: tuple[str, float, float], scale: str
) -> Any:
    """The coefficient of the normal scores of a covariance `m` of two marginals.

    The arithmetic of the double loop of `normal_score_correlation`: the marginals as
    `_moments` gives them (the family, the mean and the standard deviation, of the
    logarithm for a log-normal), `m` an element of a matrix of numpy.
    """
    family = a[0]
    m = np.float64(m)
    try:
        if family == "normal" or scale == "log":
            return m / (a[2] * b[2])
        return math.log1p(m / (a[1] * b[1])) / (a[2] * b[2])
    except (ValueError, ZeroDivisionError) as error:
        return {"error": str(error), "exception": type(error).__name__}


def draws() -> dict[str, Any]:
    """The cases of `draws` and `correlation`: `sbmlsim.uncertainty.draws` and `.covariance`.

    The nearest correlation matrix (`nearest_correlation`, Higham 2002) of the classical
    example of Higham, of matrices which are a correlation already, without iterations, of
    a not a number and of 50 and 200 targets (of the latter every 97th value and the sums
    of the rows); `_is_psd` at its tolerance; the validation of a
    `Covariance` with every message; the coefficients of the normal scores of
    `normal_score_correlation`; and the Gaussian copula of `draw` for given points and a
    positive definite correlation (a singular one has the factor of the eigendecomposition,
    which is LAPACK's and matches only in distribution).
    """
    from sbmlsim.simulation.sampling.distributions import EPS
    from sbmlsim.uncertainty.covariance import PSD_RTOL, _is_psd, nearest_correlation
    from sbmlsim.uncertainty.draws import _factor as draws_factor

    rng = np.random.default_rng(20261128)
    cases: dict[str, Any] = {"psd_rtol": {"inputs": {}, "expected": PSD_RTOL}}

    nearest: dict[str, tuple[np.ndarray, int, float]] = {
        # Higham (2002), section 4: a tridiagonal matrix of ones
        "higham": (
            np.array(
                [[1, 1, 0, 0], [1, 1, 1, 0], [0, 1, 1, 1], [0, 0, 1, 1]], dtype=float
            ),
            100,
            1e-10,
        ),
        # the coefficients of three sources which do not fit together
        "inconsistent_3": (
            np.array([[1.0, 0.9, -0.9], [0.9, 1.0, 0.9], [-0.9, 0.9, 1.0]]),
            100,
            1e-10,
        ),
        "correlation": (
            np.array([[1.0, 0.5, 0.2], [0.5, 1.0, 0.1], [0.2, 0.1, 1.0]]),
            100,
            1e-10,
        ),
        "one": (np.array([[1.0]]), 100, 1e-10),
        "empty": (np.zeros((0, 0)), 100, 1e-10),
        "no_iterations": (
            np.array([[1.0, 0.9, -0.9], [0.9, 1.0, 0.9], [-0.9, 0.9, 1.0]]),
            0,
            1e-10,
        ),
        "few_iterations": (large_correlation(8), 3, 1e-10),
        "loose": (large_correlation(8), 100, 1e-3),
        "nan": (np.array([[1.0, np.nan], [np.nan, 1.0]]), 100, 1e-10),
        "inf": (np.array([[1.0, np.inf], [np.inf, 1.0]]), 100, 1e-10),
        "large_50": (large_correlation(50), 100, 1e-10),
    }
    for name, (m, iterations, tol) in nearest.items():
        k = m.shape[0]
        # the large matrix by its formula, so the fixture holds only the result
        inputs: dict[str, Any] = {"k": k, "max_iterations": iterations, "tol": tol}
        if not name.startswith("large"):
            inputs["m"] = m.ravel()
        cases[f"nearest/{name}"] = {
            "inputs": inputs,
            "expected": nearest_correlation(m, iterations, tol).ravel(),
        }
    # 200 targets of the formula: every 97th value of the result in the order of the rows and
    # the sums of the rows, so the fixture stays small (the 40000 values are about 1 MB)
    y = nearest_correlation(large_correlation(200))
    cases["nearest_large/200"] = {
        "inputs": {"k": 200, "max_iterations": 100, "tol": 1e-10, "stride": 97},
        "expected": {"strided": y.ravel()[::97], "row_sums": y.sum(axis=1)},
    }

    psd = {
        "identity": np.eye(3),
        "singular": np.array([[1.0, 1.0], [1.0, 1.0]]),
        # the smallest eigenvalue -1e-11 of the largest 2: within the tolerance
        "within": np.diag([2.0, 1.0, -1e-11]),
        "outside": np.diag([2.0, 1.0, -1e-9]),
        "indefinite": np.array([[1.0, 0.9, -0.9], [0.9, 1.0, 0.9], [-0.9, 0.9, 1.0]]),
        "negative": np.array([[-1.0]]),
        "zero": np.zeros((2, 2)),
        "nan": np.array([[1.0, np.nan], [np.nan, 1.0]]),
        "inf": np.array([[1.0, np.inf], [np.inf, 1.0]]),
        # eigvalsh reads the lower triangle
        "lower": np.array([[1.0, 5.0], [0.5, 1.0]]),
    }
    for name, m in psd.items():
        cases[f"is_psd/{name}"] = {
            "inputs": {"k": m.shape[0], "m": m.ravel(), "rtol": PSD_RTOL},
            "expected": _is_psd(m),
        }

    ab = ["a", "b"]
    abc = ["a", "b", "c"]
    validate = {
        "correlation": _validated(
            abc, [[1, 0.5, 0.2], [0.5, 1, 0.1], [0.2, 0.1, 1]], kind="correlation"
        ),
        "covariance": _validated(ab, [[4.0, 1.0], [1.0, 2.0]], units=["mM", None]),
        "singular": _validated(ab, [[1.0, 1.0], [1.0, 1.0]], kind="correlation"),
        "empty": _validated([], np.zeros((0, 0)), kind="correlation"),
        "empty_vector": _validated([], np.zeros(0), kind="correlation"),
        "twice": _validated(["b", "a", "b", "a", "c"], np.eye(5), kind="correlation"),
        # the targets in the list as python writes them, by their repr
        "quotes": _validated(
            ["it's", 'say "x"', "a\\b", "x\ny", "x'\"", "tab\t", "é"],
            np.full((7, 7), np.nan),
            kind="correlation",
        ),
        "quotes_twice": _validated(["it's", "it's"], np.eye(2), kind="correlation"),
        "not_square": _validated(["a"], np.eye(2), kind="correlation"),
        "vector": _validated(ab, [1.0, 0.0], kind="correlation"),
        "scalar": _validated(ab, 1.0, kind="correlation"),
        "three_dimensions": _validated(ab, np.ones((2, 2, 2)), kind="correlation"),
        "nan": _validated(ab, [[1, np.nan], [np.nan, 1]], kind="correlation"),
        "inf": _validated(ab, [[np.inf, 0], [0, 1]]),
        "not_symmetric": _validated(ab, [[1, 0.9], [0.95, 1]], kind="correlation"),
        "symmetric_within": _validated(ab, [[1e6, 1.0], [1.0 + 1e-7, 1e6]]),
        "diagonal": _validated(ab, [[1, 0.5], [0.5, 1 + 1e-9]], kind="correlation"),
        "diagonal_within": _validated(
            ab, [[1, 0.5], [0.5, 1 + 1e-13]], kind="correlation"
        ),
        "rho_above_one": _validated(
            abc, [[1, 1.1, 0], [1.1, 1, 0], [0, 0, 1]], kind="correlation"
        ),
        "not_psd_correlation": _validated(
            abc, [[1, 0.9, -0.9], [0.9, 1, 0.9], [-0.9, 0.9, 1]], kind="correlation"
        ),
        "not_psd_covariance": _validated(
            abc, [[4, 1.9, -1.9], [1.9, 1, 0.9], [-1.9, 0.9, 1]]
        ),
        "not_psd_negative_largest": _validated(
            abc, [[1, -0.95, 0.9], [-0.95, 1, 0.0], [0.9, 0.0, 1]], kind="correlation"
        ),
        "zero_variance": _validated(ab, [[0, 0], [0, 1]]),
        "negative_variance": _validated(ab, [[1, 0], [0, -1]]),
        "units": _validated(ab, [[1, 0], [0, 1]], units=["mM"]),
        "small_eigenvalue": _validated(ab, [[1e-30, 0.0], [0.0, 1e-30]]),
        "tiny_not_psd": _validated(ab, [[1e-300, 2e-300], [2e-300, 1e-300]]),
    }
    for name, case in validate.items():
        cases[f"validate/{name}"] = case

    rho = {
        "normal": (0.3, ("normal", 1.0, 0.5), ("normal", 2.0, 2.0), "linear"),
        "normal_zero_sd": (0.3, ("normal", 1.0, 0.0), ("normal", 2.0, 2.0), "linear"),
        "log": (0.02, ("log-normal", 1.0, 0.2), ("log-normal", 3.0, 0.4), "log"),
        "lognormal": (
            0.12,
            ("log-normal", 2.0, 0.2),
            ("log-normal", 3.0, 0.4),
            "linear",
        ),
        "lognormal_negative": (
            -0.5,
            ("log-normal", 2.0, 0.2),
            ("log-normal", 3.0, 0.4),
            "linear",
        ),
        "lognormal_domain": (
            -6.0,
            ("log-normal", 2.0, 0.2),
            ("log-normal", 3.0, 0.4),
            "linear",
        ),
        "lognormal_below": (
            -7.5,
            ("log-normal", 2.0, 0.2),
            ("log-normal", 3.0, 0.4),
            "linear",
        ),
        "lognormal_zero_sd": (
            0.1,
            ("log-normal", 2.0, 0.0),
            ("log-normal", 3.0, 0.4),
            "linear",
        ),
    }
    for name, (m, a, b, scale) in rho.items():
        cases[f"rho/{name}"] = {
            "inputs": {"m": m, "a": list(a), "b": list(b), "scale": scale},
            "expected": _rho(m, a, b, scale),
        }

    def copula(u: np.ndarray, r: np.ndarray) -> np.ndarray:
        """The copula of `draw` of the points `u` and the correlation `r`."""
        if not np.allclose(r, np.eye(r.shape[0])):
            z = stats.norm.ppf(np.clip(u, EPS, 1.0 - EPS)) @ draws_factor(r).T
            u = stats.norm.cdf(z)
        return u

    r3 = np.array([[1.0, 0.7, 0.2], [0.7, 1.0, 0.1], [0.2, 0.1, 1.0]])
    u3 = rng.random((20, 3))
    # the ends of the unit interval, clipped to EPS
    u3[0] = [0.0, 0.5, 1.0]
    u3[1] = [1e-15, 1.0 - 1e-15, 0.5]
    with_nan = u3[:4].copy()
    with_nan[2, 1] = np.nan
    x = rng.standard_normal((8, 12))
    r8 = np.corrcoef(x)
    copulas = {
        "correlated_3": (u3, r3),
        "identity": (u3, np.eye(3)),
        # within the tolerances of np.allclose of the identity, 1e-8 + 1e-5 |I|: unchanged
        "near_identity": (
            u3,
            np.array([[1.0 + 5e-6, 5e-9, 0.0], [5e-9, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        ),
        "outside_identity": (
            u3,
            np.array([[1.0, 2e-8, 0.0], [2e-8, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        ),
        "negative": (u3[:, :2], np.array([[1.0, -0.8], [-0.8, 1.0]])),
        "nan": (with_nan, r3),
        "eight": (rng.random((16, 8)), (r8 + r8.T) / 2.0),
    }
    for name, (u, r) in copulas.items():
        n, d = u.shape
        cases[f"copula/{name}"] = {
            "inputs": {"n": n, "d": d, "u": u.ravel(), "r": r.ravel()},
            "expected": copula(u, r).ravel(),
        }
    return cases


# --- testing ---------------------------------------------------------------------------------


def pattern(n: int, period: int, ones: int, step: int, shift: int) -> list[bool]:
    """Get `n` draws of a formula of integers, whether every draw satisfies a hypothesis.

    `((i * step + shift) % period) < ones` of the draw `i`: with `step` coprime to `period`
    every period has `ones` draws which satisfy it, spread over the period; `step = 1` and
    `period > n` gives `ones` draws which satisfy it first and none after them. The tests
    compute the draws from the formula, so the fixture holds the formula, not the draws.
    """
    return [((i * step + shift) % period) < ones for i in range(n)]


#: the draws of the cases of `testing`, by name: a list of booleans or the arguments of
#: `pattern` (n, period, ones, step, shift)
SEQUENCES: dict[str, Any] = {
    "empty": [],
    "one_true": [True],
    "one_false": [False],
    "short": [True, False, True, True, False, True],
    "true_5": (5, 1, 1, 1, 0),
    "true_40": (40, 1, 1, 1, 0),
    "true_500": (500, 1, 1, 1, 0),
    "false_5": (5, 1, 0, 1, 0),
    "false_40": (40, 1, 0, 1, 0),
    "false_500": (500, 1, 0, 1, 0),
    "p70_2": (2, 10, 7, 3, 1),
    "p70_7": (7, 10, 7, 3, 1),
    "p70_50": (50, 10, 7, 3, 1),
    "p50_500": (500, 100, 50, 37, 11),
    "p52_500": (500, 100, 52, 37, 3),
    "p90_500": (500, 100, 90, 37, 5),
    "p97_300": (300, 100, 97, 37, 2),
    "p10_200": (200, 100, 10, 37, 7),
    # every draw of the first 30 satisfies the hypothesis, none after: the confidence sets
    # of the first and of the last draws exclude each other
    "contradicting": (530, 10**6, 30, 1, 0),
}

#: the thresholds, margins and alphas of the cases of every sequence
THRESHOLDS: list[tuple[float, float, float]] = [
    (0.5, 0.05, 0.05),
    (0.9, 0.05, 0.05),
    (0.0, 0.05, 0.05),
    (1.0, 0.05, 0.05),
    (0.03, 0.05, 0.01),
    (0.97, 0.2, 0.05 / 3),
]

#: the thresholds of some sequences only: outside of [0, 1], not a number, no margin
EXTRA_THRESHOLDS: dict[str, list[tuple[float, float, float]]] = {
    "short": [(-0.2, 0.05, 0.05), (1.5, 0.05, 0.05), (math.nan, 0.05, 0.05)],
    "p50_500": [(0.5, 0.0, 0.05), (0.45, 0.0, 0.01), (0.6, 0.2, 0.05)],
    "p52_500": [(0.5, 0.0, 0.05), (0.55, 0.05, 0.05)],
    "contradicting": [(0.06, 0.05, 0.05)],
}


def _draws_of(spec: Any) -> list[bool]:
    """The draws of a sequence of `SEQUENCES`."""
    return list(spec) if isinstance(spec, list) else pattern(*spec)


def _strided(values: np.ndarray, stride: int) -> list[float]:
    """Every `stride`-th value from the first and the last value."""
    indices = list(range(0, values.size, stride))
    if values.size and indices[-1] != values.size - 1:
        indices.append(values.size - 1)
    return values[indices].tolist()


def _bands(distribution: np.ndarray, qs: list[float]) -> np.ndarray:
    """The bands of a distribution of groups, the lines of `predictive_check`.

    A column per data point: the quantiles `qs` of its finite values (`np.quantile`,
    linear), `nan` of a column without one. A copy of the lines of the python, which is
    not a function of its own.
    """
    bands = np.full((distribution.shape[1], len(qs)), np.nan)
    for j in range(distribution.shape[1]):
        column = distribution[:, j]
        column = column[np.isfinite(column)]
        if column.size:
            bands[j] = np.quantile(column, qs)
    return bands


def testing() -> dict[str, Any]:
    """The cases of `testing` and `variability`: `sbmlsim.uncertainty.testing` and the bands.

    The p-values of `P >= p0`, `P < p0` and of the equivalence (`_p_values`, every value of
    the first 60 draws and every 17th after, the last included), the decision and the draws
    which first showed it (`_decide`), the confidence sequence (`_sequence`, the ends of the
    intersection of the sets, of the last draws whose sets intersect for sets which exclude
    each other) and the draws an undecided statement needs (`_needed`) of draws of all,
    none and some of the draws satisfying the hypothesis, of 0 to 500 draws, at degenerate
    thresholds (0, 1, outside, a margin past an end) as well; the errors of the sequence and
    of `_needed`; `family_level`; and the bands of `predictive_check` (numpy's quantiles of
    the finite values of every column).
    """
    from sbmlsim.uncertainty import testing as tests

    assert tests.MIXTURE == (0.5, 0.5)
    cases: dict[str, Any] = {
        "constants": {
            "inputs": {},
            "expected": {
                "margin": tests.MARGIN,
                "max_needed": tests.MAX_NEEDED,
                "bisections": tests.BISECTIONS,
            },
        }
    }
    for name, spec in SEQUENCES.items():
        x = np.asarray(_draws_of(spec), dtype=bool)
        n, k = x.size, int(np.count_nonzero(x))
        stride = 1 if n <= 60 else 17
        drawn = (
            {"satisfied": spec}
            if isinstance(spec, list)
            else dict(zip(("n", "period", "ones", "step", "shift"), spec, strict=True))
        )
        for i, (p0, margin, alpha) in enumerate(
            THRESHOLDS + EXTRA_THRESHOLDS.get(name, [])
        ):
            prefixes = tests._Prefixes.of(x)
            p_greater, p_less, p_equivalent = tests._p_values(prefixes, p0, margin)
            decision, n_decided = tests._decide(
                p_greater, p_less, p_equivalent, prefixes.m, alpha
            )
            low, high = tests._sequence(prefixes, alpha)
            cases[f"sequence/{name}/{i}"] = {
                "inputs": {
                    **drawn,
                    "p0": p0,
                    "margin": margin,
                    "alpha": alpha,
                    "stride": stride,
                },
                "expected": {
                    "p_greater": _strided(p_greater, stride),
                    "p_less": _strided(p_less, stride),
                    "p_equivalent": _strided(p_equivalent, stride),
                    "decision": decision,
                    "n_decided": n_decided,
                    "low": low,
                    "high": high,
                    "needed": (
                        tests._needed(k / n, n, p0, margin, alpha) if n else None
                    ),
                },
            }

    # the errors of the sequence of three draws: the logarithm of 1 / alpha
    for i, alpha in enumerate([0.0, -0.5, math.inf, -math.inf, 1.0, 2.0, math.nan]):
        prefixes = tests._Prefixes.of([True, False, True])
        cases[f"sequence_alpha/{i}"] = {
            "inputs": {"satisfied": [True, False, True], "alpha": alpha},
            "expected": raised(lambda p=prefixes, a=alpha: list(tests._sequence(p, a))),
        }

    inputs = {"estimate": [], "n": [], "p0": [], "margin": [], "alpha": []}
    for estimate in [0.0, 0.3, 0.5, 0.52, 0.9, 0.97, 1.0, 1 / 3]:
        for n in [1, 3, 100, 2**25 + 1, 2**26]:
            for p0, margin in [(0.5, 0.05), (0.9, 0.05), (0.5, 0.0), (0.95, 0.2)]:
                for key, value in zip(
                    inputs, (estimate, n, p0, margin, 0.05), strict=True
                ):
                    inputs[key].append(value)
    cases["needed"] = {
        "inputs": inputs,
        "expected": [
            tests._needed(*arguments)
            for arguments in zip(*inputs.values(), strict=True)
        ],
    }
    for i, (estimate, n) in enumerate(
        [(math.nan, 3), (math.inf, 3), (-math.inf, 3), (math.nan, 2**26)]
    ):
        cases[f"needed_error/{i}"] = {
            "inputs": {
                "estimate": estimate,
                "n": n,
                "p0": 0.5,
                "margin": 0.05,
                "alpha": 0.05,
            },
            "expected": raised(tests._needed, estimate, n, 0.5, 0.05, 0.05),
        }

    levels, ms = [0.95, 0.9, 0.99], [-1, 0, 1, 2, 3, 7, 1000]
    inputs = {"level": [], "m": []}
    for level in levels:
        for m in ms:
            inputs["level"].append(level)
            inputs["m"].append(m)
    cases["family_level"] = {
        "inputs": inputs,
        "expected": [
            tests.family_level(level, m)
            for level, m in zip(inputs["level"], inputs["m"], strict=True)
        ],
    }

    rng = np.random.default_rng(20261128)
    gappy = rng.lognormal(size=(40, 6))
    gappy[3, 0] = math.nan
    gappy[7, 0] = math.inf
    gappy[11, 1] = -math.inf
    gappy[:, 2] = math.nan
    gappy[:39, 3] = math.nan
    gappy[5:9, 4] = math.nan
    distributions = {
        "finite": (rng.normal(size=(25, 4)), [0.05, 0.5, 0.95]),
        "gappy": (gappy, [0.05, 0.5, 0.95]),
        "ends": (rng.normal(size=(9, 3)), [0.0, 0.025, 0.975, 1.0]),
        "one_row": (rng.normal(size=(1, 3)), [0.05, 0.95]),
        "ties": (np.repeat([[1.0, -2.0], [3.0, -2.0]], 5, axis=0), [0.1, 0.5, 0.9]),
        "empty_rows": (np.zeros((0, 3)), [0.05, 0.95]),
        "no_columns": (np.zeros((4, 0)), [0.05, 0.95]),
        "outside": (rng.normal(size=(5, 2)), [0.05, 1.5]),
        "nan_q": (rng.normal(size=(5, 2)), [0.05, math.nan]),
        # an invalid quantile of no finite value: numpy is never called
        "outside_unused": (np.full((3, 2), math.nan), [-0.5, 0.5]),
    }
    for name, (distribution, qs) in distributions.items():
        rows, cols = distribution.shape
        cases[f"bands/{name}"] = {
            "inputs": {
                "rows": rows,
                "cols": cols,
                "distribution": distribution.ravel(),
                "qs": qs,
            },
            "expected": raised(lambda d=distribution, q=qs: _bands(d, q).ravel()),
        }
    return cases


GENERATORS: dict[str, Callable[[], dict[str, Any]]] = {
    "special": special,
    "linalg": linalg,
    "metrics": metrics,
    "fisher": fisher,
    "uncertainty": uncertainty,
    "draws": draws,
    "testing": testing,
}

#: the modules whose fixture is of the functions of sbmlsim, whose version it records
OF_SBMLSIM = {"metrics", "fisher", "uncertainty", "draws", "testing"}


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
