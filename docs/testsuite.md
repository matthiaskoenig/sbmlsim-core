# Test suite

`sbmlsim-core testsuite testdata/cases` compiles and simulates the semantic cases of the SBML Test Suite and compares them with the baseline `testdata/testsuite_baseline.json`. Every case of the suite is one cell below: hover it, or walk the grid with the arrow keys, for its identifier, its outcome and either the largest difference from the expected results or the reason the engine does not compile it.

<div class="suite" data-src="../data/testsuite.json">
  <p class="suite-fallback">The grid of the cases needs JavaScript; the counts and the reasons are in the tables below.</p>
</div>

The engine passes 1754 of 1789 cases, including all 53 delay cases and 123 algebraic-rule cases. Every case the compiler accepts passes. The remaining 35 cases are reported as `not_compiled` when preparation or compilation refuses an unsupported feature.

| reason | cases |
|---|---:|
| not converted: fast reaction (libsbml conversion refuses a fast reaction) | 35 |

## Running the suite

The cases are prepared first (uv, libsbml):

```bash
uv run scripts/prepare_testsuite.py --out testdata/cases
sbmlsim-core testsuite testdata/cases                          # the analytic Jacobian
sbmlsim-core testsuite testdata/cases --jacobian difference    # the colored difference quotient
sbmlsim-core testsuite testdata/cases --no-optimize            # without the optimizer
sbmlsim-core testsuite testdata/cases --write-baseline         # the baseline from a run
sbmlsim-core testsuite testdata/cases --model-code c           # the C target (--features native)
```

`--cases 00001-00100` runs some cases, `--jobs` sets the threads, `--json` writes the result of every case and `--export DIR` writes every passing case for the JavaScript tests, which compare the CSV of Node with the one of wasmtime byte for byte. In every mode each passing case is simulated again to check that it repeats bitwise; a case which passes or fails otherwise than the baseline says fails the run.

`--model-code c` simulates every case on the native engine with the native model code of the [C target](rust.md#the-c-target) instead of `model.wasm`, against the same baseline: a case the C target refuses (a delay) is `refused`, listed in the summary with its feature and no difference to the baseline. Of the 1789 cases, 1701 pass, 35 are not compiled as in the baseline and 53 are refused, all for a delay.

CI runs the whole suite in the three modes (the job `testsuite-parity`), with the C target (`testsuite-c-target`) and the parity of Node with wasmtime over every passing case (`same_bits_as_wasmtime`).

The grid above is written from a run and committed, it is not built from a live run of the documentation:

```bash
sbmlsim-core testsuite testdata/cases --json target/suite.json
uv run scripts/testsuite_data.py target/suite.json      # into docs/data/testsuite.json
```

<style>
.suite { margin: 1.2rem 0 1.6rem; }
.suite-legend { display: flex; flex-wrap: wrap; gap: 1.1rem; margin-bottom: .6rem; font-size: .76rem; }
.suite-legend span { display: flex; align-items: center; gap: .36rem; }
.suite-key { width: .72rem; height: .72rem; border-radius: 2px; flex: none; }
.suite-key.pass { background: #0ca30c; }
.suite-key.unsupported { background: var(--suite-grey, #c3c3c0); }
.suite-key.fail { background: #d03b3b; }
.suite-grid { display: flex; flex-wrap: wrap; gap: 2px; }
.suite-grid:focus { outline: none; }
.suite-cell { width: 11px; height: 11px; border-radius: 2px; background: #0ca30c; cursor: default; }
.suite-cell.unsupported { background: var(--suite-grey, #c3c3c0); }
.suite-cell.fail { background: repeating-linear-gradient(135deg, #d03b3b 0 3px, #ffffff 3px 4px); }
.suite-cell.is-active { outline: 2px solid var(--md-default-fg-color, #000); outline-offset: 1px; }
.suite-tip { position: fixed; z-index: 20; max-width: 24rem; padding: .42rem .6rem; border-radius: 4px;
  background: var(--md-default-fg-color, #1a1a19); color: var(--md-default-bg-color, #fff);
  font-size: .74rem; line-height: 1.35; pointer-events: none; opacity: 0; transition: opacity .08s; }
.suite-tip.is-shown { opacity: 1; }
.suite-tip b { font-variant-numeric: tabular-nums; }
.suite-note { margin-top: .5rem; font-size: .74rem; color: var(--md-default-fg-color--light, #666); }
[data-md-color-scheme="slate"] .suite { --suite-grey: #4a4a47; }
@media (prefers-color-scheme: dark) { :root:not([data-md-color-scheme="default"]) .suite { --suite-grey: #4a4a47; } }
</style>

<script>
(() => {
  const KINDS = { pass: "passes", unsupported: "not supported", fail: "fails" };
  const root = document.querySelector(".suite");
  if (!root) return;
  fetch(root.dataset.src).then((r) => r.json()).then((data) => {
    const counts = data.counts;
    root.innerHTML = "";

    const legend = document.createElement("div");
    legend.className = "suite-legend";
    for (const kind of ["pass", "unsupported", "fail"]) {
      const item = document.createElement("span");
      const key = document.createElement("i");
      key.className = "suite-key " + kind;
      item.appendChild(key);
      item.appendChild(document.createTextNode(
        counts[kind] + " " + KINDS[kind]
      ));
      legend.appendChild(item);
    }
    root.appendChild(legend);

    const grid = document.createElement("div");
    grid.className = "suite-grid";
    grid.setAttribute("role", "list");
    grid.setAttribute("aria-label",
      data.n_cases + " cases of the SBML Test Suite at commit " + data.suite_commit.slice(0, 7));
    const describe = (c) => {
      let text = "Case " + c.id + ", " + KINDS[c.kind];
      if (c.reason) text += ": " + c.reason;
      else if (c.difference !== undefined) text += ", largest difference " + c.difference;
      return text;
    };
    data.cases.forEach((c, i) => {
      const cell = document.createElement("div");
      cell.className = "suite-cell " + c.kind;
      cell.setAttribute("role", "listitem");
      cell.tabIndex = i === 0 ? 0 : -1;
      cell.title = describe(c);
      cell.dataset.index = String(i);
      grid.appendChild(cell);
    });
    root.appendChild(grid);

    const note = document.createElement("p");
    note.className = "suite-note";
    note.textContent = "Suite commit " + data.suite_commit.slice(0, 7)
      + ", " + data.n_cases + " semantic cases, in the order of their identifier.";
    root.appendChild(note);

    const tip = document.createElement("div");
    tip.className = "suite-tip";
    tip.setAttribute("role", "status");
    document.body.appendChild(tip);

    let active = null;
    const show = (cell, x, y) => {
      const c = data.cases[Number(cell.dataset.index)];
      let html = "<b>" + c.id + "</b> &middot; " + KINDS[c.kind];
      if (c.reason) html += "<br>" + c.reason.replace(/[<>&]/g, (ch) =>
        ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" })[ch]);
      else if (c.difference !== undefined) html += "<br>largest difference " + c.difference;
      tip.innerHTML = html;
      tip.classList.add("is-shown");
      const box = tip.getBoundingClientRect();
      const left = Math.min(Math.max(8, x + 12), window.innerWidth - box.width - 8);
      // below the anchor, which keeps the legend clear, unless the window ends first
      const below = y + 18;
      const top = below + box.height < window.innerHeight - 8 ? below : y - box.height - 12;
      tip.style.left = left + "px";
      tip.style.top = top + "px";
      if (active && active !== cell) active.classList.remove("is-active");
      active = cell;
    };
    const hide = () => {
      tip.classList.remove("is-shown");
      if (active) active.classList.remove("is-active");
      active = null;
    };

    grid.addEventListener("mousemove", (e) => {
      const cell = e.target.closest(".suite-cell");
      if (cell) show(cell, e.clientX, e.clientY); else hide();
    });
    grid.addEventListener("mouseleave", hide);
    grid.addEventListener("focusin", (e) => {
      const cell = e.target.closest(".suite-cell");
      if (!cell) return;
      cell.classList.add("is-active");
      const box = cell.getBoundingClientRect();
      show(cell, box.left, box.bottom);
    });
    grid.addEventListener("focusout", hide);
    grid.addEventListener("keydown", (e) => {
      const cell = e.target.closest(".suite-cell");
      if (!cell) return;
      const cells = grid.children;
      const perRow = Math.max(1, Math.round(grid.clientWidth / 13));
      const i = Number(cell.dataset.index);
      const step = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: perRow, ArrowUp: -perRow }[e.key];
      if (step === undefined) return;
      const next = cells[Math.min(cells.length - 1, Math.max(0, i + step))];
      if (!next) return;
      e.preventDefault();
      cell.tabIndex = -1;
      next.tabIndex = 0;
      next.focus();
    });
  }).catch(() => {
    root.textContent = "The grid of the cases could not be loaded; the counts are in the tables below.";
  });
})();
</script>
