"""Type stubs of the extension module `sbmlsim_core._core`."""

from collections.abc import Callable, Iterable, Mapping, Sequence
from os import PathLike
from pathlib import Path
from typing import Any, Literal, SupportsFloat, overload

import numpy
import numpy.typing

COLOR_SCALES: list[str]
"""The names of the colour scales of the figures, every map followed by its reversal `<name>_r`.

Since 0.30.0: the sequential and the diverging colour maps of matplotlib in lower case.
"""
FIGURE_CONTRACT: str
FIT_CONTRACT: str
FIT_FIELDS: list[str]
"""The additive fields of `fit/1` this build reads which an older release would not refuse.

Since 0.35.0: `curve.dimension`, a curve over the points of a dimension of a scan which
interpolates between their conditions, and `condition.pinned`, a parameter whose target a
condition holds at a value of its own. A release before it has no such constant, so a host
reads it before it sends such a field.
"""
FIT_REDUCTIONS: list[str]
"""The kinds of the `reduction` of a curve of `fit/1` this build fits.

Since 0.32.0: `max`, `min`, `mean`, `at` and `pk`. A release before it neither has this
constant nor denies the field, so a host reads the constant before it sends a reduction.
"""
MODEL_CONTRACT: str
SIMULATION_CONTRACT: str
THIRD_PARTY: str
__version__: str
__allocator__: str

class SbmlsimCoreError(Exception):
    """The base class of every error of sbmlsim_core."""

class CompileError(SbmlsimCoreError):
    """An SBML model which cannot be compiled."""

class LoadError(SbmlsimCoreError):
    """A model or a runtime which cannot be loaded, or an instance which cannot be made."""

class CompiledModel:
    """A model compiled to `model.wasm` and its description (`model/2`)."""

    @property
    def wasm(self) -> bytes:
        """The module `model.wasm`."""

    @property
    def description(self) -> dict[str, Any]:
        """The description of the model, `model.json` of `model/2`, as a dict."""

class SimulationError(SbmlsimCoreError):
    """The base class of the errors of a simulation."""

class RequestError(SimulationError):
    """A request which the engine refuses."""

class IntegrationError(SimulationError):
    """An integration which failed."""

    flag: int
    time: float

class UnsupportedError(SimulationError):
    """A simulation the engine does not support."""

class InternalError(SimulationError):
    """A failure of the engine itself."""

class PointError:
    """A point of a batch which failed: the error of its simulation as a value."""

    @property
    def kind(self) -> str:
        """The kind: `request`, `integration`, `unsupported` or `internal`."""

    @property
    def message(self) -> str:
        """The message of the error."""

    @property
    def flag(self) -> int | None:
        """The return flag of CVODE of an integration which failed, else none."""

    @property
    def time(self) -> float | None:
        """The time an integration which failed reached, else none."""

class SimulationOutput:
    """The table of a simulation."""

    @property
    def columns(self) -> list[str]:
        """The names of the columns, `time` first."""

    @property
    def values(self) -> numpy.typing.NDArray[numpy.float64]:
        """The values, of the shape `(n_rows, len(columns))`, in C order."""

    @property
    def sensitivities(self) -> OutputSensitivities | None:
        """The forward sensitivities, `None` for a request which did not ask for them."""

    @property
    def local_errors(self) -> OutputLocalErrors | None:
        """The local error estimates, `None` for a request which did not ask for them."""

class OutputSensitivities:
    """The forward sensitivities of the columns of a table."""

    @property
    def parameters(self) -> list[str]:
        """The ids of the parameters, the `sensitivities` of the request, in their order."""

    @property
    def values(self) -> numpy.typing.NDArray[numpy.float64]:
        """The derivatives, of the shape `(n_rows, len(columns), len(parameters))`, in C order."""

class OutputLocalErrors:
    """The local error estimates of the integrator at the rows of a table."""

    @property
    def states(self) -> list[str]:
        """The ids of the states, in the order of the state vector of the model."""

    @property
    def errors(self) -> numpy.typing.NDArray[numpy.float64]:
        """The estimates of the last step before every row, of the shape `(n_rows, len(states))`."""

    @property
    def weights(self) -> numpy.typing.NDArray[numpy.float64]:
        """The weights of the error test of that step, of the same shape."""

class PreparedSimulation:
    """A validated request bound to one loaded model, reusable across its instances."""

    @property
    def n_values(self) -> int:
        """Number of ordered numeric inputs required for each point."""

class Instance:
    """A model in an engine of its own; simulates one request after the other."""

    def simulate(self, request: dict[str, Any]) -> SimulationOutput:
        """Simulates a request of `simulation/1` and gives its table."""

    def simulate_prepared(
        self, prepared: PreparedSimulation, values: Sequence[float] = ...
    ) -> SimulationOutput:
        """Runs a prepared request with the ordered input values."""

    def stats(self) -> dict[str, int | float]:
        """The statistics of the integrator of the last simulation.

        The counts as integers, the times and steps of its state as floats.
        """

class LoadedModel:
    """A model compiled for an engine, which makes the instances that simulate it."""

    @property
    def description(self) -> dict[str, Any]:
        """The description of the model, `model/2`, as a dict."""

    def initial_values(self) -> dict[str, float]:
        """The values of the selections of the model as loaded."""

    def instance(self) -> Instance:
        """A new instance of the model."""

    def prepare(
        self, request: dict[str, Any], *, varying: Sequence[str] = ...
    ) -> PreparedSimulation:
        """Validates the request and ordered varying input IDs once."""

    def fit(
        self,
        request: dict[str, Any],
        *,
        threads: int | None = None,
        on_run: Callable[[int, dict[str, Any]], None] | None = None,
    ) -> dict[str, Any]:
        """Fits the parameters of the model to data with a request of the contract `fit/1`."""

    def fit_problem(
        self,
        request: dict[str, Any],
        *,
        threads: int | None = None,
    ) -> FitProblem:
        """The problem of a fit of `fit/1`, for an optimizer which stays in python."""

    def batch_executor(self, *, threads: int | None = None) -> BatchExecutor:
        """Creates a reusable pool with lazy workers and retained instances."""

    def simulate_many(
        self,
        requests: Sequence[dict[str, Any]],
        *,
        threads: int | None = None,
    ) -> list[SimulationOutput | PointError]:
        """Simulates every request on `threads` threads, in the order of the requests."""

class FitProblem:
    """The objective of a fit of the engine: the residuals and the jacobian of a point."""

    @property
    def parameters(self) -> list[str]:
        """The ids of the parameters an optimizer searches, in the order of the point."""

    @property
    def n_residuals(self) -> int:
        """The number of residuals of one evaluation."""

    @property
    def threads(self) -> int:
        """The threads of the batch of an evaluation."""

    @property
    def start(self) -> list[float]:
        """The start of the fit, in the space an optimizer searches."""

    @property
    def bounds(self) -> tuple[list[float], list[float]]:
        """The bounds of the parameters in that space."""

    @property
    def jacobian_of(self) -> str:
        """Where the jacobian comes from: `sensitivities` or `difference_quotient`."""

    @property
    def simulations(self) -> int:
        """The simulations the problem has run since it was built."""

    def values(self, x: Sequence[float]) -> list[float]:
        """The values of the parameters at a point, on the linear scale."""

    def step(self, at: int, x: Sequence[float]) -> float:
        """The relative step the jacobian of that parameter takes at a point."""

    def residuals(self, x: Sequence[float]) -> numpy.typing.NDArray[numpy.float64]:
        """The residuals at a point, one batch of the engine."""

    def cost(self, x: Sequence[float]) -> float:
        """Half the sum of the squares of those residuals."""

    def jacobian(self, x: Sequence[float]) -> numpy.typing.NDArray[numpy.float64]:
        """The jacobian at a point, `(n_residuals, parameters)`, one batch of the engine."""

class PetabProblem:
    """A PEtab v2 problem read, compiled and evaluated in Rust (since 0.38.0).

    The class of `sbmlsim_core.petab.Problem`.
    """

    @staticmethod
    def from_yaml(
        yaml: str | PathLike[str],
        *,
        relative_tolerance: float | None = None,
        absolute_tolerance: float | None = None,
        data_at_change: str | None = None,
        extension: str = "read",
    ) -> PetabProblem:
        """Reads a problem from its YAML file, with the files it names next to it."""

    @staticmethod
    def from_files(
        files: Mapping[str, bytes],
        yaml: str,
        *,
        relative_tolerance: float | None = None,
        absolute_tolerance: float | None = None,
        data_at_change: str | None = None,
        extension: str = "read",
    ) -> PetabProblem:
        """Reads a problem from the contents of its files, by their names."""

    @property
    def parameters(self) -> list[str]:
        """The ids of the estimated parameters, in the order of a point."""

    @property
    def nominal(self) -> list[float]:
        """The nominal values of the estimated parameters."""

    @property
    def bounds(self) -> tuple[list[float], list[float]]:
        """The bounds of the estimated parameters."""

    @property
    def n_measurements(self) -> int:
        """The number of measurements."""

    @property
    def n_simulations(self) -> int:
        """The number of simulations of an evaluation."""

    @property
    def cost_offset(self) -> float:
        """What the cost of `fit_problem` carries on top of the objective of PEtab."""

    def requests(self, x: Sequence[float] | None = None) -> list[dict[str, Any]]:
        """The requests of `simulation/1` of every simulation at a point."""

    def evaluate(self, x: Sequence[float] | None = None) -> dict[str, Any]:
        """The log-likelihood, chi2, the log priors and the simulations at a point."""

    def llh(self, x: Sequence[float] | None = None) -> float:
        """The log-likelihood of PEtab at a point, the nominal values without."""

    def fit_problem(self, *, threads: int | None = None) -> FitProblem:
        """The problem of the fit of the engine, for an optimizer of python."""

    def fit(
        self, settings: Mapping[str, Any] | None = None, *, threads: int | None = None
    ) -> dict[str, Any]:
        """Fits the estimated parameters from their nominal values in the engine.

        A tolerance of the optimizer which `settings` does not give (`gradient_tolerance`,
        `step_tolerance`, `cost_tolerance`) is the relative tolerance of the integrator,
        at least 1e-8.
        """

class BatchExecutor:
    """A reusable batch pool. Concurrent calls raise RuntimeError."""

    def simulate_many(
        self, requests: Iterable[dict[str, Any]]
    ) -> list[SimulationOutput | PointError]:
        """Simulates requests in order, retaining worker instances."""

    def simulate_prepared(
        self, prepared: PreparedSimulation, points: Iterable[Sequence[float]]
    ) -> list[SimulationOutput | PointError]:
        """Runs numeric rows in order, retaining workers and prepared plans."""

    def simulate_plans(
        self, plans: Iterable[tuple[PreparedSimulation, Sequence[float]]]
    ) -> list[SimulationOutput | PointError]:
        """Runs prepared requests with their own values in order, retaining workers."""

    def close(self) -> None:
        """Releases resources; further submissions raise RuntimeError."""

    def __enter__(self) -> BatchExecutor: ...
    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> None: ...

class Runtime:
    """Runtime and model modules use a persistent Wasmtime compilation cache."""

    def __init__(
        self,
        max_memory_bytes: int | None = None,
        *,
        timeout_ms: int = 30_000,
        cache: bool = True,
        cache_dir: str | PathLike[str] | None = None,
    ) -> None:
        """Bounds instance memory and execution time; timeout_ms=0 disables deadlines."""

    @property
    def cache_dir(self) -> Path | None:
        """The active cache directory, or None if disabled or unavailable."""

    def load(self, wasm: bytes, description: dict[str, Any]) -> LoadedModel:
        """Loads `model.wasm` with its description."""

def compile_sbml(
    sbml: str,
    *,
    optimize: bool = True,
    jacobian: str = "auto",
    parameter_jacobian: bool = True,
) -> CompiledModel:
    """Compiles the text of an SBML Level 3 document into `model.wasm` and its description."""

def runtime_wasm() -> dict[str, bytes]:
    """The bytes of `engine.wasm`, `mathlib.wasm` and `compiler.wasm` this extension embeds."""

def plot_wasm() -> bytes:
    """The bytes of `plot.wasm` this extension embeds, the renderer of the figures in a page.

    A page renders a `figure/1` spec with it to the SVG `render_svg` gives, byte for byte.
    """

def fit_models(
    models: Mapping[str, LoadedModel],
    request: dict[str, Any],
    *,
    threads: int | None = None,
    on_run: Callable[[int, dict[str, Any]], None] | None = None,
) -> dict[str, Any]:
    """Fits several models to data, each condition of the request on the model it names."""

def fit_problem_of_models(
    models: Mapping[str, LoadedModel],
    request: dict[str, Any],
    *,
    threads: int | None = None,
) -> FitProblem:
    """The problem of a fit of several models, for an optimizer which stays in python."""

def chaos_expansion(
    d: int, degree: int, *, q: float = 1.0, max_interaction: int | None = None
) -> numpy.typing.NDArray[numpy.int64]:
    """The multi-indices of a truncated basis of a polynomial chaos expansion."""

def chaos_design_matrix(
    u: numpy.typing.ArrayLike,
    multi_indices: numpy.typing.ArrayLike,
    *,
    threads: int | None = None,
) -> numpy.typing.NDArray[numpy.float64]:
    """The values of the terms of a basis at points of the unit cube."""

def chaos_predict(
    u: numpy.typing.ArrayLike,
    multi_indices: numpy.typing.ArrayLike,
    coefficients: numpy.typing.ArrayLike,
    *,
    threads: int | None = None,
) -> numpy.typing.NDArray[numpy.float64]:
    """The values of a polynomial chaos expansion at points of the unit cube."""

def render_svg(spec: str) -> str:
    """The SVG of a figure given as the JSON text of a `figure/1` spec.

    Raises `ValueError` with the path and message of an invalid spec.
    """

def render_png(spec: str, dpi: float = 144.0) -> bytes:
    """The PNG of a figure at `dpi`, of its drawn size (`figure_geometry`) times `dpi / 72`.

    Raises `ValueError` for an invalid spec and `RuntimeError` for a figure which cannot be
    rasterized, e.g. an image of more than 64 Mpx.
    """

def figure_geometry(spec: str) -> dict[str, Any]:
    """The drawn size, panels, scales and legend entries of a figure in its pixels.

    `width` and `height` are the size the figure is drawn at, the size of the spec grown by
    the legends outside the plots.
    """

def pick(spec: str, x: float, y: float, radius: float = 8.0) -> dict[str, Any] | None:
    """The data point nearest to the pixel `(x, y)` within `radius` pixels, or `None`."""

def text_width(
    text: str, size: float, bold: bool = False, family: str = "sans"
) -> float:
    """The width in pixels of `text` at the font size `size` px in Inter, the figure layout measure.

    Since 0.28.0. The width of a text of several lines is the width of its widest line.
    `family` (since 0.31.1) is `sans`, Inter, or `monospace`, the family of a text element
    whose every character is 0.6 of the size wide; another raises `ValueError`.
    """

def color_scale(
    name: str | Sequence[tuple[float, str]],
    low: float,
    high: float,
    values: Sequence[float],
    center: float | None = None,
    log: bool = False,
    start: float = 0.0,
    end: float = 1.0,
    gamma: float | None = None,
) -> list[str]:
    """The colours (`#rrggbb`) of `values` on a colour scale.

    `name` is a name of `COLOR_SCALES`, or (since 0.31.0) the colour map as stops
    `[(0.0, "#rrggbb"), (0.3, "#rrggbb"), ..., (1.0, "#rrggbb")]`, matplotlib's
    `LinearSegmentedColormap.from_list`: at least two, the first at 0 and the last at 1, the
    positions not decreasing; its 256 colours interpolated linearly in sRGB between them.

    The values `low..high` take the part `start..end` of the colour map (since 0.28.0), e.g.
    `end=0.85` for the lines of a scan which leave out the lightest colours.

    `gamma` (since 0.33.0) makes the scale a power scale, matplotlib's `PowerNorm`: a value at
    the share `t` of the scale takes the colour at `t ** gamma`.

    Raises `ValueError` for an unknown name, for stops which are no colour map, for a part
    which is not `0 <= start < end <= 1` and for a `gamma` which is not positive.
    """

def image_png(pixels: numpy.typing.ArrayLike) -> str:
    """The `png` of an image element of a figure (since 0.34.0): the base64 of a PNG.

    `pixels` is an RGB or RGBA image, an array of shape `(rows, columns, 3)` or
    `(rows, columns, 4)` whose first row is the top of the image, of integers from 0 to 255 or
    of numbers from 0 to 1, as matplotlib's `imshow` reads an image.

    Raises `ValueError` for another shape, for values out of their range and for an image of
    more than 64 Mpx.
    """

def thin_kept_points(
    x: numpy.typing.ArrayLike,
    y: numpy.typing.ArrayLike,
    tolerance: float,
    keep: numpy.typing.ArrayLike | None = None,
) -> numpy.typing.NDArray[numpy.bool_]:
    """The points of a line which are kept, Ramer-Douglas-Peucker (since 0.35.0).

    The first and the last point, the ones of `keep`, every point which is not finite and
    its neighbours, and the points which keep every other one within `tolerance` of the
    segment between the points kept around it, on every row of `y` (a two-dimensional
    array, a row per column of the line over `x`). The core of `kept_points` of
    `sbmlsim_core.chart.thinning`.

    Raises `ValueError` for rows of another length than `x`, for no row, and for a `keep`
    of another length.
    """

def sensitivity_sobol_sequence(
    d: int, n: int, *, seed: int | None, skip: int = 0
) -> numpy.typing.NDArray[numpy.float64]:
    """The points `skip..skip + n` of the Sobol' sequence of `d` dimensions (since 0.37.0).

    The sequence of `scipy.stats.qmc.Sobol(d, bits=30)`, a row per point: bit for bit scipy's
    without scrambling (`seed=None`), scrambled as scipy scrambles it (LMS+shift) with the
    draws of `seed`. At most 2048 dimensions and `2^30` points.

    Raises `ValueError` for arguments out of range.
    """

def sensitivity_saltelli(
    d: int, n: int, *, seed: int | None, second_order: bool = False, skip: int = 0
) -> numpy.typing.NDArray[numpy.float64]:
    """The matrix of Saltelli of `d` parameters on the unit cube (since 0.37.0).

    The design of `SALib.sample.sobol.sample` on the Sobol' sequence of `2 d` dimensions of
    `sensitivity_sobol_sequence`: `n (d + 2)` points, `n (2 d + 2)` with the second order.

    Raises `ValueError` for arguments out of range, e.g. more than 1024 parameters.
    """

def sensitivity_fast_sample(
    d: int,
    n: int,
    *,
    m: int = 4,
    seed: int | None = None,
    phases: Sequence[float] | None = None,
) -> numpy.typing.NDArray[numpy.float64]:
    """The design of the extended FAST of `d` parameters on the unit cube (since 0.37.0).

    The search curves of `SALib.sample.fast_sampler.sample`, `n` points per parameter, with
    the phases of the stream of `seed` or the given phases, one per parameter.

    Raises `ValueError` for `n` not more than `4 m^2`, and unless one of `seed` and `phases`
    is given.
    """

def sensitivity_morris_sample(
    d: int, trajectories: int, *, levels: int = 4, seed: int
) -> numpy.typing.NDArray[numpy.float64]:
    """The design of Morris of `d` parameters on the grid of `levels` levels (since 0.37.0).

    The trajectories of `SALib.sample.morris.sample`, `d + 1` points each, on `[0, 1]`.

    Raises `ValueError` for arguments out of range.
    """

def sensitivity_sobol_indices(
    y: numpy.typing.ArrayLike,
    d: int,
    *,
    second_order: bool = False,
    conf_level: float = 0.95,
    num_resamples: int | None = None,
    seed: int | None = None,
    resamples: numpy.typing.ArrayLike | None = None,
    threads: int | None = None,
) -> dict[str, numpy.typing.NDArray[numpy.float64]]:
    """The Sobol indices of the values of a matrix of Saltelli, every column at once (since 0.37.0).

    `y` has a row per point and a column per element. `S1`, `S1_conf`, `ST` and `ST_conf`
    of the shape `(d, elements)`, with the second order `S2` and `S2_conf` of the shape
    `(d, d, elements)`; the intervals over `num_resamples` (100) resamples of the stream of
    `seed`, or the given `resamples` of the shape `(resamples, n)`.

    The elements are spread over `threads` threads (every CPU of the process by default),
    and an index does not depend on them.

    Raises `ValueError` for values which are no matrix of the design, and unless one of
    `seed` and `resamples` is given.
    """

def sensitivity_fast_indices(
    y: numpy.typing.ArrayLike,
    d: int,
    *,
    m: int = 4,
    conf_level: float = 0.95,
    num_resamples: int | None = None,
    seed: int | None = None,
    resamples: numpy.typing.ArrayLike | None = None,
    threads: int | None = None,
) -> dict[str, numpy.typing.NDArray[numpy.float64]]:
    """The indices of the extended FAST of the values of a FAST design (since 0.37.0).

    `S1`, `S1_conf`, `ST` and `ST_conf` of the shape `(d, elements)`; the intervals over
    windows from starts of the stream of `seed`, or the given `resamples` of the shape
    `(d, resamples)`.

    Raises `ValueError` for values which are no blocks of the design, and unless one of
    `seed` and `resamples` is given.
    """

def sensitivity_morris_effects(
    cube: numpy.typing.ArrayLike,
    y: numpy.typing.ArrayLike,
    *,
    levels: int = 4,
    conf_level: float = 0.95,
    num_resamples: int | None = None,
    seed: int | None = None,
    resamples: numpy.typing.ArrayLike | None = None,
    threads: int | None = None,
) -> dict[str, numpy.typing.NDArray[numpy.float64]]:
    """The elementary effects of the values of a Morris design (since 0.37.0).

    `cube` holds the points of the design, `y` their values. `mu`, `mu_star`, `sigma` and
    `mu_star_conf` of the shape `(d, elements)`; the interval over resamples of the
    trajectories of the stream of `seed`, or the given `resamples` of the shape
    `(d, resamples, trajectories)`.

    Raises `ValueError` for points which are no trajectories, and unless one of `seed` and
    `resamples` is given.
    """

STATS_MIN_POINTS_SUBSET_AGREEMENT: int
"""The points with a ratio `stats_subset_agreement` needs, 10 (since 0.42.0)."""

def stats_sse(residuals: numpy.typing.ArrayLike) -> float:
    """Sum of Squared Errors (SSE) of the residuals, 0 of none (since 0.42.0).

    `sbmlsim.fit.metrics.sse`: the sum of numpy, pairwise, to the last bits.
    """

def stats_mse(residuals: numpy.typing.ArrayLike) -> float:
    """Mean Squared Error (MSE) of the residuals (since 0.42.0).

    Raises `ValueError` without a residual.
    """

def stats_rmse_from_mse(mse: SupportsFloat) -> float:
    """Root Mean Squared Error (RMSE) from the mean squared error (since 0.42.0).

    Raises `ValueError` for a negative MSE.
    """

def stats_aic_from_mse(mse: SupportsFloat, n: int, k: int) -> float:
    """Akaike Information Criterion of a least squares fit, `n ln(MSE) + 2 k` (since 0.42.0).

    `-inf` for an MSE of 0. Raises `ValueError` for a negative MSE and a number of points
    which is not positive.
    """

def stats_bic_from_mse(mse: SupportsFloat, n: int, k: int) -> float:
    """Bayesian Information Criterion of a least squares fit, `n ln(MSE) + k ln(n)` (since 0.42.0).

    `-inf` for an MSE of 0. Raises `ValueError` for a negative MSE and a number of points
    which is not positive.
    """

def stats_r_squared(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> float:
    """Coefficient of determination (R²) of a prediction, `1 - SSE / SST` (since 0.42.0).

    `nan` for fewer than two data points and for data without variance. Raises
    `ValueError` for another number of predictions than of data points.
    """

def stats_fold_counts(
    y_observed: numpy.typing.ArrayLike,
    y_predicted: numpy.typing.ArrayLike,
    fold: SupportsFloat = 2.0,
) -> tuple[int, int]:
    """The points within the fold factor and the points with a ratio (since 0.42.0).

    A point has a ratio with a positive and finite value and prediction, and is within the
    fold factor for `y / fold <= f <= y * fold`. Raises `ValueError` for a fold factor which
    is not larger than 1 and another number of predictions than of data points.
    """

def stats_fold_accepted(
    y_observed: numpy.typing.ArrayLike,
    y_predicted: numpy.typing.ArrayLike,
    fold: SupportsFloat = 2.0,
) -> numpy.typing.NDArray[numpy.float64]:
    """The fold criterion of every point, `Fold(fold).accepted` of sbmlsim (since 0.42.0).

    1 within the fold factor, 0 outside, `nan` for a point without a ratio. Raises
    `ValueError` as `stats_fold_counts`.
    """

def stats_sd_accepted(
    y_observed: numpy.typing.ArrayLike,
    y_predicted: numpy.typing.ArrayLike,
    y_sd: numpy.typing.ArrayLike,
    k: float = 2.0,
) -> numpy.typing.NDArray[numpy.float64]:
    """The uncertainty criterion of every point, `Uncertainty(k).accepted` (since 0.42.0).

    1 for `|y - f| <= k sd`, 0 outside, `nan` for a point without a positive and finite
    standard deviation or with a value or prediction which is not finite. Raises
    `ValueError` for other numbers of predictions or standard deviations than of values.
    """

def stats_gmfe(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> float:
    """The geometric mean fold error `10**mean(|log10(f/y)|)` (since 0.42.0).

    Of the points with a ratio, `nan` without one. Raises `ValueError` for another number
    of predictions than of values.
    """

def stats_afe(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> float:
    """The average fold error `10**mean(log10(f/y))`, the bias as a fold (since 0.42.0).

    Of the points with a ratio, `nan` without one. Raises `ValueError` for another number
    of predictions than of values.
    """

def stats_nrmse(nres: numpy.typing.ArrayLike) -> float:
    """The root mean square of the normalized residuals (since 0.42.0).

    `nan` without one or with one which is not finite, `evaluation._rms` of sbmlsim.
    """

def stats_guest_limits(
    observed: numpy.typing.ArrayLike,
) -> tuple[numpy.typing.NDArray[numpy.float64], numpy.typing.NDArray[numpy.float64]]:
    """The limits of the predicted ratios of Guest et al., `delta = 1` (since 0.42.0).

    The lower and the upper limit of every observed ratio, of the shape of the ratios.
    """

def stats_within_guest(
    observed: numpy.typing.ArrayLike, predicted: numpy.typing.ArrayLike
) -> float:
    """The percentage of the finite predicted ratios within the limits of Guest et al. (since 0.42.0).

    `nan` without a finite predicted ratio. Raises `ValueError` for ratios of other shapes
    and an observed ratio which is not positive and finite.
    """

def stats_subset_agreement(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> tuple[float, float, float, float] | None:
    """The agreement of a subset of the points with a ratio, Bland and Altman (since 0.42.0).

    The bias `mean(log10(f/y))`, the half width `1.96 SD` of the limits of agreement and
    the half widths of the 95% intervals of the bias and of a limit (t quantile of `n - 1`
    degrees of freedom), in decades; `None` for fewer than
    `STATS_MIN_POINTS_SUBSET_AGREEMENT` points with a ratio. Raises `ValueError` for
    another number of predictions than of values.
    """

STATS_DEFAULT_RANK_TOLERANCE: float
"""The fraction of the largest eigenvalue below which an eigenvalue is a direction the data
do not constrain, 1e-8 (since 0.42.0)."""

def stats_gram(jacobian: numpy.typing.ArrayLike) -> numpy.typing.NDArray[numpy.float64]:
    """The Fisher information `J' J` of a jacobian of the residuals by the parameters (since 0.42.0).

    Raises `ValueError` for a jacobian which is not a matrix and numpy's (`array is too
    big`) for an information past what an array holds, of a jacobian of no residuals of
    more parameters than a 64 bit integer squares.
    """

def stats_fisher_statistics(
    information: numpy.typing.ArrayLike,
    cost: float,
    n: int,
    likelihood: bool = False,
    rank_tolerance: float = 1e-8,
) -> dict[str, Any]:
    """The statistics of a Fisher information, `sbmlsim.fit.fisher.FisherInformation` (since 0.42.0).

    `information` is `J' J` of `n` data points with the cost `cost` of the data, of a
    likelihood with `likelihood`. The keys: `sigma2`, the variance of the residuals
    `2 cost / (n - k)` (`nan` for `n <= k`, 1 for a likelihood); `spectrum`, the eigenvalues
    largest first; `rank`, the number of eigenvalues above `rank_tolerance` times the
    largest one; `condition`, the ratio of the largest to the smallest eigenvalue (`inf` of
    a smallest one which is not positive); `covariance`, `sigma2` times the pseudo-inverse
    of the information, `(k, k)`; `standard_errors`, `nan` of a variance which is not
    positive (zero allowed of an identifiable information); `correlation`,
    `cov_ij / (sd_i sd_j)`, `(k, k)`. Raises `ValueError` for an information which is not
    a square, finite and symmetric matrix of at least one parameter.
    """

def stats_fisher_intervals(
    values: numpy.typing.ArrayLike,
    standard_errors: numpy.typing.ArrayLike,
    scales: Sequence[Any],
    level: float = 0.95,
    dof: float | None = None,
) -> tuple[numpy.typing.NDArray[numpy.float64], numpy.typing.NDArray[numpy.float64]]:
    """The lower and upper bounds of the intervals of the parameters (since 0.42.0).

    `value ± q SE` on the scale of every parameter, transformed back into the units of the
    model, with the quantile `q` of the t distribution of `dof` degrees of freedom (`nan`
    intervals for `dof <= 0`), of the normal distribution without (a likelihood, the
    credible intervals of the Laplace approximation). Raises `ValueError` for another
    number of standard errors or scales than of values and for an unknown scale. A scale
    is a name of `sbmlsim.fit.options.ParameterScaleType` (`"LINEAR"`, `"LOG"`, `"LOG10"`),
    of PEtab (`"lin"`, `"log"`, `"log10"`) or a member of an enum of these names.
    """

def stats_fisher_relative_errors(
    values: numpy.typing.ArrayLike,
    standard_errors: numpy.typing.ArrayLike,
    scales: Sequence[Any],
) -> numpy.typing.NDArray[numpy.float64]:
    """The standard errors relative to the values, `FisherInformation.relative_errors` (since 0.42.0).

    `|SE / value|` on the linear scale, `SE` on the natural and `ln(10) SE` on the decadic
    logarithm. Raises `ValueError` as `stats_fisher_intervals`.
    """

def stats_fisher_decomposition(
    information: numpy.typing.ArrayLike,
) -> tuple[numpy.typing.NDArray[numpy.float64], numpy.typing.NDArray[numpy.float64]]:
    """The decomposition the statistics of a Fisher information come from (since 0.44.0).

    The eigenvalues, largest first, and the pseudo-inverse (numpy's, the eigenvalues cut at
    `1e-15` of the largest) of one eigendecomposition, `FisherInformation.spectrum` and
    `.inverse`: `stats_fisher_statistics(m, ...)` is
    `stats_fisher_statistics_of_decomposition(*stats_fisher_decomposition(m), ...)` to the
    bit. Raises `ValueError` as `stats_fisher_statistics`.
    """

def stats_fisher_statistics_of_decomposition(
    spectrum: numpy.typing.ArrayLike,
    inverse: numpy.typing.ArrayLike,
    cost: float,
    n: int,
    likelihood: bool = False,
    rank_tolerance: float = 1e-8,
) -> dict[str, Any]:
    """The statistics of a Fisher information from its decomposition (since 0.44.0).

    The dictionary of `stats_fisher_statistics` from the eigenvalues `spectrum` (largest
    first) and the pseudo-inverse `inverse` instead of the matrix, as `FisherInformation`
    derives them from a record which stores them, so a decomposition of LAPACK gives the
    numbers it was written with, to the bit. The pseudo-inverse is taken as it is, without a
    check of its symmetry or of its values. Raises `ValueError` for eigenvalues which are not
    a vector of one value per row of the square pseudo-inverse and eigenvalues which are not
    largest first.
    """

@overload
def stats_norm_cdf(x: float) -> float: ...
@overload
def stats_norm_cdf(
    x: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_norm_cdf(
    x: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The distribution function of the standard normal distribution, `scipy.stats.norm.cdf` (since 0.44.0).

    Value by value of an array of any shape: a float of a number (or an array of no
    dimension), an array of numpy of the shape of another array; `nan`, the infinities and
    the arguments outside of the domain as scipy gives them.
    """

@overload
def stats_norm_ppf(q: float) -> float: ...
@overload
def stats_norm_ppf(
    q: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_norm_ppf(
    q: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The quantile of the standard normal distribution, `scipy.stats.norm.ppf` (since 0.44.0).

    Value by value of an array of any shape: a float of a number (or an array of no
    dimension), an array of numpy of the shape of another array; `nan`, the infinities and
    the arguments outside of the domain as scipy gives them.
    """

@overload
def stats_erf(x: float) -> float: ...
@overload
def stats_erf(
    x: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_erf(
    x: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The error function, `scipy.special.erf` (since 0.44.0).

    Value by value of an array of any shape: a float of a number (or an array of no
    dimension), an array of numpy of the shape of another array; `nan`, the infinities and
    the arguments outside of the domain as scipy gives them.
    """

@overload
def stats_gamma(x: float) -> float: ...
@overload
def stats_gamma(
    x: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_gamma(
    x: numpy.typing.ArrayLike,
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The gamma function, `scipy.special.gamma` (since 0.44.0).

    Value by value of an array of any shape: a float of a number (or an array of no
    dimension), an array of numpy of the shape of another array; `nan`, the infinities and
    the arguments outside of the domain as scipy gives them.
    """

@overload
def stats_t_cdf(x: float, df: float) -> float: ...
@overload
def stats_t_cdf(
    x: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_t_cdf(
    x: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The distribution function of the Student t distribution of `df` degrees of freedom, `scipy.stats.t.cdf` (since 0.44.0).

    `x` and `df` of any shapes broadcast against each other as scipy broadcasts them: a
    float of two numbers, an array of numpy of the broadcast shape else; `nan`, the
    infinities and the arguments outside of the domain as scipy gives them. Raises
    `ValueError` with the message of scipy for shapes which do not broadcast.
    """

@overload
def stats_t_ppf(q: float, df: float) -> float: ...
@overload
def stats_t_ppf(
    q: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_t_ppf(
    q: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The quantile of the Student t distribution of `df` degrees of freedom, `scipy.stats.t.ppf` (since 0.44.0).

    `q` and `df` of any shapes broadcast against each other as scipy broadcasts them: a
    float of two numbers, an array of numpy of the broadcast shape else; `nan`, the
    infinities and the arguments outside of the domain as scipy gives them. Raises
    `ValueError` with the message of scipy for shapes which do not broadcast.
    """

@overload
def stats_chi2_cdf(x: float, df: float) -> float: ...
@overload
def stats_chi2_cdf(
    x: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_chi2_cdf(
    x: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The distribution function of the chi-squared distribution of `df` degrees of freedom, `scipy.stats.chi2.cdf` (since 0.44.0).

    `x` and `df` of any shapes broadcast against each other as scipy broadcasts them: a
    float of two numbers, an array of numpy of the broadcast shape else; `nan`, the
    infinities and the arguments outside of the domain as scipy gives them. Raises
    `ValueError` with the message of scipy for shapes which do not broadcast.
    """

@overload
def stats_chi2_ppf(q: float, df: float) -> float: ...
@overload
def stats_chi2_ppf(
    q: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]: ...
def stats_chi2_ppf(
    q: numpy.typing.ArrayLike, df: numpy.typing.ArrayLike
) -> float | numpy.typing.NDArray[numpy.float64]:
    """The quantile of the chi-squared distribution of `df` degrees of freedom, `scipy.stats.chi2.ppf` (since 0.44.0).

    `q` and `df` of any shapes broadcast against each other as scipy broadcasts them: a
    float of two numbers, an array of numpy of the broadcast shape else; `nan`, the
    infinities and the arguments outside of the domain as scipy gives them. Raises
    `ValueError` with the message of scipy for shapes which do not broadcast.
    """

def stats_posterior_laplace(
    precision: numpy.typing.ArrayLike,
    pids: Sequence[str] | None = None,
    opid: str | None = None,
    rank_tolerance: float = 1e-8,
) -> numpy.typing.NDArray[numpy.float64]:
    """The covariance of the Laplace approximation of a posterior of its precision (since 0.42.0).

    `sbmlsim.fit.posterior.posterior_laplace` from the precision on. Raises `ValueError` with
    its message for a direction the precision does not constrain, the parameters named by
    `pids` (by their indices without) and the message prefixed by `opid`, and for a
    precision which is not a square finite matrix of at least one parameter.
    """

def stats_rank_correlation(
    r: numpy.typing.ArrayLike,
) -> numpy.typing.NDArray[numpy.float64]:
    """The Spearman rank correlation `6 / π asin(r / 2)` of a correlation (since 0.42.0).

    `ParameterPosterior.rank_correlation`: `r` clipped to `[-1, 1]`, the diagonal 1,
    symmetrized. Raises `ValueError` for a matrix which is not square.
    """

def stats_normal_correlation(
    r: numpy.typing.ArrayLike,
) -> numpy.typing.NDArray[numpy.float64]:
    """The correlation of the normal scores of a posterior (since 0.42.0).

    `ParameterPosterior.normal_correlation`: the symmetric part of `r`, the diagonal 1.
    Raises `ValueError` for a matrix which is not square.
    """

def stats_log_sd(scale: Any, sd: float) -> float:
    """The standard deviation of the natural logarithm of an SD on a scale (since 0.42.0).

    `ln(10) sd` on `LOG10`, `sd` otherwise. Raises `ValueError` for an unknown scale.
    """

def stats_lognormal_cv(sigma: float) -> float:
    """The coefficient of variation `sqrt(exp(sigma²) - 1)` of a log-normal (since 0.42.0)."""

def stats_mean_se(x: numpy.typing.ArrayLike) -> tuple[float, float, int]:
    """The mean, its standard error `s / sqrt(n)` and the number of draws (since 0.42.0).

    `sbmlsim.uncertainty.statistics.mean_se`: the draws which are `NaN` dropped, `nan`
    without draws, the error `nan` without two of them and of infinite draws.
    """

def stats_sd_se(x: numpy.typing.ArrayLike) -> tuple[float, float, int]:
    """The standard deviation, its standard error and the number of draws (since 0.42.0).

    `sbmlsim.uncertainty.statistics.sd_se`, the error by the delta method with the fourth
    central moment: `nan` without two draws and of infinite draws. Raises `OverflowError`
    for a fourth power of the standard deviation past the largest double, as python does.
    """

def stats_quantile(x: numpy.typing.ArrayLike, q: float) -> float:
    """The quantile `q` of the draws which are not `NaN`, `np.nanquantile` (since 0.42.0).

    numpy's linear method, `nan` without draws. Raises `ValueError` for a probability
    outside [0, 1].
    """

def stats_quantiles(
    x: numpy.typing.ArrayLike, qs: numpy.typing.ArrayLike
) -> numpy.typing.NDArray[numpy.float64]:
    """The quantiles `qs` of the draws which are not `NaN`, `np.nanquantile` (since 0.42.0).

    A one-dimensional array of a quantile per value of `qs` in the order numpy ravels it,
    whatever the shape of `qs` (`np.nanquantile` keeps it). Raises `ValueError` as
    `stats_quantile`.
    """

def stats_quantile_ci(
    x: numpy.typing.ArrayLike, q: float, level: float = 0.95
) -> tuple[float, float, float, int]:
    """A quantile, the ends of its interval and the number of draws (since 0.42.0).

    `sbmlsim.uncertainty.statistics.quantile_ci`: the order statistics of the ranks of the
    binomial distribution, an end beyond the draws `nan`, all `nan` without draws. Raises
    `ValueError` for a probability outside [0, 1] and a level outside [-1, 1].
    """

def stats_first_order_given_data(
    x: numpy.typing.ArrayLike, y: numpy.typing.ArrayLike, n_bins: int | None = None
) -> float:
    """The first-order index of an input from given draws, by binning (since 0.42.0).

    `sbmlsim.uncertainty.statistics.first_order_given_data`: `nan` with fewer than 8
    finite pairs and an output without variance. Raises `ValueError` for draws of other
    lengths and no bins, `IndexError` for one draw against several and `OverflowError` for
    a square of a bin past the largest double, as the python does.
    """

def stats_precise(values: numpy.typing.ArrayLike, rel_tol: float) -> tuple[bool, float]:
    """Whether the draws of an output are precise enough, and the largest width (since 0.42.0).

    `_precise` of `sbmlsim.uncertainty.propagation`: the half widths of the 95 % intervals
    of the mean, the standard deviation and the 5 % and 95 % quantiles relative to the
    standard deviation. Raises `OverflowError` as `stats_sd_se`.
    """

def stats_interval_parameters(
    low: SupportsFloat,
    high: SupportsFloat,
    level: SupportsFloat = 0.95,
    *,
    lognormal: bool,
) -> tuple[float, float]:
    """The parameters of the distribution of a reported interval (since 0.42.0).

    `Uncertain.interval`: the median and the coefficient of variation of a log-normal, the
    mean and the standard deviation of a normal. Raises `ValueError` for a level outside
    (0, 1), an empty interval and a log-normal one which is not positive,
    `ZeroDivisionError` for a level whose quantile is 0 and `OverflowError` for an
    exponential past the largest double, as the python does.
    """

STATS_PSD_RTOL: float
"""The tolerance of the smallest eigenvalue of a positive semi-definite matrix relative to the
largest one, `PSD_RTOL` of `sbmlsim.uncertainty.covariance`, 1e-10 (since 0.42.0)."""

STATS_EPS: float
"""The distance of the points of the copula from the ends of the unit interval, 1e-12
(since 0.42.0)."""

def stats_unit_points(
    d: int,
    n: int,
    *,
    design: Literal["sobol", "lhs", "random"] = "sobol",
    seed: int,
    start: int = 0,
) -> numpy.typing.NDArray[numpy.float64]:
    """`n` points of the unit cube of `d` dimensions, `n` rows of `d` columns (since 0.42.0).

    `unit_points` of `sbmlsim.uncertainty.draws` on the streams of sbmlsim-core: `sobol`
    (the scrambled Sobol' sequence, from the point `start`), `lhs` (a Latin hypercube) or
    `random` (xoshiro256++, from the point `start`, which the stream jumps to in a time
    logarithmic in it). Raises `ValueError` for another design, a Latin hypercube after
    `start`, a negative seed, `d` or `n` with the messages of scipy and numpy, a negative
    `start` of random points and numpy's (`array is too big`) for points past what an array
    holds, `OverflowError` for a negative `start` of the Sobol' sequence, as scipy does.
    """

def stats_copula(
    u: numpy.typing.ArrayLike, r: numpy.typing.ArrayLike
) -> numpy.typing.NDArray[numpy.float64]:
    """The Gaussian copula of the correlation `r` on the points `u` (since 0.42.0).

    The copula of `draw` of `sbmlsim.uncertainty.draws`: `Φ(Φ⁻¹(clip(u)) Fᵀ)` with the factor
    `F` of `r`, the points themselves for a correlation which is the identity. Raises
    `ValueError` for points which are not a matrix or a correlation of another size.
    """

def stats_normal_score_rho(
    m: float,
    a: tuple[str, float, float],
    b: tuple[str, float, float],
    scale: Literal["linear", "log"] = "linear",
) -> float:
    """The coefficient of the normal scores of the covariance `m` of two marginals (since 0.42.0).

    The marginals are `(family, mean, sd)` of `_moments` of `sbmlsim.uncertainty.draws`,
    `normal` or `log-normal`, the covariance on the scale `linear` or `log`. Raises
    `ValueError` of `math.log1p` and `ZeroDivisionError` as the python does.
    """

def stats_nearest_correlation(
    matrix: numpy.typing.ArrayLike, max_iterations: int = 100, tol: float = 1e-10
) -> numpy.typing.NDArray[numpy.float64]:
    """The nearest correlation matrix in the Frobenius norm, Higham 2002 (since 0.42.0).

    `nearest_correlation` of `sbmlsim.uncertainty.covariance`.
    """

def stats_is_psd(matrix: numpy.typing.ArrayLike, rtol: float = 1e-10) -> bool:
    """Whether a symmetric matrix is positive semi-definite, `_is_psd` (since 0.42.0)."""

def stats_validate_covariance(
    targets: Sequence[object],
    matrix: numpy.typing.ArrayLike,
    *,
    kind: Literal["covariance", "correlation"] = "covariance",
    units: Iterable[str | None] | None = None,
) -> None:
    """Check the matrix of a `Covariance` of the targets, `Covariance._validate` (since 0.42.0).

    Raises `ValueError` with the message of the python for a matrix which is not valid.
    """

STATS_MARGIN: float
"""The margin of a threshold by default, `MARGIN` of `sbmlsim.uncertainty.testing`, 0.05
(since 0.42.0)."""

STATS_MAX_NEEDED: int
"""The most draws `stats_needed` searches, `MAX_NEEDED`, 2**26 (since 0.42.0)."""

STATS_BISECTIONS: int
"""The bisections of each end of the confidence sequence, `BISECTIONS`, 52 (since 0.42.0)."""

STATS_MIXTURE: tuple[float, float]
"""The Jeffreys mixture `(a, b)` of the confidence sequence, `MIXTURE`, (0.5, 0.5) (since
0.42.0)."""

def stats_p_values(
    satisfied: numpy.typing.ArrayLike,
    p0: float,
    side: Literal["greater", "less", "equivalent"],
    margin: float = 0.05,
) -> numpy.typing.NDArray[numpy.float64]:
    """The anytime-valid p-values of a probability after every number of draws (since 0.42.0).

    `_Prefixes.p_values` and `_p_values` of `sbmlsim.uncertainty.testing`, from whether every
    draw satisfies the hypothesis (`np.asarray(satisfied, dtype=bool)`): `greater` of
    `P > p0`, `less` of `P < p0` and `equivalent` of `|P - p0| < margin` (the larger p-value
    of the two one-sided tests of the ends of the margin). Raises `ValueError` for another
    side.
    """

def stats_confidence_sequence(
    satisfied: numpy.typing.ArrayLike, level: float = 0.95
) -> tuple[float, float]:
    """The confidence sequence of a probability after the draws (since 0.42.0).

    `confidence_sequence` of `sbmlsim.uncertainty.testing`: the lower and the upper end of the
    intersection of the confidence sets of every number of draws, `(0, 1)` without draws.
    Raises the `ZeroDivisionError` and the `ValueError` of `math.log(1 / (1 - level))` of
    python.
    """

def stats_decide(p_values: numpy.typing.ArrayLike, alpha: float) -> int | None:
    """The number of draws at which a p-value first is at most `alpha` (since 0.42.0)."""

def stats_decision(
    p_greater: numpy.typing.ArrayLike,
    p_less: numpy.typing.ArrayLike,
    p_equivalent: numpy.typing.ArrayLike,
    alpha: float,
) -> tuple[Literal["accepted", "rejected", "equivalent", "undecided"], int | None]:
    """The decision of a statement and the draws which first showed it (since 0.42.0).

    `_decide` of `sbmlsim.uncertainty.testing`: the direction shown first, else the
    equivalence, else `undecided` without a number of draws.
    """

def stats_needed(
    estimate: float, n: int, p0: float, margin: float, alpha: float
) -> int | None:
    """The draws which decide a statement at its estimate (since 0.42.0).

    `_needed` of `sbmlsim.uncertainty.testing`, `None` beyond `STATS_MAX_NEEDED` and of no
    draws. Raises the `ValueError` and the `OverflowError` of `round` of python of an
    estimate which is not finite, and `ValueError` for a negative `n`.
    """

def stats_family_level(level: float, m: int) -> float:
    """The level of a statement in a report of `m` of them, Bonferroni (since 0.42.0)."""

def stats_group_means(
    values: numpy.typing.ArrayLike,
    size: int,
    groups: int,
    *,
    geometric: bool = False,
    seed: int,
) -> numpy.typing.NDArray[numpy.float64]:
    """The statistic of virtual groups of subjects of the predictions (since 0.42.0).

    `_group_means` of `sbmlsim.fit.variability` on the stream xoshiro256++ of sbmlsim-core:
    the subjects which are not finite at every data point are removed, groups of one are
    the subjects themselves, else `groups` groups of `size` subjects drawn with replacement,
    a row per group, their mean or their geometric mean; predictions at no data point give
    the rows without a draw. Raises `ValueError` for predictions which are not a matrix,
    numpy's for a negative size or number of groups and numpy's (`array is too big`) for
    groups whose members or statistics are past what an array holds. That is no check of the
    memory: as with numpy, the memory of a size which can be counted is the caller's.
    """

def stats_bands(
    distribution: numpy.typing.ArrayLike, qs: numpy.typing.ArrayLike
) -> numpy.typing.NDArray[numpy.float64]:
    """The bands of a distribution of groups, a row per data point (since 0.42.0).

    The quantiles of `predictive_check` of `sbmlsim.fit.variability`: numpy's linear
    quantiles `qs` of the finite values of every column, `nan` of a column without one.
    Raises `ValueError` for a distribution which is not a matrix, numpy's for a quantile
    outside `[0, 1]` and numpy's (`array is too big`) for bands past what an array holds.
    """

ODE_CONTRACT: str

class _EquationSystem:
    def __init__(
        self,
        source: bytes,
        documents: list[tuple[str, bytes]] | None = None,
        location: str = "./model.xml",
        max_expanded_elements: int = 10000,
    ) -> None: ...
    @staticmethod
    def from_path(path: str, max_expanded_elements: int = 10000) -> _EquationSystem: ...
    @property
    def model_id(self) -> str | None: ...
    def render(self, fmt: str, options: str) -> tuple[str, str]: ...
    def render_template(self, fmt: str, options: str, template: str) -> str: ...
    def typeset_json(self, links: str) -> str: ...
