"""Type stubs of the extension module `sbmlsim_core._core`."""

from collections.abc import Callable, Iterable, Mapping, Sequence
from os import PathLike
from pathlib import Path
from typing import Any, SupportsFloat

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
"""The points with a ratio `stats_subset_agreement` needs, 10 (since 0.40.0)."""

def stats_sse(residuals: numpy.typing.ArrayLike) -> float:
    """Sum of Squared Errors (SSE) of the residuals, 0 of none (since 0.40.0).

    `sbmlsim.fit.metrics.sse`: the sum of numpy, pairwise, to the last bits.
    """

def stats_mse(residuals: numpy.typing.ArrayLike) -> float:
    """Mean Squared Error (MSE) of the residuals (since 0.40.0).

    Raises `ValueError` without a residual.
    """

def stats_rmse_from_mse(mse: SupportsFloat) -> float:
    """Root Mean Squared Error (RMSE) from the mean squared error (since 0.40.0).

    Raises `ValueError` for a negative MSE.
    """

def stats_aic_from_mse(mse: SupportsFloat, n: int, k: int) -> float:
    """Akaike Information Criterion of a least squares fit, `n ln(MSE) + 2 k` (since 0.40.0).

    `-inf` for an MSE of 0. Raises `ValueError` for a negative MSE and a number of points
    which is not positive.
    """

def stats_bic_from_mse(mse: SupportsFloat, n: int, k: int) -> float:
    """Bayesian Information Criterion of a least squares fit, `n ln(MSE) + k ln(n)` (since 0.40.0).

    `-inf` for an MSE of 0. Raises `ValueError` for a negative MSE and a number of points
    which is not positive.
    """

def stats_r_squared(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> float:
    """Coefficient of determination (R²) of a prediction, `1 - SSE / SST` (since 0.40.0).

    `nan` for fewer than two data points and for data without variance. Raises
    `ValueError` for another number of predictions than of data points.
    """

def stats_fold_counts(
    y_observed: numpy.typing.ArrayLike,
    y_predicted: numpy.typing.ArrayLike,
    fold: SupportsFloat = 2.0,
) -> tuple[int, int]:
    """The points within the fold factor and the points with a ratio (since 0.40.0).

    A point has a ratio with a positive and finite value and prediction, and is within the
    fold factor for `y / fold <= f <= y * fold`. Raises `ValueError` for a fold factor which
    is not larger than 1 and another number of predictions than of data points.
    """

def stats_fold_accepted(
    y_observed: numpy.typing.ArrayLike,
    y_predicted: numpy.typing.ArrayLike,
    fold: SupportsFloat = 2.0,
) -> numpy.typing.NDArray[numpy.float64]:
    """The fold criterion of every point, `Fold(fold).accepted` of sbmlsim (since 0.40.0).

    1 within the fold factor, 0 outside, `nan` for a point without a ratio. Raises
    `ValueError` as `stats_fold_counts`.
    """

def stats_sd_accepted(
    y_observed: numpy.typing.ArrayLike,
    y_predicted: numpy.typing.ArrayLike,
    y_sd: numpy.typing.ArrayLike,
    k: float = 2.0,
) -> numpy.typing.NDArray[numpy.float64]:
    """The uncertainty criterion of every point, `Uncertainty(k).accepted` (since 0.40.0).

    1 for `|y - f| <= k sd`, 0 outside, `nan` for a point without a positive and finite
    standard deviation or with a value or prediction which is not finite. Raises
    `ValueError` for other numbers of predictions or standard deviations than of values.
    """

def stats_gmfe(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> float:
    """The geometric mean fold error `10**mean(|log10(f/y)|)` (since 0.40.0).

    Of the points with a ratio, `nan` without one. Raises `ValueError` for another number
    of predictions than of values.
    """

def stats_afe(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> float:
    """The average fold error `10**mean(log10(f/y))`, the bias as a fold (since 0.40.0).

    Of the points with a ratio, `nan` without one. Raises `ValueError` for another number
    of predictions than of values.
    """

def stats_nrmse(nres: numpy.typing.ArrayLike) -> float:
    """The root mean square of the normalized residuals (since 0.40.0).

    `nan` without one or with one which is not finite, `evaluation._rms` of sbmlsim.
    """

def stats_guest_limits(
    observed: numpy.typing.ArrayLike,
) -> tuple[numpy.typing.NDArray[numpy.float64], numpy.typing.NDArray[numpy.float64]]:
    """The limits of the predicted ratios of Guest et al., `delta = 1` (since 0.40.0).

    The lower and the upper limit of every observed ratio, of the shape of the ratios.
    """

def stats_within_guest(
    observed: numpy.typing.ArrayLike, predicted: numpy.typing.ArrayLike
) -> float:
    """The percentage of the finite predicted ratios within the limits of Guest et al. (since 0.40.0).

    `nan` without a finite predicted ratio. Raises `ValueError` for ratios of other shapes
    and an observed ratio which is not positive and finite.
    """

def stats_subset_agreement(
    y_observed: numpy.typing.ArrayLike, y_predicted: numpy.typing.ArrayLike
) -> tuple[float, float, float, float] | None:
    """The agreement of a subset of the points with a ratio, Bland and Altman (since 0.40.0).

    The bias `mean(log10(f/y))`, the half width `1.96 SD` of the limits of agreement and
    the half widths of the 95% intervals of the bias and of a limit (t quantile of `n - 1`
    degrees of freedom), in decades; `None` for fewer than
    `STATS_MIN_POINTS_SUBSET_AGREEMENT` points with a ratio. Raises `ValueError` for
    another number of predictions than of values.
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
