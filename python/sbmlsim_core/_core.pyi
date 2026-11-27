"""Type stubs of the extension module `sbmlsim_core._core`."""

from collections.abc import Callable, Iterable, Mapping, Sequence
from os import PathLike
from pathlib import Path
from typing import Any

import numpy
import numpy.typing

COLOR_SCALES: list[str]
"""The names of the colour scales of the figures, every map followed by its reversal `<name>_r`.

Since 0.30.0: the sequential and the diverging colour maps of matplotlib in lower case.
"""
FIGURE_CONTRACT: str
FIT_CONTRACT: str
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
) -> list[str]:
    """The colours (`#rrggbb`) of `values` on a colour scale.

    `name` is a name of `COLOR_SCALES`, or (since 0.31.0) the colour map as stops
    `[(0.0, "#rrggbb"), (0.3, "#rrggbb"), ..., (1.0, "#rrggbb")]`, matplotlib's
    `LinearSegmentedColormap.from_list`: at least two, the first at 0 and the last at 1, the
    positions not decreasing; its 256 colours interpolated linearly in sRGB between them.

    The values `low..high` take the part `start..end` of the colour map (since 0.28.0), e.g.
    `end=0.85` for the lines of a scan which leave out the lightest colours.

    Raises `ValueError` for an unknown name, for stops which are no colour map and for a part
    which is not `0 <= start < end <= 1`.
    """
