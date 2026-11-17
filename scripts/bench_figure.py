# /// script
# requires-python = ">=3.14,<3.15"
# dependencies = [
#     "matplotlib>=3.10",
# ]
# ///
"""The tables and the figure of the benchmark of the hosts against roadrunner.

    uv run scripts/bench_figure.py target/bench/sb --out docs/images

Reads the JSON files one run of `bench/sbmlsim.toml` wrote, `<prefix>-roadrunner.json` and one
per host (`scripts/roadrunner_reference.py` and `sbmlsim-core bench`, see `bench/SBMLSIM.md`),
and writes the markdown tables of `docs/benchmarks.md` on stdout and the figure of the page as
two files, one for each mode of the documentation (`--out`, `benchmark-hosts-light.png` and
`-dark.png`). Nothing regenerates the page: a new measurement is a run of the benchmark, a run
of this script and a commit of its tables and its figures.

The figure is a panel per model, a row per engine and a logarithmic axis, the dot of the median
of a configuration of a solver. The upper row of panels is the time of one simulation and the
lower the ratio against roadrunner of the same solver, which puts roadrunner itself at 1 and
draws its line there. Every panel holds every engine in the order of the tables, so a row is
one engine across the whole figure, and identity is the label of the row and not a colour: the
two colours are the solvers (the validated categorical slots 1 and 2, blue and orange, stepped
for each mode).
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, NamedTuple

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.figure import Figure
from matplotlib.lines import Line2D
from matplotlib.ticker import FuncFormatter, LogLocator, NullLocator

#: The engines of the tables and of the figure, in order, by the suffix of their file.
ENGINES: tuple[tuple[str, str], ...] = (
    ("roadrunner", "roadrunner"),
    ("native", "native"),
    ("wasmtime", "wasmtime"),
    ("node", "node"),
    ("chrome", "chrome"),
)

#: The solvers of a model, in the order they are drawn and tabulated.
SOLVERS: tuple[str, ...] = ("dense", "klu")

#: The colours of the solvers: the categorical slots 1 and 2, light and dark.
# the first five slots of the categorical theme, which validate on the adjacent
# pairlist in both modes; identity is the row label and its position, the colour
# ties one engine across the panels
COLORS: dict[str, dict[str, str]] = {
    "light": {
        "roadrunner": "#2a78d6",
        "native": "#eb6834",
        "wasmtime": "#1baf7a",
        "node": "#eda100",
        "chrome": "#e87ba4",
    },
    "dark": {
        "roadrunner": "#3987e5",
        "native": "#d95926",
        "wasmtime": "#199e70",
        "node": "#c98500",
        "chrome": "#d55181",
    },
}

MARKERS: dict[str, str] = {"dense": "o", "klu": "D"}

INK: dict[str, dict[str, str]] = {
    "light": {
        "surface": "#ffffff",
        "primary": "#0b0b0b",
        "secondary": "#52514e",
        "grid": "#dededc",
    },
    "dark": {
        "surface": "#1a1a19",
        "primary": "#f0efec",
        "secondary": "#c3c2b7",
        "grid": "#3a3a38",
    },
}


def _plain(value: float, _position: int = 0) -> str:
    """Format a tick of a logarithmic axis as a plain number.

    Args:
        value: The value of the tick.
        _position: The position matplotlib passes and the formatter ignores.

    Returns:
        The number to two significant digits and without an exponent, e.g. `0.02`.
    """
    if value <= 0:
        return ""
    return f"{value:.2g}"


class Cell(NamedTuple):
    """One measured configuration of a model."""

    model: str
    engine: str
    solver: str
    ms: float
    states: int


def read_cells(prefix: Path) -> tuple[list[Cell], list[str]]:
    """Read every measured configuration of a run of the benchmark.

    Args:
        prefix: The path the files share, e.g. `target/bench/sb`.

    Returns:
        The cells and the models in the order of the files.

    Raises:
        SystemExit: A file of an engine is missing.
    """
    cells: list[Cell] = []
    models: list[str] = []
    for engine, suffix in ENGINES:
        path = prefix.with_name(f"{prefix.name}-{suffix}.json")
        if not path.exists():
            raise SystemExit(f"{path} is missing, see bench/SBMLSIM.md for the run")
        payload: dict[str, Any] = json.loads(path.read_text())
        for model in payload["models"]:
            if model["name"] not in models:
                models.append(model["name"])
            for config in model["configs"]:
                if config["solver"] not in SOLVERS:
                    continue
                cells.append(
                    Cell(
                        model=model["name"],
                        engine=engine,
                        solver=config["solver"],
                        ms=config["median_s"] * 1e3,
                        states=config.get("states") or model.get("states") or 0,
                    )
                )
    return cells, models


def measured_solvers(
    by: dict[tuple[str, str, str], Cell], model: str, engines: list[str]
) -> list[str]:
    """Get the solvers of a model which every engine was measured with.

    A large model is measured with KLU alone, which is what a simulator picks for it (dense is
    5.9 s against 0.070 s on the perfusion model), so a solver which is missing for one engine
    is left out of the tables and of the figure rather than shown as a gap.

    Args:
        by: The cells by model, engine and solver.
        model: The model.
        engines: The engines which must all have the cell.

    Returns:
        The solvers, in the order of `SOLVERS`.
    """
    return [s for s in SOLVERS if all((model, e, s) in by for e in engines)]


def tables(cells: list[Cell], models: list[str]) -> str:
    """Render the markdown tables of the times and of the ratios.

    One table of twelve columns is cut off at the width of a page of the documentation, so the
    times and the ratios are a table each.

    Args:
        cells: The measured configurations.
        models: The models in order.

    Returns:
        The two tables, with the heading of each, one row per model and solver.
    """
    by = {(c.model, c.engine, c.solver): c for c in cells}
    engines = [e for e, _ in ENGINES]
    out: list[str] = []
    for measure in ("ms", "ratio"):
        shown = (
            engines if measure == "ms" else [e for e in engines if e != "roadrunner"]
        )
        out.append(
            "### Milliseconds per simulation"
            if measure == "ms"
            else "### The ratio against roadrunner with the same solver"
        )
        out.append("")
        out.append("| model (states) | solver | " + " | ".join(shown) + " |")
        out.append("|---|---|" + "---:|" * len(shown))
        for model in models:
            for solver in measured_solvers(by, model, engines):
                reference = by[(model, "roadrunner", solver)]
                states = reference.states
                row = [f"{model}&nbsp;({states})", solver]
                for engine in shown:
                    cell = by.get((model, engine, solver))
                    if cell is None:
                        row.append("")
                    elif measure == "ms":
                        row.append(f"{cell.ms:.3g}")
                    else:
                        row.append(f"{cell.ms / reference.ms:.2f}")
                out.append("| " + " | ".join(row) + " |")
        out.append("")
    return "\n".join(out).strip()


def _thin_ticks(fig: Figure) -> None:
    """Hide the tick labels of an axis which would overlap their neighbour.

    A logarithmic axis of less than a decade gets more ticks from matplotlib
    than its width carries, so the labels are thinned after the layout, when
    their drawn width is known.

    Args:
        fig: The figure, drawn.
    """
    renderer = fig.canvas.get_renderer()
    for ax in fig.get_axes():
        kept: float | None = None
        for label in ax.xaxis.get_ticklabels():
            if not label.get_text():
                continue
            box = label.get_window_extent(renderer=renderer)
            if kept is not None and box.x0 < kept + 3.0:
                label.set_visible(False)
                continue
            kept = box.x1


def figure(cells: list[Cell], models: list[str], mode: str) -> Figure:
    """Draw the figure of one mode of the documentation.

    The two measures are two subfigures with a gap between them, each with its
    own heading and an axis label under every panel, so a time is never read as
    a ratio. A colour is an engine and a marker is a solver.

    Args:
        cells: The measured configurations.
        models: The models in order.
        mode: `light` or `dark`.

    Returns:
        The figure, a panel per model and measure.
    """
    ink, color = INK[mode], COLORS[mode]
    by = {(c.model, c.engine, c.solver): c for c in cells}
    engines = [e for e, _ in ENGINES]
    headings = {"ms": "Milliseconds per simulation", "ratio": "Ratio to roadrunner"}

    fig = plt.figure(figsize=(7.6, 1.5 * len(models) + 0.8), facecolor=ink["surface"])
    columns = fig.subfigures(1, 2, wspace=0.14)
    for column, measure in enumerate(("ms", "ratio")):
        panel = columns[column]
        panel.set_facecolor(ink["surface"])
        panel.suptitle(
            headings[measure],
            fontsize=9.5,
            color=ink["primary"],
            y=0.995,
            fontweight="medium",
        )
        axes = panel.subplots(len(models), 1)
        for row, model in enumerate(models):
            ax = axes[row]
            ax.set_facecolor(ink["surface"])
            for spine in ("top", "right", "left"):
                ax.spines[spine].set_visible(False)
            ax.spines["bottom"].set_color(ink["grid"])
            ax.tick_params(colors=ink["secondary"], labelsize=8, length=3, width=0.8)
            ax.xaxis.grid(True, color=ink["grid"], linewidth=0.6, zorder=0)
            ax.set_axisbelow(True)
            ax.set_xscale("log")
            ax.xaxis.set_major_locator(
                LogLocator(base=10.0, subs=(1.0, 2.0, 5.0), numticks=6)
            )
            ax.xaxis.set_minor_locator(NullLocator())
            ax.xaxis.set_major_formatter(FuncFormatter(_plain))
            ax.set_ylim(len(engines) - 0.5, -0.5)
            ax.set_yticks(range(len(engines)))
            ax.set_yticklabels(
                engines if column == 0 else [""] * len(engines),
                fontsize=7.5,
                color=ink["primary"],
            )
            ax.tick_params(axis="y", length=0 if column else 3)
            solvers = measured_solvers(by, model, engines)
            if measure == "ratio":
                ax.axvline(1.0, color=ink["secondary"], linewidth=1.0, zorder=1)
                values = [
                    by[(model, e, s)].ms / by[(model, "roadrunner", s)].ms
                    for e in engines
                    for s in solvers
                ]
                ax.set_xlim(min(values) / 1.35, max(values) * 1.35)
            for position, engine in enumerate(engines):
                for solver in solvers:
                    cell = by.get((model, engine, solver))
                    reference = by.get((model, "roadrunner", solver))
                    if cell is None or reference is None:
                        continue
                    value = cell.ms if measure == "ms" else cell.ms / reference.ms
                    # a model measured with one solver has its dot on the row of its engine
                    offset = (
                        0.0
                        if len(solvers) == 1
                        else (-0.16 if solver == "dense" else 0.16)
                    )
                    ax.plot(
                        value,
                        position + offset,
                        marker=MARKERS[solver],
                        markersize=5.5,
                        markerfacecolor=color[engine],
                        markeredgecolor=ink["surface"],
                        markeredgewidth=1.2,
                        linestyle="none",
                        zorder=3,
                    )
            if column == 0:
                states = next(
                    (c.states for c in cells if c.model == model and c.states), 0
                )
                ax.set_ylabel(
                    f"{model}\n{states} states",
                    fontsize=8.5,
                    color=ink["primary"],
                    linespacing=1.4,
                    labelpad=10,
                )
            ax.set_xlabel(
                "ms per simulation" if measure == "ms" else "ratio to roadrunner",
                fontsize=7.5,
                color=ink["secondary"],
                labelpad=2,
            )
        # the left column carries the engine names and the model heading
        panel.subplots_adjust(
            left=0.34 if column == 0 else 0.07,
            right=0.98,
            hspace=0.85,
            top=0.94,
            bottom=0.09,
        )
    handles = [
        Line2D(
            [],
            [],
            marker=MARKERS[solver],
            markersize=5.5,
            markerfacecolor=ink["secondary"],
            markeredgecolor=ink["surface"],
            markeredgewidth=1.2,
            linestyle="none",
            label=f"{solver} solver",
        )
        for solver in SOLVERS
    ]
    legend = fig.legend(
        handles=handles,
        loc="lower center",
        ncol=2,
        frameon=False,
        fontsize=8,
        labelcolor=ink["primary"],
        bbox_to_anchor=(0.5, 0.0),
    )
    legend.set_zorder(5)
    fig.canvas.draw()
    _thin_ticks(fig)
    return fig


def main() -> None:
    """Write the tables on stdout and the figures into the output directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("prefix", type=Path, help="the path the JSON files share")
    parser.add_argument("--out", type=Path, default=Path("docs/images"))
    arguments = parser.parse_args()

    cells, models = read_cells(arguments.prefix)
    print(tables(cells, models))
    arguments.out.mkdir(parents=True, exist_ok=True)
    for mode in ("light", "dark"):
        fig = figure(cells, models, mode)
        path = arguments.out / f"benchmark-hosts-{mode}.png"
        fig.savefig(path, dpi=200, facecolor=fig.get_facecolor())
        plt.close(fig)
        print(f"\nwrote {path}")


if __name__ == "__main__":
    main()
