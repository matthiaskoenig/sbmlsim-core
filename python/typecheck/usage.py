"""The API of sbmlsim_core used as a caller uses it, which `ty` checks against the stub.

The chart builder `sbmlsim_core.chart` is pure python with type hints, which `ty` checks
the calls against as well.

The file is checked and never run (`ty check python/typecheck`, the job `python bindings`).
A line which must be refused carries an ignore of its rule: with `error-on-warning` the
ignore of a line which is accepted is a diagnostic, so the signatures are held in both
directions.

What it cannot check: the default values (`optimize=True`, `jacobian="auto"`) are invisible to a
type checker, and `jacobian` is a plain `str` in the stub, so an invalid value is not refused.
Those are held by the tests of `python/tests`.
"""

from collections.abc import Mapping
from pathlib import Path
from typing import Any

import numpy
import numpy.typing
import sbmlsim_core
from sbmlsim_core import (
    CompiledModel,
    CompileError,
    FitProblem,
    Instance,
    IntegrationError,
    InternalError,
    LoadedModel,
    LoadError,
    PointError,
    PreparedSimulation,
    RequestError,
    Runtime,
    SbmlsimCoreError,
    SimulationError,
    SimulationOutput,
    UnsupportedError,
    sensitivity,
    stats,
)
from sbmlsim_core.chart import (
    LEGEND_LOCS,
    MARKERS,
    Chart,
    ChartAxis,
    ChartPlot,
    Errors,
    axis_label,
    color_alpha,
    normalize_color,
    short_unit,
)


def use(sbml: str, request: dict[str, Any], fit: dict[str, Any]) -> None:
    """Calls every function and member of the package."""
    contract: str = sbmlsim_core.MODEL_CONTRACT
    version: str = sbmlsim_core.__version__
    simulation: str = sbmlsim_core.SIMULATION_CONTRACT
    fitting: str = sbmlsim_core.FIT_CONTRACT

    compiled: CompiledModel = sbmlsim_core.compile_sbml(sbml)
    sbmlsim_core.compile_sbml(sbml, optimize=False, jacobian="difference_quotient")
    sbmlsim_core.compile_sbml(sbml, parameter_jacobian=False)
    wasm: bytes = compiled.wasm
    description: dict[str, Any] = compiled.description

    runtime: Runtime = Runtime()
    Runtime(max_memory_bytes=1 << 30)
    Runtime(None)
    Runtime(cache=False)
    Runtime(cache_dir=Path("wasmtime-cache"))
    Runtime(cache_dir="wasmtime-cache")
    cache_dir: Path | None = runtime.cache_dir
    model: LoadedModel = runtime.load(wasm, description)
    loaded: dict[str, Any] = model.description
    values: dict[str, float] = model.initial_values()

    instance: Instance = model.instance()
    out: SimulationOutput = instance.simulate(request)
    columns: list[str] = out.columns
    table: numpy.typing.NDArray[numpy.float64] = out.values
    local = out.local_errors
    if local is not None:
        states: list[str] = local.states
        errors: numpy.typing.NDArray[numpy.float64] = local.errors
        weights: numpy.typing.NDArray[numpy.float64] = local.weights
        print(states, errors, weights)
    stats: dict[str, int | float] = instance.stats()
    prepared: PreparedSimulation = model.prepare(request, varying=["k"])
    n_values: int = prepared.n_values
    instance.simulate_prepared(prepared, [0.5])
    instance.simulate_prepared(model.prepare(request))

    answer: dict[str, Any] = model.fit(fit)
    model.fit(fit, threads=2)
    model.fit(fit, threads=None)

    objective: FitProblem = model.fit_problem(fit, threads=2)
    searched: list[str] = objective.parameters
    residuals_of: int = objective.n_residuals
    on: int = objective.threads
    from_point: list[float] = objective.start
    bounds: tuple[list[float], list[float]] = objective.bounds
    at_point: list[float] = objective.values(from_point)
    relative: float = objective.step(0, from_point)
    r: numpy.typing.NDArray[numpy.float64] = objective.residuals(from_point)
    c: float = objective.cost(from_point)
    j: numpy.typing.NDArray[numpy.float64] = objective.jacobian(from_point)
    of: str = objective.jacobian_of
    ran: int = objective.simulations

    results: list[SimulationOutput | PointError] = model.simulate_many([request])
    model.simulate_many([request], threads=4)
    model.simulate_many([request], threads=None)
    with model.batch_executor(threads=2) as executor:
        executor.simulate_many([request])
        executor.simulate_prepared(prepared, [[0.5], [0.8]])
        executor.simulate_plans([(prepared, [0.5]), (prepared, [0.8])])
    executor.close()
    for result in results:
        if isinstance(result, PointError):
            kind: str = result.kind
            message: str = result.message
            flag: int | None = result.flag
            time: float | None = result.time
        else:
            rows: list[str] = result.columns

    print(
        contract, version, simulation, loaded, values, columns, table, stats, cache_dir
    )


def errors(request: dict[str, Any], instance: Instance) -> None:
    """The hierarchy of the errors and the members of an integration error."""
    try:
        instance.simulate(request)
    except IntegrationError as error:
        flag: int = error.flag
        time: float = error.time
        print(flag, time)
    except SimulationError as error:
        print(error)
    except CompileError as error:
        base: SbmlsimCoreError = error
        print(base)


def hierarchy() -> None:
    """The base of every error, held by assignment (a changed base class fails here)."""
    simulation: SimulationError = IntegrationError()
    simulation = RequestError()
    simulation = UnsupportedError()
    simulation = InternalError()
    base: SbmlsimCoreError = simulation
    base = CompileError()
    base = LoadError()
    base = SimulationError()
    print(base)


def refused(model: LoadedModel, request: dict[str, Any], sbml: str) -> None:
    """What the stub must refuse."""
    model.simulate_many([request], 4)  # ty: ignore[too-many-positional-arguments]
    sbmlsim_core.compile_sbml(sbml, False)  # ty: ignore[too-many-positional-arguments]
    sbmlsim_core.compile_sbml(1)  # ty: ignore[invalid-argument-type]
    sbmlsim_core.compile_sbml(sbml, optimize="yes")  # ty: ignore[invalid-argument-type]
    Runtime(max_memory_bytes="big")  # ty: ignore[invalid-argument-type]
    model.simulate_many(request)  # ty: ignore[invalid-argument-type]
    model.simulate_many([request], threads=1.5)  # ty: ignore[invalid-argument-type]
    model.instance().simulate("request")  # ty: ignore[invalid-argument-type]
    wrong: int = model.initial_values()  # ty: ignore[invalid-assignment]
    print(wrong)


def the_runtime_and_the_inline_build() -> None:
    """The runtime and the inline build which a page of one file embeds."""
    from sbmlsim_core import web

    files: dict[str, bytes] = sbmlsim_core.runtime_wasm()
    engine: bytes = files["engine"]
    plot: bytes = sbmlsim_core.plot_wasm()
    sbmlsim_core.plot_wasm("svg")  # ty: ignore[too-many-positional-arguments]
    notices: str = sbmlsim_core.THIRD_PARTY
    script: str = web.script()
    page: str = web.figure_script()
    print(engine, plot, notices, script, page)


def a_polynomial_chaos_expansion(u: numpy.typing.NDArray[numpy.float64]) -> None:
    """The basis of an expansion, its design matrix and its surrogate."""
    basis: numpy.typing.NDArray[numpy.int64] = sbmlsim_core.chaos_expansion(
        22, 4, q=0.75, max_interaction=2
    )
    x: numpy.typing.NDArray[numpy.float64] = sbmlsim_core.chaos_design_matrix(
        u, basis, threads=4
    )
    coefficients = numpy.ones((len(basis), 3))
    values: numpy.typing.NDArray[numpy.float64] = sbmlsim_core.chaos_predict(
        u, basis, coefficients
    )
    sbmlsim_core.chaos_expansion(22, 4, 0.75)  # ty: ignore[too-many-positional-arguments]
    sbmlsim_core.chaos_design_matrix(u, basis, threads=1.5)  # ty: ignore[invalid-argument-type]
    print(x, values)


def a_global_sensitivity_analysis() -> None:
    """The designs of the three global methods and the estimators of their indices."""
    u: numpy.typing.NDArray[numpy.float64] = sensitivity.saltelli(3, 64, seed=1)
    base = sensitivity.sobol_sequence(6, 64, seed=None, skip=0)
    found: dict[str, numpy.typing.NDArray[numpy.float64]] = sensitivity.sobol_indices(
        u[:, :1], 3, second_order=False, seed=1, threads=2
    )
    fast = sensitivity.fast_sample(3, 65, m=4, phases=[0.0, 1.0, 2.0])
    sensitivity.fast_indices(fast[:, :1], 3, resamples=numpy.zeros((3, 10), dtype=int))
    cube = sensitivity.morris_sample(3, 10, levels=4, seed=2)
    effects = sensitivity.morris_effects(cube, cube[:, :1], num_resamples=50, seed=2)
    sensitivity.saltelli(3, 64)  # ty: ignore[missing-argument]
    sensitivity.sobol_indices(u, 3, seed=1.5)  # ty: ignore[invalid-argument-type]
    print(base, found["S1"], effects["mu_star"])


def the_statistics(y: list[float], f: numpy.typing.NDArray[numpy.float64]) -> None:
    """The kernels of numpy and scipy of the analyses of sbmlsim, every function of `stats`."""
    Float = numpy.typing.NDArray[numpy.float64]
    # the metrics of the residuals and the criteria of the data points
    sse: float = stats.sse(f)
    mse: float = stats.mse(y)
    rmse: float = stats.rmse_from_mse(mse)
    aic: float = stats.aic_from_mse(mse, 10, 2)
    bic: float = stats.bic_from_mse(numpy.float64(mse), n=10, k=2)
    r2: float = stats.r_squared(y, f)
    counts: tuple[int, int] = stats.fold_counts(y, f, fold=2.0)
    fold: Float = stats.fold_accepted(y, f, 3)
    within_sd: Float = stats.sd_accepted(y, f, f, k=2.0)
    gmfe: float = stats.gmfe(y, f)
    afe: float = stats.afe(y, f)
    nrmse: float = stats.nrmse(f)
    low, high = stats.guest_limits(f)
    guest: float = stats.within_guest(f, f)
    agreement: tuple[float, float, float, float] | None = stats.subset_agreement(y, f)
    points: int = stats.MIN_POINTS_SUBSET_AGREEMENT
    stats.sse()  # ty: ignore[missing-argument]
    stats.aic_from_mse(mse, 10.5, 2)  # ty: ignore[invalid-argument-type]
    # the Fisher information of a fit and its posterior
    information: Float = stats.gram(numpy.ones((10, 2)))
    fisher: dict[str, Any] = stats.fisher_statistics(
        information,
        1.5,
        10,
        likelihood=False,
        rank_tolerance=stats.DEFAULT_RANK_TOLERANCE,
    )
    lower, upper = stats.fisher_intervals(f, f, ["LOG10", "lin"], level=0.9, dof=8.0)
    stats.fisher_intervals(f, f, ["LINEAR"], dof=None)
    relative: Float = stats.fisher_relative_errors(f, f, ["log"])
    covariance: Float = stats.posterior_laplace(
        information, pids=["k1", "k2"], opid="fit"
    )
    rank: Float = stats.rank_correlation(covariance)
    normal: Float = stats.normal_correlation(covariance)
    log_sd: float = stats.log_sd("LOG10", 0.2)
    cv: float = stats.lognormal_cv(0.3)
    stats.fisher_statistics(information, 1.5)  # ty: ignore[missing-argument]
    stats.posterior_laplace(information, pids=[1, 2])  # ty: ignore[invalid-argument-type]
    # the statistics of the draws of an uncertainty analysis
    mean_se: tuple[float, float, int] = stats.mean_se(f)
    sd_se: tuple[float, float, int] = stats.sd_se(y)
    median: float = stats.quantile(f, 0.5)
    quantiles: Float = stats.quantiles(f, [0.05, 0.95])
    ci: tuple[float, float, float, int] = stats.quantile_ci(f, 0.5, level=0.95)
    index: float = stats.first_order_given_data(f, f, n_bins=None)
    precise: tuple[bool, float] = stats.precise(f, 0.05)
    median_cv: tuple[float, float] = stats.interval_parameters(
        1.0, 2.0, 0.9, lognormal=True
    )
    stats.interval_parameters(1.0, 2.0)  # ty: ignore[missing-argument]
    stats.quantile(f, "0.5")  # ty: ignore[invalid-argument-type]
    # the draws and the correlations of an uncertainty analysis
    u: Float = stats.unit_points(3, 64, design="sobol", seed=1, start=0)
    stats.unit_points(3, 64, design="lhs", seed=2)
    correlated: Float = stats.copula(u, numpy.eye(3))
    rho: float = stats.normal_score_rho(
        0.1, ("normal", 1.0, 0.2), ("log-normal", 2.0, 0.5), scale="log"
    )
    nearest: Float = stats.nearest_correlation(
        numpy.eye(3), max_iterations=50, tol=1e-12
    )
    psd: bool = stats.is_psd(nearest, rtol=stats.PSD_RTOL)
    stats.validate_covariance(
        ["k1", "k2"], numpy.eye(2), kind="correlation", units=None
    )
    stats.validate_covariance(["k1", "k2"], numpy.eye(2), units=["1/hr", None])
    eps: float = stats.EPS
    stats.unit_points(3, 64)  # ty: ignore[missing-argument]
    stats.normal_score_rho(0.1, ("normal", 1.0), ("normal", 1.0, 0.2))  # ty: ignore[invalid-argument-type]
    # the probability statements about predictions
    satisfied = f > 1.0
    greater: Float = stats.p_values(satisfied, 0.9, "greater", margin=stats.MARGIN)
    less: Float = stats.p_values(satisfied, 0.9, "less")
    equivalent: Float = stats.p_values([True, False], 0.5, "equivalent", 0.1)
    sequence: tuple[float, float] = stats.confidence_sequence(satisfied, level=0.95)
    first: int | None = stats.decide(greater, 0.05)
    decision, draws = stats.decision(greater, less, equivalent, 0.05)
    shown: str = decision
    needed: int | None = stats.needed(0.95, 100, 0.9, 0.05, 0.05)
    family: float = stats.family_level(0.95, 4)
    most: int = stats.MAX_NEEDED
    bisections: int = stats.BISECTIONS
    mixture: tuple[float, float] = stats.MIXTURE
    stats.p_values(satisfied, 0.9, "two-sided")  # ty: ignore[invalid-argument-type]
    stats.needed(0.95, 100.0, 0.9, 0.05, 0.05)  # ty: ignore[invalid-argument-type]
    # the visual predictive check of a variability
    groups: Float = stats.group_means(
        numpy.ones((20, 3)), 4, 100, geometric=True, seed=7
    )
    bands: Float = stats.bands(groups, [0.05, 0.5, 0.95])
    stats.group_means(numpy.ones((20, 3)), 4, 100)  # ty: ignore[missing-argument]
    stats.group_means(numpy.ones((20, 3)), 4, 100, seed=1.5)  # ty: ignore[invalid-argument-type]
    print(sse, rmse, aic, bic, r2, counts, fold, within_sd, gmfe, afe, nrmse, low, high)
    print(
        guest, agreement, points, fisher["rank"], lower, upper, relative, rank, normal
    )
    print(log_sd, cv, mean_se, sd_se, median, quantiles, ci, index, precise, median_cv)
    print(
        correlated, rho, psd, eps, sequence, first, draws, shown, needed, family, most
    )
    print(bisections, mixture, bands)


def the_figures(values: list[float]) -> None:
    """The SVG, PNG, geometry and picks of a figure and its colour scales."""
    contract: str = sbmlsim_core.FIGURE_CONTRACT
    spec = "{}"
    svg: str = sbmlsim_core.render_svg(spec)
    png: bytes = sbmlsim_core.render_png(spec, 72.0)
    default_png: bytes = sbmlsim_core.render_png(spec)
    geometry: dict[str, Any] = sbmlsim_core.figure_geometry(spec)
    hit: dict[str, Any] | None = sbmlsim_core.pick(spec, 1.0, 2.0, radius=4.0)
    colors: list[str] = sbmlsim_core.color_scale(
        "viridis", 0.0, 1.0, values, center=0.5, log=False, start=0.0, end=0.85
    )
    stop_colors: list[str] = sbmlsim_core.color_scale(
        [(0.0, "#000000"), (1.0, "#ffffff")], 0.0, 1.0, values
    )
    power_colors: list[str] = sbmlsim_core.color_scale(
        "viridis", 0.0, 1.0, values, gamma=0.5
    )
    image: str = sbmlsim_core.image_png(numpy.zeros((2, 3, 3), dtype=numpy.uint8))
    sbmlsim_core.image_png()  # ty: ignore[missing-argument]
    sbmlsim_core.render_png(spec, dpi="x")  # ty: ignore[invalid-argument-type]
    sbmlsim_core.render_svg(1)  # ty: ignore[invalid-argument-type]
    sbmlsim_core.pick(spec, 1.0)  # ty: ignore[missing-argument]
    sbmlsim_core.color_scale("viridis", 0.0, 1.0)  # ty: ignore[missing-argument]
    sbmlsim_core.color_scale("viridis", 0.0, 1.0, values, end="1")  # ty: ignore[invalid-argument-type]
    names: list[str] = sbmlsim_core.COLOR_SCALES
    width: float = sbmlsim_core.text_width("Smith2001", 13.0, bold=True)
    mono: float = sbmlsim_core.text_width("AUC", 12.0, family="monospace")
    sbmlsim_core.text_width("x")  # ty: ignore[missing-argument]
    print(contract, svg, png, default_png, geometry, hit, colors, names, width, mono)


def the_chart_builder(values: list[float], path: Path) -> None:
    """A chart of `sbmlsim_core.chart`: its plots, elements, spec and files."""
    chart = Chart(400, 300, title="decay", rows=1, columns=2, thinning=None)
    plot: ChartPlot = chart.plot(0, 1, x=ChartAxis(label="time", unit="hr"))
    element: str = plot.curve(values, values, label="A", marker="circle")
    band: str = plot.band(values, values, values)
    # errors of two sides: a tuple `(low, high)` or a mapping of `low` and `high`
    two_sided: Errors = (values, values)
    sides: str = plot.curve(values, values, yerr=two_sided, error_cap=3.0)
    tupled: str = plot.curve(values, values, xerr=(values, values))
    mapped: str = plot.curve(values, values, xerr={"low": values, "high": values})
    bars: str = plot.bar(values, values, err=(values, values), error_cap=3.0)
    spec: dict[str, Any] = chart.spec()
    text: str = chart.to_json()
    svg: str = chart.svg()
    png: bytes = chart.png(dpi=72.0)
    saved: Path = chart.save(path)
    label: str = axis_label("time", "hr")
    short: str = short_unit("nanogram / milliliter")
    color: str | None = normalize_color("tab:red")
    alpha: tuple[str | None, float] = color_alpha((1.0, 0.0, 0.0, 0.5))
    locs: tuple[str, ...] = LEGEND_LOCS
    markers: tuple[str, ...] = MARKERS
    Chart("400")  # ty: ignore[invalid-argument-type]
    chart.plot(x="time")  # ty: ignore[invalid-argument-type]
    chart.save(1)  # ty: ignore[invalid-argument-type]
    short_unit(1)  # ty: ignore[invalid-argument-type]
    print(element, band, spec, text, svg, png, saved, label, short, color, alpha)
    print(locs, markers, sides, tupled, mapped, bars)


def equation_exports(source: Path) -> None:
    """The same shared equation source exports native code and linked data."""
    system = sbmlsim_core.OdeSystem.from_sbml(source)
    identifier: str | None = system.model_id
    code: str = system.render("python", simulator=False)
    output: Path = system.write("model.py", fmt="diffrax")
    document: str = system.render("typst", symbols="name", standalone=False)
    equations: dict[str, Any] = system.typeset({"symbols": {"S": "model/Species:S"}})
    formats: Mapping[str, sbmlsim_core.OdeFormat] = sbmlsim_core.ODE_FORMATS


def petab_problem(directory: Path, files: Mapping[str, bytes]) -> None:
    """A PEtab v2 problem read, evaluated and fitted in Rust."""
    from sbmlsim_core import petab

    problem: petab.Problem = petab.Problem.from_yaml(directory / "problem.yaml")
    problem = petab.Problem.from_files(files, "problem.yaml", relative_tolerance=1e-8)
    names: list[str] = problem.parameters
    evaluation: dict[str, Any] = problem.evaluate(problem.nominal)
    llh: float = problem.llh()
    objective: FitProblem = problem.fit_problem(threads=2)
    answer: dict[str, Any] = problem.fit({"max_iterations": 50}, threads=2)
    problem.evaluate(1.0)  # ty: ignore[invalid-argument-type]
    del names, evaluation, llh, objective, answer
