"""Draw the figures of the page of the chart builder, `docs/chart.md`.

The `python` blocks of the page run in the order of the page and in one
namespace, as a reader runs them, in a temporary directory, so the files a block
saves do not end up in the repository. After every block the chart `chart` of
the namespace, when it is another one than after the block before, is drawn as
`docs/images/chart/chart-<n>.svg`, which the page shows below the block.

```bash
uv run --no-project --with <wheel> --with pint scripts/chart_docs.py          # draw them
uv run --no-project --with <wheel> --with pint scripts/chart_docs.py --check  # compare
```

The page runs on the installed package, a wheel of this checkout, so the
figures are those of its renderer: run the script after a change of the page,
of the chart builder or of what the renderer draws, and commit the figures with
it. `--check` fails when a figure differs from the committed one or a block
fails.
"""

from __future__ import annotations

import argparse
import contextlib
import os
import re
import sys
import tempfile
from pathlib import Path

from sbmlsim_core.chart import Chart

ROOT = Path(__file__).resolve().parent.parent
PAGE = ROOT / "docs" / "chart.md"
IMAGES = ROOT / "docs" / "images" / "chart"

_BLOCK = re.compile(r"^```python\n(.*?)^```", re.DOTALL | re.MULTILINE)


def figures() -> list[str]:
    """Run the blocks of the page and get the SVG of every chart it draws."""
    namespace: dict[str, object] = {"__name__": "__main__"}
    svgs: list[str] = []
    last = None
    with tempfile.TemporaryDirectory() as folder, contextlib.chdir(folder):
        for k, match in enumerate(_BLOCK.finditer(PAGE.read_text("utf-8")), 1):
            try:
                exec(
                    compile(match.group(1), f"{PAGE.name} block {k}", "exec"), namespace
                )
            except Exception as error:
                raise SystemExit(
                    f"The block {k} of {PAGE.name} fails: {error}"
                ) from error
            chart = namespace.get("chart")
            if isinstance(chart, Chart) and chart is not last:
                svgs.append(chart.svg())
                last = chart
    return svgs


def main() -> int:
    """Draw or compare the figures of the page."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="compare, write nothing")
    args = parser.parse_args()
    svgs = figures()
    referenced = re.findall(r"images/chart/(chart-\d+\.svg)", PAGE.read_text("utf-8"))
    names = [f"chart-{n:02d}.svg" for n in range(1, len(svgs) + 1)]
    if referenced != names:
        print(f"The page shows {referenced}, its blocks draw {names}.")
        return 1
    if args.check:
        stale = [
            name
            for name, svg in zip(names, svgs, strict=True)
            if not (IMAGES / name).is_file()
            or (IMAGES / name).read_text("utf-8") != svg
        ]
        extra = sorted({p.name for p in IMAGES.glob("*.svg")} - set(names))
        for name in stale:
            print(f"{name} is out of date, run scripts/chart_docs.py")
        for name in extra:
            print(f"{name} is no figure of the page")
        return 1 if stale or extra else 0
    IMAGES.mkdir(parents=True, exist_ok=True)
    for path in IMAGES.glob("*.svg"):
        if path.name not in names:
            path.unlink()
    for name, svg in zip(names, svgs, strict=True):
        (IMAGES / name).write_text(svg, "utf-8")
    print(f"{len(svgs)} figures in {os.path.relpath(IMAGES, ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
