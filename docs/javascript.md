# JavaScript

`js/` is the JavaScript host of sbmlsim-core: plain ES modules without dependencies and without APIs of Node, so they run in Node and in browsers alike.

| module | export of `js/package.json` | what it is |
|---|---|---|
| `loader.mjs` | `.` | runs `engine.wasm`, `mathlib.wasm` and a `model.wasm` with the bits of wasmtime |
| `compile.mjs` | `./compile` | compiles an SBML document into `model.wasm` and its description with `compiler.wasm` |
| `pool.mjs` (with `worker.mjs`) | `./pool` | a pool of workers which run the loader off the main thread |
| `panels.mjs` | `./panels` | the routes of the panel service of sbmlsim, answered inside a page from a bundle |
| `plot.mjs` | `./plot` | draws the figures of sbmlsim with `plot.wasm`, see [Figures](plot.md) |
| `figure.mjs` (since 0.28.0) | `./figure` | makes a drawn figure interactive in a page: legend toggles, selection and the read-out of a point |
| `simulate.mjs` | the command `sbmlsim-core-simulate` | the CSV of `sbmlsim-core simulate` in Node |

The package is published to npm as [`sbmlsim-core`](https://www.npmjs.com/package/sbmlsim-core), with the `compiler.wasm`, `engine.wasm`, `mathlib.wasm` and `plot.wasm` (the renderer of the figures of sbmlsim, `crates/plot-wasm`) of its release in `runtime/`:

```bash
npm install sbmlsim-core
```

A page imports the modules from its own files or from a CDN (`https://cdn.jsdelivr.net/npm/sbmlsim-core@0.31.1/panels.mjs`); a pool of a module of another origin starts its workers through a module of the page's own origin, which browsers require of a worker. A checkout serves the files of `js/` with the runtime of a build (`sbmlsim-core runtime -o js/runtime`, see [Installation](installation.md#building)). A release publishes the package from the tag `vX.Y.Z` (`.github/workflows/publish.yml`, trusted publishing with provenance, together with the python package on PyPI); the version of `js/package.json` is the one of the crates and of `CORE_VERSION`, which `npm pack` checks.

## Draw a figure

`plot.mjs` loads `plot.wasm` and renders a `figure/1` spec to SVG, gives its geometry and picks the data point under a pointer; every call is synchronous. A figure a pointer moves over is prepared once and then answers without parsing it again:

```js
import { loadPlot } from "sbmlsim-core/plot";

const plot = await loadPlot(new URL("./plot.wasm", import.meta.url));
const svg = plot.render(spec);
const figure = plot.prepare(spec);
canvas.onpointermove = (e) => show(figure.pick(e.offsetX, e.offsetY));
figure.dispose();
```

A refused spec raises a `PlotError`. The contract, the classes of the SVG and the measured times are on the page [Figures](plot.md).

`figure.mjs` draws a spec into a container (replacing an SVG already there, e.g. the static figure of a report) and makes it interactive with the DOM alone, also from `file://`:

```js
import { attachFigure } from "sbmlsim-core/figure";

const figure = attachFigure(container, plot, spec, { readout: true, select: true, toggle: true });
figure.hidden();   // the hidden legend groups
figure.selected(); // the selected legend group or null
figure.update(next); // draws another spec, keeping the hidden and selected groups by their id
figure.dispose();  // the static figure again, without listeners
```

A legend entry is a toggle button (a click, Enter or Space; `aria-pressed` says whether its group is shown) which hides every element of its group, and Shift+Enter on it selects its group. A click on a data point or on the line of a curve selects its group and dims the others, a click on it again or Escape (with the focus or the pointer last in the figure) clears the selection, and a tooltip (`role="status"`) reads out the point under the pointer with the units of the axes, once per animation frame, never a point of a hidden group and the selected group first; it is announced (`aria-live="polite"`) when a click shows it and not while the pointer moves; `format(value, unit)` replaces the default text of a value, 4 significant digits and the unit. The state is the classes `sbmlsim-hidden`, `sbmlsim-selected` and `sbmlsim-dimmed` on the `[data-element]` groups of the SVG, styled by a stylesheet the module injects once per document (`#sbmlsim-figure-style`). The inline build has it as `sbmlsimCore.attachFigure`.

### Interactive figures

`hydrateFigures` (`hydrate.mjs`, since 0.28.0) is the page script of a static report: every `<figure data-figure="KEY">` holds the inline SVG of a figure and, as a direct child, `<script type="application/json" data-figure-spec="KEY">` with its `figure/1` spec (`</` written as `<\/`, which JSON reads back). The static SVG may be a direct child of the figure or deeper inside it (the direct child is preferred, else the first `<svg>` inside); that one is replaced. It attaches `attachFigure` to each of them:

```html
<figure data-figure="dose">
  <svg class="sbmlsim-figure">...</svg>
  <script type="application/json" data-figure-spec="dose">{"contract": "figure/1", ...}</script>
</figure>
```

```js
import { hydrateFigures } from "sbmlsim-core/hydrate";

const handles = await hydrateFigures(document, { wasm: bytes }); // or { plot }
```

`hydrateFigures(root = document, options = {})` returns a promise of the `FigureHandle`s of the figures which attached. `options.wasm` is the source of `plot.wasm` (anything `loadPlot` takes) and `options.plot` a `Plot` to reuse instead; a page without a figure loads nothing. A figure whose spec does not parse or render keeps its static SVG and gets one `console.warn` naming its key. An attached figure gets the attribute `data-hydrated`, which a second call skips, so calling it twice attaches once. The inline build has it as `sbmlsimCore.hydrateFigures`.

In Python `sbmlsim_core.web.figure_script()` is the script of a report, written once beside its pages (`<script src="figures.js">`): the inline build plus `plot.wasm` as base64 (decoded with `atob`), which calls `hydrateFigures` on `DOMContentLoaded` (or at once when the document is loaded) and warns once when that fails. It loads nothing from the network and works from `file://`.

`attachFigure(container, plot, spec, options = {})` returns a `FigureHandle` with these methods:

| method | what it does |
|---|---|
| `hidden()` | the hidden legend groups |
| `selected()` | the selected legend group or `null` |
| `update(spec)` | draws another spec, keeping the hidden and selected groups by their id; a refused spec raises a `PlotError` and leaves the figure as it was |
| `dispose()` | restores the static figure and removes the listeners |

Its options are `readout`, `select` and `toggle` (each true: the tooltip, the selection by a click and the legend toggles) and `format(value, unit)`. The classes, the attributes and the keyboard are the contract of [Figures](plot.md#semantic-classes-of-the-svg).

## A first simulation

```js
import { readFile } from "node:fs/promises";
import { createRuntime } from "sbmlsim-core";

const runtime = await createRuntime({
  engine: await readFile("js/runtime/engine.wasm"),
  mathlib: await readFile("js/runtime/mathlib.wasm"),
});
const instance = await runtime.instantiate(
  await readFile("model.wasm"),
  await readFile("model.json", "utf8"),
);

const { columns, nRows, values } = instance.simulate({
  contract: "simulation/1",
  start: 0,
  end: 10,
  output: { uniform: { steps: 100 } },
  selections: ["time", "[S]"],
});
console.log(columns, nRows, values); // values: a Float64Array, row major
```

`model.wasm` and `model.json` are what `sbmlsim-core compile` writes, or what the [Python](python.md) binding compiles; the request is the one every host takes, see [Rust](rust.md). The rest of this page is the API in detail.

## A live simulation in this page

The repressilator (Elowitz and Leibler 2000, [BIOMD0000000012](https://www.ebi.ac.uk/biomodels/BIOMD0000000012)) is three repressors in a ring, each one repressing the next. Whether the ring oscillates or settles depends on how cooperative the repression is, the Hill coefficient `n`. Move the slider: the SBML of the model is compiled in this page by `compiler.wasm`, instantiated and integrated here, with no server and nothing but the files of this site.

<div class="rp" data-base="../demo/">
<div class="rp-head">
<label class="rp-knob">Hill coefficient <b>n</b><input class="rp-range" type="range" min="1" max="4" step="0.05" value="2" aria-label="Hill coefficient n"><output class="rp-value">2.00</output></label>
<p class="rp-state">loading the compiler and the engine ...</p>
</div>
<svg class="rp-plot" viewBox="0 0 660 270" role="img" aria-label="The three repressilator proteins PX, PY and PZ over 200 time units. The same numbers are in the table below the figure."></svg>
<details class="rp-table"><summary>The simulated values</summary><div></div></details>
</div>

Below `n` of about 1.5 the ring settles to a steady state; above it the three proteins chase each other around the ring and the amplitude grows with `n`. The engine is the same `engine.wasm` the command line and the python wheel run, so these are the numbers of [the benchmarks](benchmarks.md), produced in the browser of the reader.

This is the whole of it, the loader and the two modules of the engine taken from the npm package:

```js
import { createRuntime } from "sbmlsim-core";

const fetchBytes = (name) => fetch(name).then((r) => r.arrayBuffer());
const runtime = await createRuntime({
  engine: await fetchBytes("engine.wasm"),
  mathlib: await fetchBytes("mathlib.wasm"),
});
const instance = await runtime.instantiate(
  await fetchBytes("model.wasm"),
  await fetch("model.json").then((r) => r.text()),
);

const simulate = (n) =>
  instance.simulate({
    contract: "simulation/1",
    start: 0,
    end: 200,
    output: { uniform: { steps: 400 } },
    preinit: [["n", n]],          // the one value the slider sets
    changes: [],
    steady_state: null,
    selections: ["time", "PX", "PY", "PZ"],
    settings: {
      relative_tolerance: 1e-8,
      absolute_tolerance: { scalar: 1e-10 },
      maximum_num_steps: 100000,
      maximum_time_step: 0,
      initial_time_step: 0,
      stiff: true,
      linear_solver: "dense",
      jacobian: "auto",
    },
    seed: 0,
  });

const { columns, nRows, values } = simulate(2.0); // values: a Float64Array, row major
```

`instance.simulate` is synchronous and takes one to two milliseconds here, which is why the slider can redraw on every `input` event without a worker. That is longer than the 0.2 ms the [benchmarks](benchmarks.md) report for this model, because this simulation runs twice as far, asks for four times the output rows and integrates at a hundred times the accuracy; the readout above is what the browser measures, coarsened to a tenth of a millisecond as browsers coarsen their clock. A model whose simulations are long enough to be felt belongs in `js/pool.mjs` instead, see [the worker pool](#the-worker-pool); the module this page runs is what `compiler.wasm` compiled from the SBML next to it, byte for byte what `sbmlsim-core compile` writes, and `docs/demo/` holds the model with the vendored compiler and runtime (`scripts/demo_assets.py`).

<style>
.rp { margin: 1rem 0 1.4rem; --rp-a: #2a78d6; --rp-b: #eb6834; --rp-c: #1baf7a;
  --rp-ink: var(--md-default-fg-color, #1a1a19); --rp-soft: var(--md-default-fg-color--light, #6b6b68);
  --rp-line: var(--md-default-fg-color--lighter, #d6d6d3); }
[data-md-color-scheme="slate"] .rp { --rp-a: #3987e5; --rp-b: #d95926; --rp-c: #199e70; }
.rp-head { display: flex; flex-wrap: wrap; align-items: center; gap: .6rem 1.4rem; margin-bottom: .5rem; }
.rp-knob { display: flex; align-items: center; gap: .6rem; font-size: .82rem; }
.rp-range { width: 15rem; max-width: 46vw; accent-color: var(--rp-a); }
.rp-value { font-variant-numeric: tabular-nums; font-weight: 600; min-width: 2.6rem; }
.rp-state { margin: 0; font-size: .76rem; color: var(--rp-soft); font-variant-numeric: tabular-nums; }
.rp-plot { width: 100%; height: auto; }
.rp-plot text { fill: var(--rp-soft); font-size: 11px;
  font-family: var(--md-text-font-family, system-ui), sans-serif; }
.rp-plot .rp-name { font-size: 12px; font-weight: 600; }
.rp-plot .rp-axis { stroke: var(--rp-line); stroke-width: 1; }
.rp-plot .rp-curve { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
.rp-table { margin-top: .4rem; font-size: .76rem; }
.rp-table summary { cursor: pointer; color: var(--rp-soft); }
.rp-table table { font-variant-numeric: tabular-nums; }
.rp-failed { color: var(--rp-ink); }
</style>
<script type="module">
const root = document.querySelector(".rp");
if (root) {
  const base = root.dataset.base;
  const plot = root.querySelector(".rp-plot");
  const state = root.querySelector(".rp-state");
  const range = root.querySelector(".rp-range");
  const value = root.querySelector(".rp-value");
  const table = root.querySelector(".rp-table div");
  const SERIES = [
    { id: "PX", color: "--rp-a" },
    { id: "PY", color: "--rp-b" },
    { id: "PZ", color: "--rp-c" },
  ];
  const SVG = "http://www.w3.org/2000/svg";
  const W = 660, H = 270, L = 52, R = 42, T = 14, B = 34;
  const node = (name, attrs, text) => {
    const e = document.createElementNS(SVG, name);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    if (text !== undefined) e.textContent = text;
    return e;
  };
  // a round number at or above the largest value, so the axis reads cleanly
  const niceMax = (v) => {
    if (!(v > 0)) return 1;
    const step = 10 ** Math.floor(Math.log10(v));
    return Math.ceil(v / step) * step;
  };
  const draw = (columns, nRows, values, end) => {
    const index = Object.fromEntries(columns.map((c, i) => [c, i]));
    const at = (row, id) => values[row * columns.length + index[id]];
    let peak = 0;
    for (let r = 0; r < nRows; r += 1)
      for (const s of SERIES) peak = Math.max(peak, at(r, s.id));
    const top = niceMax(peak);
    const x = (t) => L + (t / end) * (W - L - R);
    const y = (v) => H - B - (v / top) * (H - T - B);
    plot.replaceChildren();
    plot.append(node("line", { class: "rp-axis", x1: L, y1: H - B, x2: W - R, y2: H - B }));
    plot.append(node("line", { class: "rp-axis", x1: L, y1: T, x2: L, y2: H - B }));
    for (const t of [0, 50, 100, 150, 200]) {
      plot.append(node("text", { x: x(t), y: H - B + 16, "text-anchor": "middle" }, String(t)));
    }
    plot.append(node("text", { x: (L + W - R) / 2, y: H - 4, "text-anchor": "middle" }, "time"));
    for (const v of [0, top / 2, top]) {
      plot.append(node("text", { x: L - 8, y: y(v) + 4, "text-anchor": "end" },
        v >= 1000 ? (v / 1000).toFixed(v % 1000 ? 1 : 0) + "k" : String(Math.round(v))));
      if (v > 0) plot.append(node("line", { class: "rp-axis", x1: L, y1: y(v), x2: W - R, y2: y(v), "stroke-dasharray": "3 4" }));
    }
    const labels = [];
    for (const s of SERIES) {
      const points = [];
      for (let r = 0; r < nRows; r += 1) points.push(x(at(r, "time")) + "," + y(at(r, s.id)));
      const stroke = getComputedStyle(root).getPropertyValue(s.color).trim();
      plot.append(node("polyline", { class: "rp-curve", points: points.join(" "), stroke }));
      labels.push({ id: s.id, stroke, y: y(at(nRows - 1, s.id)) });
    }
    // the name sits at the end of its own curve, so the lines are not read by colour
    // alone; two curves which end together have their names pushed apart instead
    labels.sort((a, b) => a.y - b.y);
    for (let i = 1; i < labels.length; i += 1) {
      labels[i].y = Math.max(labels[i].y, labels[i - 1].y + 13);
    }
    const overflow = labels[labels.length - 1].y - (H - B);
    if (overflow > 0) for (const label of labels) label.y -= overflow;
    for (const label of labels) {
      plot.append(node("text", { class: "rp-name", x: W - R + 6, y: label.y + 4, fill: label.stroke }, label.id));
    }
    const rows = [];
    for (let r = 0; r < nRows; r += 20) {
      rows.push("<tr><td>" + at(r, "time").toFixed(0) + "</td>" +
        SERIES.map((s) => "<td>" + at(r, s.id).toFixed(1) + "</td>").join("") + "</tr>");
    }
    table.innerHTML = "<table><thead><tr><th>time</th>" +
      SERIES.map((s) => "<th>" + s.id + "</th>").join("") +
      "</tr></thead><tbody>" + rows.join("") + "</tbody></table>";
  };
  const request = (n) => ({
    contract: "simulation/1",
    start: 0,
    end: 200,
    output: { uniform: { steps: 400 } },
    preinit: [["n", n]],
    changes: [],
    steady_state: null,
    selections: ["time", "PX", "PY", "PZ"],
    settings: {
      relative_tolerance: 1e-8,
      absolute_tolerance: { scalar: 1e-10 },
      maximum_num_steps: 100000,
      maximum_time_step: 0,
      initial_time_step: 0,
      stiff: true,
      linear_solver: "dense",
      jacobian: "auto",
    },
    seed: 0,
  });
  try {
    const { createCompiler } = await import(base + "compile.mjs");
    const { createRuntime } = await import(base + "loader.mjs");
    const bytes = (name) => fetch(base + name).then((r) => r.arrayBuffer());
    const compiler = await createCompiler({
      compiler: await bytes("compiler.wasm"),
    });
    const runtime = await createRuntime({
      engine: await bytes("engine.wasm"),
      mathlib: await bytes("mathlib.wasm"),
    });
    const sbml = await fetch(base + "repressilator.xml").then((r) => r.text());
    const { wasm, description } = compiler.compile(sbml);
    const instance = await runtime.instantiate(wasm, description);
    const run = () => {
      const n = Number(range.value);
      value.textContent = n.toFixed(2);
      const started = performance.now();
      const out = instance.simulate(request(n));
      const ms = performance.now() - started;
      draw(out.columns, out.nRows, out.values, 200);
      state.textContent = out.nRows + " output rows simulated in this page in "
        + ms.toFixed(2) + " ms";
    };
    range.addEventListener("input", run);
    run();
  } catch (error) {
    state.classList.add("rp-failed");
    state.textContent = "The demo could not run in this browser: " + error.message;
  }
}
</script>

## The loader

```js
import { createRuntime } from "./js/loader.mjs";

// engine and mathlib: WebAssembly.Modules or their bytes
const runtime = await createRuntime({ engine, mathlib });
// modelWasm: a Module or bytes; modelJson: the text of model.json or its object
const instance = await runtime.instantiate(modelWasm, modelJson);
// values: a Float64Array, row major
const { columns, nRows, values } = instance.simulate(request);
```

`createRuntime({ engine, mathlib, console?, maxMemoryPages? })` compiles (or takes) `engine.wasm` and `mathlib.wasm` and checks their imports, exports and the custom section `sbmlsim.engine` of `engine.wasm` (the minimum of its memory and the version of its output). `runtime.modules` are the compiled modules, which a worker gets to create a runtime of its own.

`runtime.instantiate(modelWasm, description)` instantiates a memory, mathlib, the model and the engine, as the wasmtime host links them, and loads the description into the engine. It is asynchronous: bytes compile with `WebAssembly.compile`, which browsers require for modules above a few KB on the main thread.

An instance is one model in one engine, for one thread:

| method | what it gives |
|---|---|
| `instance.simulate(request)` | a request of `simulation/1` (an object or its JSON text); `{ columns, nRows, values }`, the values a `Float64Array` whose bits are the ones of wasmtime, NaN payloads too |
| `instance.stats()` | the statistics of the integrator of the last simulation, by the names of `STATS_NAMES`: the first `STATS_COUNTS` are counts, the ones after them the times and steps of the state of the solver |
| `instance.wasiCalls()` | the calls of the engine into WASI; the engine calls none while it loads or simulates |
| `instance.memoryBytes()` | the size of the memory of the instance |

An error of the engine is an `EngineError` with `kind` (`load`, `request`, `unsupported`, `integration` with `flag` and `time`, or `internal`); an instance which trapped (`internal`) is not used again. The engine gets WASI preview1 stubs: `fd_write` writes to the console, `proc_exit` throws a `WasiExit`, the environment is empty and the clock and the random bytes throw. The module also exports `toCsv` and `formatNumber` (the CSV of the command line), `MODEL_CONTRACT`, `ENGINE_OUTPUT_FORMAT`, `MAX_MEMORY_PAGES` (2 GiB) and `STATS_NAMES`.

In Node, `js/simulate.mjs` writes the CSV of `sbmlsim-core simulate` byte for byte, with `engine.wasm` and `mathlib.wasm` of `$SBMLSIM_CORE_RUNTIME`, else of `js/runtime`:

```bash
node js/simulate.mjs crates/host/tests/decay-model crates/host/tests/decay-model/request.json [-o out.csv]
```

Byte inputs to `runtime.instantiate` and `pool.addModel` are validated before compilation. Models may import only the engine memory and supported math functions; defined memories, mutable globals, tables, element/data segments and start functions are rejected. Immutable scalar constant globals are accepted; extended constant initializers are conservatively rejected. This keeps reusable model instances free of hidden persistent state.

A precompiled `WebAssembly.Module` is a **trusted input**: JavaScript reflection exposes imports and exports but cannot inspect these state sections. Supply bytes at the validation boundary, or only pass modules compiled from previously validated bytes. Neither metadata nor these structural checks prove that arbitrary model instructions respect borrowed buffers in the shared engine memory. Use compiler-produced models, and run untrusted execution in workers with resource limits. Direct main-thread loader calls are synchronous and cannot be interrupted by an `AbortSignal`.

## The compiler

```js
import { createCompiler } from "./js/compile.mjs";

// compiler: a WebAssembly.Module or the bytes of compiler.wasm
const compiler = await createCompiler({ compiler });
const { wasm, description } = compiler.compile(sbmlText);
const instance = await runtime.instantiate(wasm, description);
```

`compiler.wasm` is the compiler of the command line built for `wasm32-unknown-unknown` (`crates/wasm`, the C ABI of `allocate`, `compile` and `deallocate` over the memory of the module): the same reader of SBML, the same model representation and the same code generation, so a model compiled in a page gives the `model.wasm` and the `model.json` of `sbmlsim-core compile`, byte for byte, which `js/test/compile.test.mjs` asserts. The module reads no file and imports nothing of its host, so a page compiles a document a visitor drops into it without a server; `sbmlsim-core runtime -o DIR` writes it next to `engine.wasm` and `mathlib.wasm`.

`compiler.compile(sbml, { optimize = true, jacobian = "auto", parameterJacobian = true })` gives `{ wasm, description }`, the bytes of the module and the description as an object, which `runtime.instantiate` takes as they are. The call is synchronous and takes a few milliseconds for a model of the size of the repressilator, so a page compiles on the main thread; a model of many reactions belongs in a worker next to the simulations. `optimize: false` leaves the optimizer out (the results are the same, the module is larger), `jacobian` is `"auto"`, `"difference-quotient"` or `"analytic"`, and a document the compiler refuses raises a `CompileError` with the message of the compiler. `parameterJacobian: false` omits analytic parameter derivatives and their cached constants for ordinary simulations (0.26.0). Forward sensitivities then use numerical parameter derivatives. This option requires a matching `compiler.wasm` with the `compile_with_options` export. One compiler compiles any number of models, and `compiler.module` is the compiled `WebAssembly.Module`, which a worker gets to create a compiler of its own.

## The worker pool

A simulation is one synchronous call of the engine. `pool.mjs` runs them in a pool of workers, each with a runtime of its own, so a page stays responsive and the points of a scan run in parallel:

<svg class="core-diagram" viewBox="0 10 840 300" xmlns="http://www.w3.org/2000/svg" role="img"
     aria-label="The page calls pool.simulate, which queues requests and hands each to a free worker. Every worker is a module Web Worker with a runtime of its own, holding an instance of the engine, mathlib and the model it was given, so a simulation is one synchronous call inside the worker and the page stays responsive. The results come back to the page as Promises. The pool has as many workers as defaultWorkers gives: the logical processors but one, between 1 and 8.">
  <defs>
    <marker id="pl-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="currentColor"/>
    </marker>
  </defs>
  <text class="cd-title" x="92" y="30">the page</text>
  <g class="cd-step cd-active">
    <rect x="16" y="40" width="152" height="60" rx="6"/>
    <text x="92" y="64">pool.simulate</text>
    <text class="cd-sub" x="92" y="82">a Promise per point</text>
  </g>
  <line x1="92" y1="106" x2="92" y2="140" marker-end="url(#pl-arrow)"/>
  <g class="cd-step">
    <rect x="16" y="146" width="152" height="52" rx="6"/>
    <text x="92" y="167">the queue</text>
    <text class="cd-sub" x="92" y="185">one point per worker</text>
  </g>
  <line x1="174" y1="172" x2="236" y2="118" marker-end="url(#pl-arrow)"/>
  <line x1="174" y1="172" x2="236" y2="172" marker-end="url(#pl-arrow)"/>
  <line x1="174" y1="172" x2="236" y2="252" marker-end="url(#pl-arrow)"/>
  <text class="cd-title" x="536" y="30">a module Web Worker each, with a runtime of its own</text>
  <g class="cd-instance">
    <rect x="242" y="78" width="420" height="56" rx="6"/>
  </g>
  <text class="cd-sub" x="302" y="112">worker 1</text>
  <g class="cd-module">
    <rect x="360" y="90" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="406" y="110">engine</text>
    <rect x="460" y="90" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="506" y="110">mathlib</text>
    <rect x="560" y="90" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="606" y="110">model</text>
  </g>
  <g class="cd-instance">
    <rect x="242" y="144" width="420" height="56" rx="6"/>
  </g>
  <text class="cd-sub" x="302" y="178">worker 2</text>
  <g class="cd-module">
    <rect x="360" y="156" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="406" y="176">engine</text>
    <rect x="460" y="156" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="506" y="176">mathlib</text>
    <rect x="560" y="156" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="606" y="176">model</text>
  </g>
  <text class="cd-sub" x="452" y="222">...</text>
  <g class="cd-instance">
    <rect x="242" y="236" width="420" height="56" rx="6"/>
  </g>
  <text class="cd-sub" x="302" y="270">worker n</text>
  <g class="cd-module">
    <rect x="360" y="248" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="406" y="268">engine</text>
    <rect x="460" y="248" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="506" y="268">mathlib</text>
    <rect x="560" y="248" width="92" height="32" rx="4"/>
    <text class="cd-sub" x="606" y="268">model</text>
  </g>
  <line x1="668" y1="106" x2="712" y2="106" marker-end="url(#pl-arrow)"/>
  <line x1="668" y1="172" x2="712" y2="172" marker-end="url(#pl-arrow)"/>
  <line x1="668" y1="264" x2="712" y2="264" marker-end="url(#pl-arrow)"/>
  <g class="cd-artifact">
    <rect x="718" y="88" width="106" height="36" rx="4"/>
    <text class="cd-sub" x="771" y="110">rows</text>
    <rect x="718" y="154" width="106" height="36" rx="4"/>
    <text class="cd-sub" x="771" y="176">rows</text>
    <rect x="718" y="246" width="106" height="36" rx="4"/>
    <text class="cd-sub" x="771" y="268">rows</text>
  </g>
  <text class="cd-note" x="771" y="218">back to the page</text>
  <text class="cd-label" x="16" y="226">n is defaultWorkers():</text>
  <text class="cd-label" x="16" y="244">the logical processors</text>
  <text class="cd-label" x="16" y="262">but one, 1 to 8</text>
</svg>

```js
import { createPool } from "./js/pool.mjs";

const pool = await createPool({ engine, mathlib }); // WebAssembly.Modules or bytes
await pool.addModel("decay", modelWasm, modelJson);
const { columns, nRows, values } = await pool.simulate("decay", request, { signal });
pool.close();
```

| call | what it does |
|---|---|
| `createPool(options)` | a Promise of the pool; the options are `engine` and `mathlib` (`WebAssembly.Module`s or bytes), `workers` is the most workers (`defaultWorkers()`: the logical processors but one, 1 to 8), `createWorker(url)` creates one (a module Web Worker without), `workerUrl` its script (`WORKER_URL`, `worker.mjs` next to `pool.mjs`) |
| `pool.addModel(key, model, description)` | adds a model under a key: a `WebAssembly.Module` or the bytes of `model.wasm`, the text of `model.json` or its object; a Promise |
| `pool.removeModel(key)` | removes a registered model and its retained worker instances; returns whether it existed; refuses while its requests are queued or running |
| `pool.hasModel(key)` | whether the pool has a model under a key |
| `pool.simulate(key, request, options?)` | a Promise of `{ columns, nRows, values }`, as `instance.simulate`; `options.signal` drops the request; `options.deadlineMs` and `options.hardAbort` override pool defaults |
| `pool.close()` | terminates the workers and rejects every queued and running request |
| `pool.size`, `pool.modules` | the most workers, the compiled runtime |
| `pool.maxMemoryPages`, `pool.maxTotalMemoryBytes` | effective per-instance and aggregate retained linear-memory policies |

`engine.wasm`, `mathlib.wasm` and every model are compiled once, on the thread which creates the pool, and posted to the workers as `WebAssembly.Module`s, which a structured clone shares instead of copying. The workers are created when the queue needs them; a model is posted to a worker before the first request of it the worker runs, until evicted from that worker. The requests run in the order they were made, one at a time per worker:

- A request whose `signal` aborts is rejected with its exact reason, including `null`, `false`, `0` and an empty string. A queued request never runs. A running request finishes and discards its result unless `hardAbort: true` terminates and replaces the worker. Its execution deadline remains active after a soft abort.
- An error of the engine (`EngineError`) rejects only its request; a worker which crashes rejects the request it ran and is replaced.
- The values are a `Float64Array` whose buffer the worker transfers. The results are bitwise the ones of the main thread (`js/test/pool.test.mjs`).

Pool limits are configurable in `createPool`:

| Option | Default | Policy |
| --- | --- | --- |
| `deadlineMs` | `30000` | Wall-clock deadline from dispatch, covering worker startup, model instantiation and simulation. A timeout rejects with `TimeoutError`, terminates the worker, and lets queued work use a replacement. `0` disables it; a job can override it. Queue waiting and main-thread compilation are excluded. |
| `hardAbort` | `false` | Terminate and replace a running worker when its signal aborts. |
| `maxWorkers` | `32` | Reject a larger `workers` option. |
| `maxQueuedJobs` | `1024` | Reject submissions once this many jobs are waiting; running jobs are additional. |
| `maxQueuedBytes` | 64 MiB | Maximum sum of UTF-8 request bytes waiting in the queue; running jobs are additional and retain the per-request cap. |
| `maxModelBytes` | 128 MiB | Maximum registered model input bytes plus UTF-8 descriptions. Trusted precompiled modules charge their descriptions only, since code size is unavailable. |
| `maxModels` | `128` | Reject additional registered models; use `removeModel` to free a slot. |
| `maxWorkerModels` | `8` | Evict the least recently used retained model/instance on each worker before loading another; registered compiled modules remain reusable. |
| `maxTotalMemoryBytes` | `2147483648` (2 GiB) | Aggregate retained shared engine linear-memory capacity across `workers * min(maxWorkerModels, maxModels)` instances. |
| `maxMemoryPages` | derived from aggregate budget | Per-instance linear-memory maximum in 64 KiB pages, forwarded to every worker. Explicit values exceeding the aggregate budget are rejected. The engine initial memory must fit. |
| `maxModuleBytes` | 32 MiB | Maximum byte size of each model/runtime module before compilation. Precompiled trusted modules cannot be measured by reflection. |
| `maxDescriptionBytes` | 16 MiB | Maximum UTF-8 JSON bytes per registered description. |
| `maxRequestBytes` | 16 MiB | Maximum UTF-8 JSON bytes per request before queueing. |

The default aggregate policy gives each instance 32 MiB with eight workers and eight retained models each. To allow larger instances, lower the worker/retention counts or explicitly raise `maxTotalMemoryBytes`. This budget bounds retained shared engine linear-memory capacity only; it excludes mathlib's private fixed memory, JavaScript heap, JIT code, returned output buffers, and memory awaiting garbage collection after eviction. It is not a process RSS limit. Request objects are converted to JSON once at submission, preserving that submission snapshot in the queue; request strings are checked directly. JSON serialization can temporarily allocate before its byte limit is checked. Choose smaller limits for large models. A worker's `postMessage` failure or Node exit rejects its affected request and releases the slot. Termination relies on the host worker implementation.

In Node a worker is a worker thread: `worker.mjs` gets its port from `node:worker_threads` (a dynamic import which never runs in a browser), and the pool takes it through `createWorker`:

```js
import { Worker } from "node:worker_threads";
const pool = await createPool({
  engine,
  mathlib,
  createWorker: (url) => new Worker(url),
});
```

## The panel endpoint

[sbmlsim](https://matthiaskoenig.github.io/sbmlsim/) declares the interactive panels of a model (knobs, curves, the study data under them) and serves them with `python -m sbmlsim.serve`; the [sbmlsim-web](https://matthiaskoenig.github.io/sbmlsim-web/) components `<sbmlsim-panel>` and `<sbmlsim-app>` draw them. `panels.mjs` answers the same routes inside a page, from a bundle the service exports, with the simulations on a pool of workers, so an app of panels is a static page without a server.

```js
import { createPanelEndpoint } from "./sbmlsim-core/panels.mjs";

const endpoint = createPanelEndpoint({ bundle: "./bundles/glimepiride/bundle.json" });
// a Response, as fetch gives it
const response = await endpoint("local:glimepiride/api/models/glimepiride/app", {});
```

`createPanelEndpoint(options)` gives a function `(url, init) => Promise<Response>` with the signature of `fetch`. It loads lazily: `bundle.json` and the runtime at the first request, a model at the first simulation which needs it. Options:

| option | what it is |
|---|---|
| `bundle` | the URL of `bundle.json` (relative to the page, or a `URL`) |
| `runtime` | `{ engine, mathlib }`, the URLs of `engine.wasm` and `mathlib.wasm` (or their `WebAssembly.Module`s or bytes); `./runtime/engine.wasm` and `./runtime/mathlib.wasm` next to `panels.mjs` without |
| `workers` | the most workers of the pool, `defaultWorkers()` without |
| `createWorker` | creates a worker of the pool, see the pool |
| `pool` | a pool of `createPool` to run on instead of one of its own, e.g. shared by the endpoints of several bundles |
| `fetch` | loads the files of the bundle and the runtime, `globalThis.fetch` without |
| `maxRunBytes` | the cache byte budget, including numeric buffers and estimated key/metadata overhead, `MAX_RUN_BYTES` (64 MiB) without |
| `maxRunEntries` | maximum completed cached runs, `128` without |
| `maxInflightRuns` | maximum distinct concurrent runs, `32` without; requests of an existing run still join it |

`endpoint.close()` terminates the workers of its own pool (not of a pool it was given). The module also exports `CORE_VERSION` (the version of `js/package.json`), `ENGINE` (`"sbmlsim-core <version>"`, the engine a provenance names), `BUNDLE_CONTRACT`, `PANEL_CONTRACT`, `MAX_RUN_BYTES`, `MAX_ID`, `knobValues(knob, n)`, `canonicalJson(value)` and `sha256Hex(text)`.

### The routes

The path is the part of the URL from `/api/` on, so `local:glimepiride/api/...` and `https://example.org/any/api/...` are answered alike. The model id is the one of the bundle; the query `variant` is a key of `answers` of the bundle (none is the key `""`).

The routes hang under `/api/models/{model}`:

| route | answer |
|---|---|
| `GET /app[?variant=V]` | the app (`app/1`), a 404 for a model without one |
| `GET /panels/{panel}[?variant=V]` | the declaration of the panel (`panel/1`) |
| `GET /panels/{panel}/data[?variant=V]` | the study data of the panel (`paneldata/1`) |
| `POST /panels/{panel}/simulate` | a simulation, body `{ values?, grid?, variant?, draws? }`, answer `panel/1` |

A simulation is checked as the service checks it: a knob the panel does not declare, a value outside the bounds of a range knob or none of the choices of a choice knob, a grid of fewer than 1 or more points than the `limits` of the declaration, and a grid over a choice knob of other points than its choices are refused with a 422. The knobs of the grid span the product of their values in C order (the first knob of the declaration slowest): a range knob of `n` points runs `min`, `(min * (n - 1 - i) + max * i) / (n - 1)` for `i` from 1 to `n - 2`, and `max` (`n` 1 runs `min`), a choice knob its choices. Every other knob takes its value, or its default.

A point is one request of `simulation/1`: the start, the end and `{"uniform": {"steps": points}}` of the run of the bundle, the pre-initialization of the run, of the target of every knob (its value times its factor, in the units of the model) and of its parameter (its value), sorted by id, the selections of the model and its settings with an absolute tolerance per state. The answer:

| field | what it holds |
|---|---|
| `contract` | `"panel/1"` |
| `spec_hash` | the sha256 of the canonical JSON (sorted keys, no whitespace, every character which is not ASCII escaped) of the resolved request: the model, the panel, the query `variant`, the declaration, the sha256 of the SBML, the value of every knob (its default for one the request leaves out), the grid, the draws, the engine and the settings |
| `x` | the times of the bundle, in the unit `units.x` |
| `dims` | one axis per knob of the grid: `{ id, label, unit, values }` |
| `curves` | per curve a row per point of the grid, the values of its selection times its factor; a value which is not finite is `null` |
| `units` | the units of the run of the bundle |
| `status` | `{ ok, points, failed }`: a point which does not integrate is a row of `null` in every curve and its flat index is in `failed` |
| `uncertainty` | the band of the draws, for a request which asked for them; absent without |
| `provenance` | `{ model, panel, model_sha256, variant, engine, sbmlsim, seconds, created }`: the variant the bundle simulated, `sbmlsim-core <version>`, the version of sbmlsim which exported the bundle |

A panel whose run carries an `uncertainty` - the sample of the fit of its model, which the bundle holds - answers a band for the `draws` a request asks for. Every point of the grid is run once per draw, with the parameters of that draw set before the initialization, exactly as a knob with a `parameter` is set, and the answer carries `uncertainty`: the `draws` it was run with, the `fit` and the `level` of the recipe, `band` (per curve one `{ low, high }` per point of the grid, the percentiles of the sample of that point time by time, the linear interpolation of the order statistics as numpy and the service compute them), `curves` (per curve the row of every draw, only for a run of one point, where their size is bounded) and `failed` (the flat index of every draw which did not integrate, which is no member of the percentiles). The draws are the first ones of the sample, which the export writes in the order of a latin hypercube: a prefix of such a sample is a sample of its own, so the band settles as a page asks for more draws instead of jumping. A request which names no draws runs none, and one which asks for more than the sample holds is refused with a 422 rather than cut, so a page never states a band it did not get.

The runs are shared: the points of one model, end and pre-initialization are one run, so the panels of a page which show one state, each with its own curves, cost one scan. A run in flight is joined, a successful finished one is kept in a cache bounded by bytes and entry count, the least recently used evicted first. Runs with any failed point are not cached, so transient failures can recover and failed runs cannot accumulate at zero accounted bytes. A request whose `signal` aborts is rejected with its reason, as `fetch` is; a run is dropped when every request which waits for it aborted.

An error is `{"detail": "..."}` with the status of the service: 404 (a route, a model or a panel which does not exist, a model without an app), 405 (a route of another method, with `Allow`), 422 (a request which is refused) and 500 (a bundle or a model which does not load, a run which fails otherwise than by an integration). `spec_hash` identifies what this endpoint simulated; it is no hash of the service, whose engine and settings differ.

### The bundle, `panelbundle/1`

`python -m sbmlsim.serve --model id=module:attribute --export DIR` writes the bundle of every model it is given into `DIR/<model id>/`:

```text
DIR/<model id>/bundle.json
DIR/<model id>/models/<model key>/model.xml     the SBML the export writes
DIR/<model id>/models/<model key>/model.wasm    sbmlsim-core compile
DIR/<model id>/models/<model key>/model.json    sbmlsim-core compile
```

`bundle.json` holds what the service answers, so the endpoint resolves nothing itself:

- `contract` (`"panelbundle/1"`), `model` (the model id of the routes), `title`, `sbmlsim` (the version which exported it) and `created`.
- `answers`: per query `variant` (`""` and every variant the app is served in), the `app` (or `null`) and per panel its `declaration`, its `data` and its `run`.
- A `run`: the `variant` and the `model` (a key of `models`) it simulates, the `model_sha256` of its SBML, `start`, `end`, `points` (intervals, `points + 1` rows), the delivered `x`, the `units`, the `curves` (per curve id the `selection` and the `factor` from the unit of the model to the delivered one), the `preinit` (pairs in the units of the model) and the `knobs` (per knob id its `target` and `factor`, its `parameter` and its `unit`).
- The `uncertainty` of a `run`, for a model whose parameters carry one: the `fit` it comes from, the `level` of the band, the `seed` the sample was drawn with, the `default_draws` a client asks for and the `max_draws` it may ask for, the `parameters` (ids of the model) and the `draws`, one vector per draw in the order of the parameters and in the units of the model.
- `models`: per model key the `sbml` and the `compiled` directory (relative to `bundle.json`), its `variant`, the `selections` every panel run on it reads, the `settings` of `simulation/1` and the `absolute_tolerances` per state id, which the endpoint turns into a `per_state` tolerance in the order of the `states` of `model.json` (the scalar one for a state without one).

Nothing of the semantics of a simulation lives in JavaScript. The formulas of the derived targets of a panel are compiled into the model by the export: one parameter `panel_knob__<knob id>` per knob a formula reads and one initial assignment per derived target, so a model key is the variant, or the variant and a hash of its derived targets. The unit conversions are factors, and the times of the output are computed by the export.

### A static panel app

1. Export the bundle with sbmlsim:

    ```bash
    python -m sbmlsim.serve --model glimepiride=pkdb_models.models.glimepiride:MODEL --export site/bundles
    ```

2. Compile every model of the bundle into its directory (prepared first, see [Compiling a model](compiling.md#preparing-a-model)):

    ```bash
    for dir in site/bundles/glimepiride/models/*/; do
      uv run scripts/prepare_model.py "$dir/model.xml" -o "$dir/prepared.xml"
      sbmlsim-core compile "$dir/prepared.xml" -o "$dir"
    done
    ```

3. Copy the JavaScript host and the runtime next to the page, the runtime in `runtime/` next to `panels.mjs`:

    ```bash
    mkdir -p site/sbmlsim-core
    cp js/loader.mjs js/pool.mjs js/worker.mjs js/panels.mjs site/sbmlsim-core/
    sbmlsim-core runtime -o site/sbmlsim-core/runtime
    ```

4. Register the endpoint with sbmlsim-web and point the elements at it. An element whose `endpoint` starts with `local:` is answered by the function registered under that name; it waits for the registration, so the module script may run after the elements connected. `sbmlsim-web.iife.js` is the IIFE build of sbmlsim-web (its documentation has the URL of a pinned release), which defines the global `sbmlsimWeb`:

    ```html
    <script src="sbmlsim-web.iife.js"></script>
    <script type="module">
      import { createPanelEndpoint } from "./sbmlsim-core/panels.mjs";
      sbmlsimWeb.registerEndpoint("local:glimepiride", createPanelEndpoint({ bundle: "./bundles/glimepiride/bundle.json" }));
    </script>
    <sbmlsim-app endpoint="local:glimepiride" model="glimepiride"></sbmlsim-app>
    ```

5. Serve the directory from any static server, e.g. GitHub Pages. The workers are module workers, so the page is served over HTTP(S), not opened as a file. `sha256Hex` takes `crypto.subtle`, which a browser has only in a secure context (HTTPS or localhost), and a digest in plain JavaScript elsewhere.

The components, their attributes and `registerEndpoint` are documented with [sbmlsim-web](https://matthiaskoenig.github.io/sbmlsim-web/), which has the glimepiride app as an example.

### A panel app of one file

The inline build (`js/inline.mjs`, built with esbuild by `node js/build.mjs` into `python/sbmlsim_core/web/sbmlsim-core.js`, committed and checked by the tests) is the host as one classic script with the global `sbmlsimCore`: `createInlineEndpoint({ bundle, files, engine, mathlib, workers })` answers the routes of a bundle held in memory (the bundle as an object, its compiled models as bytes by their path in the bundle, the runtime as bytes), and its pool starts its workers as classic workers from a `blob:` URL of the worker the script embeds (`WORKER_SOURCE`). A page which embeds the script, the runtime, the bundle and its models needs neither a server nor the network and opens from `file://` too. The python package ships the build with the runtime (see [Python](python.md#a-page-of-one-file)); sbmlsim writes the app of a model as such a page.

## Prepared simulations

Prepare a fixed request once and supply finite numeric values for each run. `varying` gives the pre-initialization entity IDs in vector order, including concentration IDs such as `[A]`. Existing template entries retain their quantity and position; changes, selections, settings and steady-state configuration stay in the prepared request.

```javascript
const plan = instance.prepare(request, { varying: ["k", "y"] });
try {
  const result = instance.simulatePrepared(plan, [0.5, 2]);
  // result.values is an owned Float64Array, as with simulate().
} finally {
  plan.close();
}
```

`plan.nValues` is the required vector length. Plans belong to their creating instance, cannot be cloned or transferred, and reject use after `close()`. Closing twice is safe. An instance retains at most 32 live plans. Request errors, including invalid vector lengths and nonfinite values, leave it usable. Traps poison the instance as with ordinary simulation. The loader reuses one aligned guest input buffer for JSON and numeric vectors.

The worker pool exposes the same operation asynchronously:

```javascript
const plan = await pool.prepare("model-key", request, { varying: ["k"] });
try {
  const result = await pool.simulatePrepared(plan, [0.5], { signal });
} finally {
  plan.close();
}
```

Workers prepare each plan lazily for each retained instance and subsequently receive only numeric vectors. Evicted worker instances recreate their plans when needed. Closing a plan releases worker handles and rejects its queued jobs; a running job may finish. Removing its registered model invalidates the plan, including when that key is later reused. Pool close invalidates every plan. Prepared definitions count toward the registered byte budget, and queued vectors use the existing queue, byte, cancellation and deadline limits.

Prepared requests require the optional `engine_prepare`, `engine_simulate_prepared` and `engine_release_prepared` exports. Older engines continue to support `simulate()`; `prepare()` raises an error named `UnsupportedError` with kind `unsupported`. The model and simulation contracts remain `model/2` and `simulation/1`.
