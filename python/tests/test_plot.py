"""The figures: their SVG and PNG, their geometry and picks, and their colour scales."""

import json
import os
import statistics
import struct
import time
from pathlib import Path
from typing import Any

import pytest
import sbmlsim_core

ROOT = Path(__file__).resolve().parents[2]
PLOT = ROOT / "crates" / "plot" / "tests"

CURVE = {
    "contract": "figure/1",
    "width": 600,
    "height": 400,
    "grid": {"rows": 1, "columns": 1},
    "plots": [
        {
            "id": "p",
            "row": 0,
            "column": 0,
            "x": {"label": "t"},
            "y": {"label": "c"},
            "elements": [
                {
                    "id": "c",
                    "label": "conc",
                    "kind": "curve",
                    "x": [0, 1, 2, 3, 4],
                    "y": [1, 4, 2, 8, 5],
                }
            ],
        }
    ],
}


def test_contract() -> None:
    assert sbmlsim_core.FIGURE_CONTRACT == "figure/1"


def test_render_svg_fixture() -> None:
    spec = (PLOT / "specs" / "benchmark.json").read_text()
    golden = (PLOT / "golden" / "benchmark.svg").read_text()
    assert sbmlsim_core.render_svg(spec) == golden


def test_invalid_spec_value_error() -> None:
    spec = json.loads((PLOT / "specs" / "benchmark.json").read_text())
    spec["plots"][0]["elements"][0]["y"] = spec["plots"][0]["elements"][0]["y"][:-1]
    with pytest.raises(ValueError, match=r"plots\[0\]\.elements\[0\]\.y"):
        sbmlsim_core.render_svg(json.dumps(spec))
    with pytest.raises(ValueError):
        sbmlsim_core.figure_geometry("{")


def test_render_png_size() -> None:
    png = sbmlsim_core.render_png(json.dumps(CURVE), 144.0)
    assert png[:8] == b"\x89PNG\r\n\x1a\n"
    width, height = struct.unpack(">II", png[16:24])
    assert (width, height) == (1200, 800)
    default = sbmlsim_core.render_png(json.dumps(CURVE))
    assert struct.unpack(">II", default[16:24]) == (1200, 800)


def test_render_png_error() -> None:
    with pytest.raises(ValueError):
        sbmlsim_core.render_png("{}")


def test_geometry_and_pick() -> None:
    spec = json.dumps(CURVE)
    geometry: dict[str, Any] = sbmlsim_core.figure_geometry(spec)
    plot = geometry["plots"][0]
    assert plot["id"] == "p"
    rect, sx, sy = plot["rect"], plot["x"], plot["y"]
    px = rect["x"] + (3 - sx["low"]) / (sx["high"] - sx["low"]) * rect["width"]
    py = (
        rect["y"]
        + rect["height"]
        - (8 - sy["low"]) / (sy["high"] - sy["low"]) * rect["height"]
    )
    hit = sbmlsim_core.pick(spec, px + 1.0, py - 1.0, 5.0)
    assert hit is not None
    assert (hit["plot"], hit["element"], hit["index"]) == ("p", "c", 3)
    assert hit["values"]["y"] == 8.0
    assert sbmlsim_core.pick(spec, -50.0, -50.0) is None


def test_color_scale_viridis() -> None:
    colors = sbmlsim_core.color_scale(
        "viridis", 0.0, 1.0, [0.0, 0.5, 1.0, float("nan")]
    )
    assert len(colors) == 4
    assert colors[0] == "#440154"
    assert colors[2] == "#fde725"
    assert colors[3] == "#d3d3d3"
    assert len(sbmlsim_core.color_scale("rdbu", -1.0, 1.0, [0.0], center=0.0)) == 1
    assert len(sbmlsim_core.color_scale("magma", 1.0, 100.0, [10.0], log=True)) == 1


def test_plot_wasm_magic() -> None:
    wasm = sbmlsim_core.plot_wasm()
    assert isinstance(wasm, bytes)
    assert wasm.startswith(b"\0asm\x01\0\0\0")
    assert wasm not in sbmlsim_core.runtime_wasm().values()


def test_color_scale_unknown_name() -> None:
    with pytest.raises(ValueError, match="viridis"):
        sbmlsim_core.color_scale("jet", 0.0, 1.0, [0.5])


@pytest.mark.skipif(
    bool(os.environ.get("CI")) and os.environ.get("SBMLSIM_CORE_BENCH") != "1",
    reason="timing on CI needs SBMLSIM_CORE_BENCH=1",
)
def test_benchmark_time() -> None:
    spec = (PLOT / "specs" / "benchmark.json").read_text()
    times = []
    for _ in range(50):
        start = time.perf_counter()
        sbmlsim_core.render_svg(spec)
        times.append(time.perf_counter() - start)
    assert statistics.median(times) < 0.005
