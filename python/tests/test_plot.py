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
    # 600 x 400 at 1224 dpi is 10200 x 6800 pixels, more than the 64 Mpx of an image
    with pytest.raises(RuntimeError, match=r"10200 by 6800 pixels"):
        sbmlsim_core.render_png(json.dumps(CURVE), 72.0 * 17)
    with pytest.raises(RuntimeError, match="dpi"):
        sbmlsim_core.render_png(json.dumps(CURVE), 0.0)


def test_nan_is_no_json() -> None:
    spec = json.loads(json.dumps(CURVE))
    spec["plots"][0]["elements"][0]["y"][1] = float("nan")
    # json.dumps writes the token NaN, which is no JSON: the error names the gap of JSON
    with pytest.raises(
        ValueError, match=r"plots\[0\]\.elements\[0\]\.y\[1\]: .*`null`"
    ):
        sbmlsim_core.render_svg(json.dumps(spec))
    # a gap written as None is null
    spec["plots"][0]["elements"][0]["y"][1] = None
    assert sbmlsim_core.render_svg(json.dumps(spec, allow_nan=False)).startswith("<svg")


def test_geometry_reports_the_drawn_size() -> None:
    geometry: dict[str, Any] = sbmlsim_core.figure_geometry(json.dumps(CURVE))
    assert (geometry["width"], geometry["height"]) == (600, 400)
    outside = {**CURVE, "legend": {"position": "outside"}}
    spec = json.dumps(outside)
    geometry = sbmlsim_core.figure_geometry(spec)
    assert geometry["width"] > 600
    svg = sbmlsim_core.render_svg(spec)
    assert f' width="{geometry["width"]:g}"' in svg.split(">", 1)[0]
    png = sbmlsim_core.render_png(spec, 144.0)
    assert struct.unpack(">II", png[16:24]) == (
        round(geometry["width"] * 2),
        round(geometry["height"] * 2),
    )


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


def test_color_scale_part_of_the_map() -> None:
    # the lines of a scan on viridis over 0..0.85: the high end takes the colour at 0.85
    full = sbmlsim_core.color_scale("viridis", 0.0, 1.0, [0.0, 0.85])
    part = sbmlsim_core.color_scale("viridis", 2.0, 8.0, [2.0, 8.0, 100.0], end=0.85)
    assert part == [full[0], full[1], full[1]]
    assert part[1] != "#fde725"
    shifted = sbmlsim_core.color_scale("viridis", 0.0, 1.0, [0.0], start=0.5)
    assert shifted == sbmlsim_core.color_scale("viridis", 0.0, 1.0, [0.5])
    for start, end in [(-0.1, 1.0), (0.0, 1.5), (0.5, 0.5), (0.6, 0.4)]:
        with pytest.raises(ValueError, match="start"):
            sbmlsim_core.color_scale("viridis", 0.0, 1.0, [0.5], start=start, end=end)


def test_colorbars_render() -> None:
    spec = {
        "contract": "figure/1",
        "width": 500,
        "height": 400,
        "grid": {"rows": 1, "columns": 1},
        "plots": [
            {
                "id": "p",
                "row": 0,
                "column": 0,
                "x": {},
                "y": {},
                "colorbars": [
                    {
                        "scale": {"name": "viridis", "low": 0, "high": 3, "end": 0.85},
                        "categorical": True,
                        "ticks": [0, 1, 2, 3],
                        "tick_labels": ["a", "b", "c", "d"],
                    },
                    {"scale": {"name": "magma", "low": 1, "high": 100, "log": True}},
                ],
                "elements": [{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}],
            }
        ],
    }
    svg = sbmlsim_core.render_svg(json.dumps(spec))
    assert svg.count('class="sbmlsim-colorbar"') == 2
    for color in sbmlsim_core.color_scale("viridis", 0, 3, [0, 1, 2, 3], end=0.85):
        assert f'fill="{color}"' in svg


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
