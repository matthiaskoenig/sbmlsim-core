# JavaScript

`js/` is the JavaScript host of sbmlsim-core: plain ES modules without dependencies and without APIs of Node, so they run in Node and in browsers alike.

| module | export of `js/package.json` | what it is |
|---|---|---|
| `loader.mjs` | `.` | runs `engine.wasm`, `mathlib.wasm` and a `model.wasm` with the bits of wasmtime |
| `compile.mjs` | `./compile` | compiles an SBML document into `model.wasm` and its description with `compiler.wasm` |
| `petab.mjs` (since 0.39.0) | `./petab` | a problem of [PEtab v2](petab.md) read, simulated, evaluated and (since 0.40.0) fitted in a page |
| `stats.mjs` (since 0.43.0) | `./stats` | the [statistics of the analyses](stats.md), every function of `sbmlsim_core.stats` with the bits of python, see [The statistics](#the-statistics) |
| `kernel.mjs` (since 0.41.0) | `./kernel` | instantiates the [batched kernel](#the-batched-kernel) of a model, which advances many instances of it per call |
| `pool.mjs` (with `worker.mjs`) | `./pool` | a pool of workers which run the loader off the main thread |
| `panels.mjs` | `./panels` | the routes of the panel service of sbmlsim, answered inside a page from a bundle |
| `plot.mjs` | `./plot` | draws the figures of sbmlsim with `plot.wasm`, see [Figures](plot.md) |
| `figure.mjs` (since 0.28.0) | `./figure` | makes a drawn figure interactive in a page: legend toggles, selection and the read-out of a point |
| `simulate.mjs` | the command `sbmlsim-core-simulate` | the CSV of `sbmlsim-core simulate` in Node |

The package is published to npm as [`sbmlsim-core`](https://www.npmjs.com/package/sbmlsim-core), with the `compiler.wasm`, `engine.wasm`, `mathlib.wasm` and `plot.wasm` (the renderer of the figures of sbmlsim, `crates/plot-wasm`) of its release in `runtime/`:

```bash
npm install sbmlsim-core
```

A page imports the modules from its own files or from a CDN (`https://cdn.jsdelivr.net/npm/sbmlsim-core@0.43.0/panels.mjs`); a pool of a module of another origin starts its workers through a module of the page's own origin, which browsers require of a worker. A checkout serves the files of `js/` with the runtime of a build (`sbmlsim-core runtime -o js/runtime`, see [Installation](installation.md#building)). A release publishes the package from the tag `vX.Y.Z` (`.github/workflows/publish.yml`, trusted publishing with provenance, together with the python package on PyPI); the version of `js/package.json` is the one of the crates and of `CORE_VERSION`, which `npm pack` checks.

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
<label class="rp-knob">Hill coefficient <b>n</b><input class="rp-range" name="hill-coefficient" type="range" min="1" max="4" step="0.05" value="2" aria-label="Hill coefficient n"><output class="rp-value">2.00</output></label>
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
.rp-knob { display: flex; align-items: center; gap: .6rem; font-size: .82rem; white-space: nowrap; }
.rp-range { width: 15rem; max-width: 40vw; accent-color: var(--rp-a); }
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

// compilerWasm: a WebAssembly.Module or the bytes of compiler.wasm
const compiler = await createCompiler({ compiler: compilerWasm });
const { wasm, description } = compiler.compile(sbmlText);
const instance = await runtime.instantiate(wasm, description);
```

`compiler.wasm` is the compiler of the command line built for `wasm32-unknown-unknown` (`crates/wasm`, the C ABI of `allocate`, `compile`, `compile_kernel` and `deallocate` over the memory of the module, the options of a compilation as JSON): the same reader of SBML, the same model representation and the same code generation, so a model compiled in a page gives the `model.wasm` and the `model.json` of `sbmlsim-core compile`, byte for byte, which `js/test/compile.test.mjs` asserts. The module reads no file and imports nothing of its host, so a page compiles a document a visitor drops into it without a server; `sbmlsim-core runtime -o DIR` writes it next to `engine.wasm` and `mathlib.wasm`.

`compiler.compile(sbml, { optimize = true, jacobian = "auto", parameterJacobian = true, driven = [] })` gives `{ wasm, description }`, the bytes of the module and the description as an object, which `runtime.instantiate` takes as they are. The call is synchronous and takes a few milliseconds for a model of the size of the repressilator, so a page compiles on the main thread; a model of many reactions belongs in a worker next to the simulations. `optimize: false` leaves the optimizer out (the results are the same, the module is larger), `jacobian` is `"auto"`, `"difference-quotient"` or `"analytic"`, and a document the compiler refuses raises a `CompileError` with the message of the compiler. `parameterJacobian: false` omits analytic parameter derivatives and their cached constants for ordinary simulations (0.26.0). Forward sensitivities then use numerical parameter derivatives. `driven` (0.44.0) are the ids of the species to drive, as `sbmlsim-core compile --driven` drives them: each follows the path `X__value + X__slope * (time - driven__t_ref)` of three parameters, the state `X__exchange` accumulates the amount the model gave to it, and `description.driven` and `description.t_ref` give their indices. One compiler compiles any number of models, and `compiler.module` is the compiled `WebAssembly.Module`, which a worker gets to create a compiler of its own. Every buffer of a call into the module is freed when the call ends, also when it throws, and a buffer the module cannot allocate (its memory cannot grow) throws an `Error` instead of being written at the address 0; `withBuffers` of `compile.mjs` is that call, which `petab.mjs` and `stats.mjs` use as well.

## PEtab problems

`petab.mjs` (since 0.39.0) evaluates a problem of [PEtab v2](petab.md) in a page: the YAML file, the tables and the SBML of its models go in as text or bytes, and the log-likelihood, `chi2`, the priors and the simulation table come out, with no server and no python on the way.

```js
import { createCompiler } from "./js/compile.mjs";
import { createRuntime } from "./js/loader.mjs";
import { openPetab } from "./js/petab.mjs";

// compilerWasm: compiler.wasm with the feature petab (the default), a Module or its bytes
const compiler = await createCompiler({ compiler: compilerWasm });
// or a pool of pool.mjs, openPetab({ compiler, pool, files })
const runtime = await createRuntime({ engine, mathlib });
// the files by their names, as text or bytes
const problem = await openPetab({ compiler, runtime, files });

// at the nominal values, and at a value per estimated parameter
const { llh, chi2, logPrior, simulationTable } = await problem.evaluate();
const atX = await problem.evaluate([0.5, 0.4, 3]);
problem.close();
```

What a problem is, its plan and its likelihood are the Rust of `sbmlsim-petab` in `compiler.wasm`, the problem of `sbmlsim-core petab` and of `sbmlsim_core.petab` in python; the module only simulates the requests of the plan with the engine of the page:

1. `openPetab` gives the files to `petab_open` of `compiler.wasm`, which reads and plans the problem, compiles every model of the plan with `petab_model` (the SBML with the observables of the problem added) and instantiates it in the `runtime`, or adds it to the `pool`,
2. `evaluate(x)` asks `petab_requests` for the request of `simulation/1` of every simulation at the point, simulates them (one after the other with a runtime, next to each other on the workers of a pool) and gives the results to `petab_evaluate`: the shapes of the tables as JSON and their values as the bytes of the `Float64Array`s of the loader, so the values cross exactly, `NaN` included, without being written into text and parsed again,
3. `close()` drops the plan in `compiler.wasm` and the models of the runtime or the pool.

The engine of the page is `engine.wasm`, which the command line runs as well, and the likelihood is the same Rust in either, so a page evaluates a problem with the bits of the command line: `js/test/petab.test.mjs` compares the simulation table and the log-likelihood with the ones of `sbmlsim-core petab`, the pool with the loader, and runs the cases of the PEtab test suite at the tolerances of `crates/petab/tests/testsuite.rs`: the 31 of PEtab v2 pass, and the 20 of PEtab v1 which sbmlsim converts where they are present, as in Rust.

| member | what it is |
| --- | --- |
| `openPetab({ compiler, runtime, files, yaml, options })` | the problem from the contents of its files (an object or a `Map` of names to text or bytes), simulated with a `runtime` or, in its place, a `pool`; `yaml` is the name of its YAML file, without it the one file of `.yaml` or `.yml`; `options` are `relativeTolerance` and `absoluteTolerance` of the integrator, `dataAtChange` (`"after"`, `"before"`) and `extension` (`"read"`, `"ignore"`), see [the extension of sbmlsim](petab.md#the-extension-of-sbmlsim); a problem which is not read, planned or compiled raises a `PetabError` with the reason |
| `models`, `parameters`, `nominal`, `lower`, `upper` | the ids of the models and of the estimated parameters, in the order of a point, their nominal values and their bounds (`Float64Array`s) |
| `nSimulations`, `nMeasurements` | the simulations of an evaluation, one per experiment of a model, and the measurements |
| `requests(x)` | the requests of `simulation/1` of the simulations at a point, `[{ model, request }]` |
| `simulations(x, { signal })` | the simulations at a point, `[{ model, request, result }]`, the result `{ columns, nRows, values }` of the loader or `{ error, kind }` of a simulation which failed, `kind` the one of the `EngineError` (since 0.40.0) |
| `evaluate(x, { simulations, signal })` | `llh`, `chi2`, `logPrior`, `logPriors` (by parameter), `unnormLogPosterior`, `simulated` and `sigma` (per measurement, `Float64Array`s), `failures` (`[{ simulation, message }]`) and `simulationTable`, the text of the simulation table of PEtab; from the `simulations` of `simulations(x)` when given |
| `evaluateSimulations(x, simulations)` | the same, synchronous, from simulations which are given |
| `fit({ starts, seed, algorithm, maxIterations, gradientTolerance, stepTolerance, costTolerance, maxSeconds, signal, onStep })` | the fit of the estimated parameters (since 0.40.0), see [the fit of a problem](#the-fit-of-a-problem) |
| `close()` | drops the problem |

A point `x` is a value per estimated parameter, the nominal values without one. A simulation which fails is in `failures` and gives `NaN` for its measurements, so a log-likelihood of `NaN`, as PEtab asks; a log prior outside of the support of its prior is `-Infinity`.

The fit of the pharmacokinetics of hydrochlorothiazide of pkdb_models ([A fit of sbmlsim as PEtab](petab.md#a-fit-of-sbmlsim-as-petab): 6 parameters, 59 simulations, 777 measurements) evaluates in a page to the log-likelihood of the command line, `1620.2615718892766`, bit for bit, and so does its simulation table:

| host | open (read, plan, compile, instantiate) | an evaluation |
| --- | --- | --- |
| Node 24, the runtime on the main thread | 150 ms | 60 ms |
| Node 24, a pool of 8 workers | 150 ms | 21 ms |
| Chrome (headless), the runtime on the main thread | 220 ms | 145 ms |
| Chrome (headless), a pool of the default workers | 180 ms | 45 ms |
| the command line (`sbmlsim-core petab`, wasmtime on the threads of rayon) | 40 ms (the modules of wasmtime cached) | 30 ms |

`compiler.wasm` carries PEtab in every build since 0.39.0: the feature `petab` of `sbmlsim-wasm` is on by default and adds 340 kB to the 3.07 MB of the compiler alone (112 kB of 1.10 MB gzipped), and the fit of a problem (since 0.40.0) 409 kB more, see [the fit of a problem](#the-fit-of-a-problem). A module of its own would have been larger in all: `petab_model` compiles the models of a plan, so a `petab.wasm` holds the whole compiler again (3.41 MB), and the package, the python wheels and the command line would carry both. `cargo build -p sbmlsim-wasm --no-default-features` builds the compiler alone. The inline build of the python package (`sbmlsim_core.web.script()`) carries the module as well (since 0.39.0), `sbmlsimCore.openPetab` with `sbmlsimCore.createCompiler` and `sbmlsimCore.createRuntime`, for a page of one file which evaluates a problem with the bytes of `sbmlsim_core.runtime_wasm()`.

### The fit of a problem

`fit` (since 0.40.0) fits the estimated parameters of a problem in the page: the maximum likelihood estimate, or the maximum a posteriori estimate of a problem whose parameters carry priors. The optimizer is the one of `sbmlsim-core petab --fit`, `sbmlsim-fit` in `compiler.wasm`, and its simulations run on the engine of the problem:

```js
const fit = await problem.fit(); // from the nominal values, as `sbmlsim-core petab --fit`
fit.x;          // the values of the estimated parameters at the optimum (Float64Array)
fit.objective;  // the negative log-likelihood there (the negative log posterior with priors)

const best = await problem.fit({
  starts: 8, seed: 1,             // the nominal values and 7 points drawn in the box
  maxIterations: 200,
  signal: controller.signal,       // stops every fit at its next step, with its best point
  onStep: ({ start, step, simulations, seconds }) => {},
});
best.runs;      // every fit, in the order of the starts; `best` is the one of the lowest objective
```

| option | what it is |
| --- | --- |
| `starts` | the points the fits start from (each a value per estimated parameter), or their number: the nominal values and points drawn uniformly in the space the optimizer searches, `log10` of a parameter of positive bounds, with `seed`; the nominal values alone without it |
| `algorithm` | `"trustRegion"` (trust region reflective, the default), `"levenbergMarquardt"` or `{ differentialEvolution: { seed, population, f, cr, polish } }`, the [optimizers](fitting.md#the-optimizer) of the command line |
| `maxIterations`, `gradientTolerance`, `stepTolerance`, `costTolerance` | what an optimizer may spend and when it stops, `max_iterations` (100) and the tolerances of [the optimizer](fitting.md#the-optimizer); since 0.41.0 a tolerance which is not given is the relative tolerance of the integrator of the problem, at least `1e-8` ([the fit](petab.md#the-fit)) |
| `maxSeconds`, `signal` | the time the fits may take on the clock of the page, and an `AbortSignal`: either stops the fits at their next step, with the best point they reached (`status` `UserRequested`) |
| `onStep` | called after the simulations of every step of every fit, with the index of its `start`, the `step`, the `simulations` it ran and the `seconds` since the fits began |

The answer carries `x`, `objective`, `cost` (the cost of `sbmlsim-fit`, the objective plus the constant `offset` of [the fit](petab.md#the-fit)), `iterations`, `evaluations`, `jacobians`, `status` (the termination of the optimizer, e.g. `RelativeCostTolerance` or `RelativeParamTolerance` for a fit which converged at the tolerances of its step and its cost), `simulations` (the simulations the page ran), `start`, `seconds` and `runs`.

A fit in a page cannot call its simulations, which run on the workers of the page and answer later, so the optimizer is driven ask/tell ([`sbmlsim_fit::Asking`](fitting.md#a-fit-whose-host-simulates)): `petab_fit_ask` runs the optimizer in `compiler.wasm` until it needs evaluations the page has not given and answers the requests of their simulations, which `fit` simulates (next to each other on the workers of a pool) and hands to `petab_fit_tell`, as `evaluate` hands its results to `petab_evaluate`. A step asks for everything it needs at once: the residuals of a trial point, the `k + 1` evaluations of the finite differences of a jacobian, the members of a generation. The page yields to its events between the steps, so a fit on the loader does not freeze a page and a click on stop is heard.

The jacobian is the forward differences of the simulations, which is what `sbmlsim-core petab --fit` takes as well (`JacobianChoice::Auto`): `engine.wasm` writes no sensitivities into its output (the native engine of the command line computes them, for `JacobianChoice::Sensitivities`). The sensitivities in a page need a format of the output of `engine.wasm` with room for them, their transfer by the loader and the pool, and the block of the jacobian of a condition read off them in `compiler.wasm`, which the batch already does natively ([#156](https://github.com/matthiaskoenig/sbmlsim-core/issues/156)).

The fit of a page is the fit of the command line: the same definition, the same optimizer and the same evaluations, so the two differ only where `engine.wasm` and the native engine of the command line differ in the last bits of a double. `js/test/petab-fit.test.mjs` holds the page to the command line on the problem of the demo below and on the 49 cases of the PEtab test suite whose fit the engine carries: 29 are the same to the bit and the objectives of the others agree to `1e-14`, but for three whose objective has no minimum (a noise which is estimated runs to zero), which stop at the tolerance of their step where their rounding takes them; where the paths of the two optimizers part at their last iterations, the optima agree to `1e-4` or are two points of the same valley of the likelihood (a parameter the data does not determine). On the native engine (`crates/petab/tests/asking.rs`) the fit driven ask/tell is the fit of `Executor::fit` to the bit, on every one of them.

The fit of the pharmacokinetics of hydrochlorothiazide of pkdb_models (6 parameters, 59 simulations, 777 measurements, from the nominal values with the default settings of 0.40.0, before the default tolerances of [the fit](petab.md#the-fit)) finds the optimum of the command line, the objective `-1631.5231473` (the command line `-1631.5231468`) and the parameters to `1e-6`, after 41 evaluations and 8 iterations, 6136 simulations:

| host | the fit |
| --- | --- |
| Node 24, the runtime on the main thread | 6.4 s |
| Node 24, a pool of 8 workers | 2.1 s |
| the command line (`sbmlsim-core petab --fit`, the native engine on 20 threads) | 0.85 s |

`compiler.wasm` carries the fit by default since 0.40.0: the feature `fit` of `sbmlsim-wasm` (the optimizers of basin, the objective of `sbmlsim-fit` and the non-compartmental analysis of pkpdutils-core a curve may read) adds 409 kB to the 3.42 MB of the compiler with PEtab, 3.83 MB in all (143 kB of 1.25 MB gzipped). `cargo build -p sbmlsim-wasm --no-default-features --features petab` builds a module which evaluates a problem and fits none, and `fit` then says so. The module imports nothing: basin reads the clock of its time limit from web-time, which is JavaScript through wasm-bindgen in `wasm32-unknown-unknown`, so the workspace replaces web-time with the time of the standard library (`crates/web-time`), and a fit in a page reads the clock of the page.

### A problem evaluated in this page

A small problem of PEtab v2 runs below, with the `petab.mjs`, `compiler.wasm` and `engine.wasm` of the npm package. Its model converts A into B (`k1`) and B back into A (`k2`), an experiment starts with A at 1 and doses one more unit of A at time 5, and A and B are measured twelve times each, with a normal noise of 0.04 (the data are drawn at `k1 = 0.6` and `k2 = 0.25`, the nominal values of the problem). Move the slider: the problem is evaluated again at the new `k1` and the nominal `k2`, which gives the log-likelihood `llh` and `chi2` of PEtab and the simulated value of every measurement. With a package which carries [the fit](#the-fit-of-a-problem) the button **fit k1 and k2** fits both in the page, from the nominal values, and shows the problem at the optimum; the slider then moves `k1` at the `k2` of the fit.

<div class="pt" data-base="../demo/">
<div class="pt-head">
<label class="pt-knob">rate constant <b>k1</b><input class="pt-range" name="rate-constant-k1" type="range" min="0.2" max="1.5" step="0.01" value="0.6" aria-label="rate constant k1"><output class="pt-value">0.60</output></label>
<button class="pt-fit md-button" type="button" hidden>fit k1 and k2</button>
</div>
<p class="pt-stats"><span>llh <b class="pt-llh">-</b></span><span>chi2 <b class="pt-chi2">-</b></span><span class="pt-nominal"></span></p>
<p class="pt-state">loading the compiler, the engine and the problem ...</p>
<svg class="pt-plot" viewBox="0 0 660 270" role="img" aria-label="The measurements of A and B over 20 time units, the simulation at the chosen k1 and the residual of every measurement. The same numbers are in the table below the figure."></svg>
<details class="pt-table"><summary>The measurements and their simulated values</summary><div></div></details>
</div>

The dots are the measurements and the thin lines between a dot and its curve the residuals which `chi2` sums, each divided by its noise and squared. The `llh` is largest near the nominal `k1` and falls off on either side: a smaller `k1` leaves too much A, a larger one too much B. The problem is the seven files of `docs/demo/conversion/` (`problem.yaml`, the SBML of the model and the five tables, 3 kB), fetched and handed to `openPetab` as text: at the nominal values the page evaluates the llh of `sbmlsim_core.petab` in python, `47.29120890853149`, bit for bit, and a fit in python finds `k1 = 0.63` and `k2 = 0.27`. The curves are a second simulation of the model on a fine grid, with the dose of the experiment as an increment of A, since the simulations of a problem give only the times of its measurements.

```js
import { createCompiler } from "sbmlsim-core/compile";
import { createRuntime } from "sbmlsim-core";
import { openPetab } from "sbmlsim-core/petab";

const NAMES = ["problem.yaml", "model.xml", "conditions.tsv", "experiments.tsv",
  "observables.tsv", "measurements.tsv", "parameters.tsv"];
const files = Object.fromEntries(await Promise.all(
  NAMES.map(async (name) => [name, await fetch(name).then((r) => r.text())]),
));
const problem = await openPetab({ compiler, runtime, files });

problem.parameters;                    // ["k1", "k2"], the order of a point
const nominal = await problem.evaluate();
const { llh, chi2, simulated } = await problem.evaluate([1.2, problem.nominal[1]]);
```

<style>
.pt { margin: 1rem 0 1.4rem; --pt-a: #2a78d6; --pt-b: #eb6834;
  --pt-ink: var(--md-default-fg-color, #1a1a19); --pt-soft: var(--md-default-fg-color--light, #6b6b68);
  --pt-line: var(--md-default-fg-color--lighter, #d6d6d3); --pt-bg: var(--md-default-bg-color, #ffffff); }
[data-md-color-scheme="slate"] .pt { --pt-a: #3987e5; --pt-b: #d95926; }
.pt-head { display: flex; flex-wrap: wrap; align-items: center; gap: .6rem 1.8rem; margin-bottom: .3rem; }
.pt-knob { display: flex; align-items: center; gap: .6rem; font-size: .82rem; white-space: nowrap; }
.pt-range { width: 15rem; max-width: 40vw; accent-color: var(--pt-a); }
.pt-value { font-variant-numeric: tabular-nums; font-weight: 600; min-width: 2.6rem; }
.pt .pt-fit { margin: 0; padding: .3em 1.1em; font-size: .76rem; }
.pt .pt-fit:disabled { opacity: .5; cursor: progress; }
.pt-stats { display: flex; flex-wrap: wrap; align-items: baseline; gap: .2rem 1.4rem; margin: 0 0 .2rem;
  font-size: .82rem; font-variant-numeric: tabular-nums; }
.pt-stats b { display: inline-block; min-width: 4.4rem; margin-left: .35rem; }
.pt-nominal { color: var(--pt-soft); font-size: .76rem; }
.pt-state { margin: 0 0 .5rem; font-size: .76rem; color: var(--pt-soft); font-variant-numeric: tabular-nums; }
.pt-plot { width: 100%; height: auto; }
.pt-plot text { fill: var(--pt-soft); font-size: 11px;
  font-family: var(--md-text-font-family, system-ui), sans-serif; }
.pt-plot .pt-name { font-size: 12px; font-weight: 600; }
.pt-plot .pt-axis { stroke: var(--pt-line); stroke-width: 1; }
.pt-plot .pt-curve { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
.pt-plot .pt-residual { stroke-width: 1; }
.pt-plot .pt-dot { stroke: var(--pt-bg); stroke-width: 1.5; }
.pt-table { margin-top: .4rem; font-size: .76rem; }
.pt-table summary { cursor: pointer; color: var(--pt-soft); }
.pt-table table { font-variant-numeric: tabular-nums; }
.pt-failed { color: var(--pt-ink); }
</style>
<script type="module">
const root = document.querySelector(".pt");
if (root) {
  const base = root.dataset.base;
  const plot = root.querySelector(".pt-plot");
  const state = root.querySelector(".pt-state");
  const range = root.querySelector(".pt-range");
  const value = root.querySelector(".pt-value");
  const llhOut = root.querySelector(".pt-llh");
  const chi2Out = root.querySelector(".pt-chi2");
  const nominalOut = root.querySelector(".pt-nominal");
  const fitButton = root.querySelector(".pt-fit");
  const table = root.querySelector(".pt-table div");
  const NAMES = ["problem.yaml", "model.xml", "conditions.tsv", "experiments.tsv",
    "observables.tsv", "measurements.tsv", "parameters.tsv"];
  // the observables of the problem, the species they read and their colours
  const SERIES = [
    { observable: "obs_a", species: "A", color: "--pt-a" },
    { observable: "obs_b", species: "B", color: "--pt-b" },
  ];
  // the experiment `dosing` of `experiments.tsv`: one unit of A more at this time
  const DOSE_TIME = 5;
  const END = 20, TOP = 2;
  const SVG = "http://www.w3.org/2000/svg";
  const W = 660, H = 270, L = 52, R = 42, T = 14, B = 34;
  const node = (name, attrs, text) => {
    const e = document.createElementNS(SVG, name);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    if (text !== undefined) e.textContent = text;
    return e;
  };
  const text = async (name) => {
    const response = await fetch(base + "conversion/" + name);
    if (!response.ok) throw new Error(`${name}: ${response.status} ${response.statusText}`);
    return response.text();
  };
  const x = (t) => L + (t / END) * (W - L - R);
  const y = (v) => H - B - (Math.min(v, TOP) / TOP) * (H - T - B);
  const color = (s) => getComputedStyle(root).getPropertyValue(s.color).trim();

  // the curves: the model alone on a fine grid, the dose as an increment of A
  const curveRequest = (k1, k2) => ({
    contract: "simulation/1",
    start: 0,
    end: END,
    output: { uniform: { steps: 400 } },
    preinit: [["k1", k1], ["k2", k2]],
    changes: [{ time: DOSE_TIME, values: [], increments: [["A", 1]] }],
    steady_state: null,
    selections: ["time", "A", "B"],
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

  const draw = (curve, measurements, simulated) => {
    const index = Object.fromEntries(curve.columns.map((c, i) => [c, i]));
    const at = (row, id) => curve.values[row * curve.columns.length + index[id]];
    plot.replaceChildren();
    plot.append(node("line", { class: "pt-axis", x1: L, y1: H - B, x2: W - R, y2: H - B }));
    plot.append(node("line", { class: "pt-axis", x1: L, y1: T, x2: L, y2: H - B }));
    for (const t of [0, 5, 10, 15, 20]) {
      plot.append(node("text", { x: x(t), y: H - B + 16, "text-anchor": "middle" }, String(t)));
    }
    plot.append(node("text", { x: (L + W - R) / 2, y: H - 4, "text-anchor": "middle" }, "time"));
    for (const v of [0, 0.5, 1, 1.5, 2]) {
      plot.append(node("text", { x: L - 8, y: y(v) + 4, "text-anchor": "end" }, v.toFixed(1)));
      if (v > 0) plot.append(node("line", { class: "pt-axis", x1: L, y1: y(v), x2: W - R, y2: y(v), "stroke-dasharray": "3 4" }));
    }
    plot.append(node("line", { class: "pt-axis", x1: x(DOSE_TIME), y1: T, x2: x(DOSE_TIME), y2: H - B }));
    plot.append(node("text", { x: x(DOSE_TIME) + 6, y: T + 10 }, "dose of A"));
    const labels = [];
    for (const s of SERIES) {
      const stroke = color(s);
      const points = [];
      for (let r = 0; r < curve.nRows; r += 1) points.push(x(at(r, "time")) + "," + y(at(r, s.species)));
      plot.append(node("polyline", { class: "pt-curve", points: points.join(" "), stroke }));
      labels.push({ id: s.species, stroke, y: y(at(curve.nRows - 1, s.species)) });
    }
    // the residual of every measurement, from the measurement to the simulated value of
    // the problem, and the measurement on top
    measurements.forEach((m, k) => {
      const stroke = color(SERIES.find((s) => s.observable === m.observable));
      plot.append(node("line", { class: "pt-residual", stroke,
        x1: x(m.time), y1: y(m.value), x2: x(m.time), y2: y(simulated[k]) }));
    });
    measurements.forEach((m) => {
      const fill = color(SERIES.find((s) => s.observable === m.observable));
      plot.append(node("circle", { class: "pt-dot", cx: x(m.time), cy: y(m.value), r: 3.6, fill }));
    });
    labels.sort((a, b) => a.y - b.y);
    for (let i = 1; i < labels.length; i += 1) {
      labels[i].y = Math.max(labels[i].y, labels[i - 1].y + 13);
    }
    for (const label of labels) {
      plot.append(node("text", { class: "pt-name", x: W - R + 6, y: label.y + 4, fill: label.stroke }, label.id));
    }
    // what the marks are, top right, where the curves leave room for it
    const key = W - R - 150;
    plot.append(node("circle", { cx: key + 4, cy: T + 8, r: 3.6, fill: color(SERIES[0]), class: "pt-dot" }));
    plot.append(node("text", { x: key + 14, y: T + 12 }, "measured"));
    plot.append(node("line", { x1: key + 78, y1: T + 8, x2: key + 96, y2: T + 8, stroke: color(SERIES[0]), "stroke-width": 2 }));
    plot.append(node("text", { x: key + 102, y: T + 12 }, "simulated"));
  };

  const tabulate = (measurements, evaluation) => {
    const rows = measurements.map((m, k) => {
      const s = evaluation.simulated[k], sigma = evaluation.sigma[k];
      return "<tr><td>" + m.observable + "</td><td>" + m.time + "</td><td>" + m.value.toFixed(3) +
        "</td><td>" + s.toFixed(3) + "</td><td>" + sigma + "</td><td>" + ((m.value - s) / sigma).toFixed(2) + "</td></tr>";
    });
    table.innerHTML = "<table><thead><tr><th>observable</th><th>time</th><th>measured</th>" +
      "<th>simulated</th><th>noise</th><th>residual</th></tr></thead><tbody>" + rows.join("") + "</tbody></table>";
  };

  try {
    const { createCompiler } = await import(base + "compile.mjs");
    const { createRuntime } = await import(base + "loader.mjs");
    const { openPetab } = await import(base + "petab.mjs");
    const bytes = (name) => fetch(base + name).then((r) => r.arrayBuffer());
    const compiler = await createCompiler({ compiler: await bytes("compiler.wasm") });
    const runtime = await createRuntime({
      engine: await bytes("engine.wasm"),
      mathlib: await bytes("mathlib.wasm"),
    });
    const files = Object.fromEntries(
      await Promise.all(NAMES.map(async (name) => [name, await text(name)])),
    );
    const opened = performance.now();
    const problem = await openPetab({ compiler, runtime, files });
    const openMs = performance.now() - opened;
    const { wasm, description } = compiler.compile(files["model.xml"]);
    const model = await runtime.instantiate(wasm, description);
    // the measurements in the order of the table, the order of `simulated` and `sigma`
    const measurements = files["measurements.tsv"].trim().split("\n").slice(1).map((line) => {
      const [observable, , time, value] = line.split("\t");
      return { observable, time: Number(time), value: Number(value) };
    });
    const k1Nominal = problem.nominal[problem.parameters.indexOf("k1")];
    // the k2 of the evaluations: the nominal one, and the one of a fit after it
    let k2 = problem.nominal[problem.parameters.indexOf("k2")];
    const nominal = await problem.evaluate();
    nominalOut.textContent = "llh " + nominal.llh.toFixed(2) + " at the nominal k1 = " + k1Nominal.toFixed(2);
    let busy = false, again = false;
    const run = async () => {
      if (busy) { again = true; return; }
      busy = true;
      try {
        do {
          again = false;
          const k1 = Number(range.value);
          value.textContent = k1.toFixed(2);
          const point = problem.parameters.map((id) => (id === "k1" ? k1 : k2));
          const started = performance.now();
          const evaluation = await problem.evaluate(point);
          const ms = performance.now() - started;
          llhOut.textContent = evaluation.llh.toFixed(2);
          chi2Out.textContent = evaluation.chi2.toFixed(2);
          draw(model.simulate(curveRequest(k1, k2)), measurements, evaluation.simulated);
          tabulate(measurements, evaluation);
          state.textContent = problem.nMeasurements + " measurements evaluated in this page in " +
            ms.toFixed(1) + " ms, the problem opened in " + openMs.toFixed(0) + " ms";
        } while (again);
      } finally {
        busy = false;
      }
    };
    range.addEventListener("input", () => {
      run().catch((error) => {
        state.classList.add("pt-failed");
        state.textContent = "The evaluation failed: " + error.message;
      });
    });
    await run();
    // the fit, with a package whose petab.mjs and compiler.wasm carry it
    if (typeof problem.fit === "function" && compiler.exports?.petab_fit_open !== undefined) {
      fitButton.hidden = false;
      fitButton.addEventListener("click", async () => {
        fitButton.disabled = true;
        range.disabled = true;
        try {
          const fit = await problem.fit({
            onStep: ({ step, simulations }) => {
              state.textContent = "fitting in this page: step " + step + ", " + simulations + " simulations";
            },
          });
          const at = (id) => fit.x[problem.parameters.indexOf(id)];
          k2 = at("k2");
          range.value = String(at("k1"));
          await run();
          nominalOut.textContent = "the fit: k1 = " + at("k1").toFixed(3) + ", k2 = " + k2.toFixed(3) +
            ", llh " + (-fit.objective).toFixed(2) + " after " + fit.iterations + " iterations";
          state.textContent = "fitted in this page in " + (1000 * fit.seconds).toFixed(0) + " ms, " +
            fit.simulations + " simulations of the problem (" + fit.status + ")";
        } catch (error) {
          state.classList.add("pt-failed");
          state.textContent = "The fit failed: " + error.message;
        } finally {
          fitButton.disabled = false;
          range.disabled = false;
        }
      });
    }
  } catch (error) {
    state.classList.add("pt-failed");
    state.textContent = "The demo could not run in this browser: " + error.message;
  }
}
</script>

## The statistics

`stats.mjs` (since 0.43.0) gives a page the [statistics of the analyses](stats.md), every function of `sbmlsim_core.stats` of python: the metrics of a fit, the Fisher information and the posterior, the statistics of draws, the draws and their correlations, the probability statements, the visual predictive check and (since 0.44.0) the distributions and special functions of scipy (`normCdf`, `normPpf`, `tCdf`, `tPpf`, `chi2Cdf`, `chi2Ppf`, `erf`, `gamma`), whose arguments broadcast as numpy broadcasts them: `stats.chi2Ppf(0.95, [1, 2, 3])` is a `Float64Array` of three quantiles, `stats.erf(0.5)` a number.

```js
import { createCompiler } from "./js/compile.mjs";
import { createStats } from "./js/stats.mjs";

// compilerWasm: compiler.wasm with the feature stats (the default), a Module or its bytes
const stats = createStats({ compiler: await createCompiler({ compiler: compilerWasm }) });

// a fit of two parameters to 20 data points: the jacobian of the residuals, a row per residual
const jacobian = Array.from({ length: 20 }, (_, i) => {
  const t = (10 * i) / 19;
  return [Math.exp(-0.3 * t), -t * Math.exp(-0.3 * t)];
});
const fisher = stats.fisherStatistics(stats.gram(jacobian), 0.05, 20);
const [low, high] = stats.fisherIntervals([1.0, 0.3], fisher.standardErrors, ["LINEAR", "LOG10"], { dof: 18 });
// fisher.rank 2, low [0.89, 0.27], high [1.11, 0.34], Float64Arrays

// 1024 draws of two uniform variables whose normal scores correlate, and their sum
const u = stats.unitPoints(2, 1024, { design: "sobol", seed: 1 });
const z = stats.copula(u, [[1.0, 0.8], [0.8, 1.0]]);
const y = z.map(([a, b]) => a + b);
const [q, qLow, qHigh, n] = stats.quantileCi(y, 0.95);

// is the probability of y > 0.5 above 0.6?
const sides = ["greater", "less", "equivalent"].map((side) => stats.pValues(y.map((v) => v > 0.5), 0.6, side));
const [decision, draws] = stats.decision(...sides, 0.05); // ["accepted", 73]
```

The functions are the ones of python with their names in camelCase (`quantile_ci` is `quantileCi`), and their constants are members by their names (`stats.EPS`, `stats.MARGIN`, `stats.MIXTURE`, ...). `createStats({ compiler })` takes the compiler of `createCompiler` and is synchronous, as every function is.

- The required arguments of python are the arguments of the function in their order; every optional or keyword-only argument of python is a key of an object of options after them, in camelCase: `stats.foldCounts(y, f, { fold: 3 })`, `stats.unitPoints(2, 64, { design: "lhs", seed: 7 })`, `stats.intervalParameters(1, 2, { level: 0.9, lognormal: true })`. An option `undefined` or `null` takes the default of python; an unknown option, a missing argument or a value of another type (a string for a number, a fraction for an integer) is a `TypeError`.
- An array is an array of numbers or a typed array of numbers (not `BigInt64Array`); nested arrays are the dimensions of numpy, a matrix an array of its rows, and a number alone an array of no dimension. A matrix of no rows has no nested form: `[]` is a vector of no values. Booleans (`satisfied` of `pValues`) are numbers or booleans, a number true unless it is 0, as numpy converts them.
- An integer is a number or a `BigInt` (a seed of 64 bits); one past the integers of python's binding (`i64`, `u64` of a seed) is a `RangeError`.
- A vector of a result is a `Float64Array`, a matrix an array of `Float64Array` rows, a tuple of python an array (`const [mean, se, n] = stats.meanSe(x)`), the dictionary of `fisherStatistics` an object with its keys in camelCase (`standardErrors`), and `None` is `null`.
- An argument python refuses throws a `StatsError` with the message of python and `kind`, the class of the exception python raises: `ValueError`, `OverflowError`, `ZeroDivisionError` or `IndexError`. Where python writes a number into a message as it was given (`'1'` of `fold=1`), JavaScript cannot tell 1 from 1.0 and the message has the double (`'1.0'`), as python gives it for `fold=1.0`; and an option `null` takes the default where python refuses `None` (`design=None`).

What every function computes, and how it checks its arguments, is the Rust of `sbmlsim-stats` in `compiler.wasm` (the feature `stats` of `sbmlsim-wasm`, on by default), the code python runs; the module only converts the arguments and the results. One call of the C ABI runs every function, `stats_call(request, request_len, values, values_len, out)`: the request is the JSON of `{"function": <the name of python>, <its arguments by their names of python>}`, in which a double, alone or of an array, is `{"at", "shape"}` of the values, the little-endian bytes of the doubles, and an integer the string of its digits; the answer is the JSON of the result in the same form with the bytes of its doubles, or the JSON of `{"kind", "message"}` of an error, in the four words of `compile` (`crates/wasm/src/stats.rs`). Every double crosses as its bits, the infinities and `-0` included (a `NaN` arrives as a `NaN`, JavaScript keeps no payload), and the functions are deterministic, so a page gives the bits of python and of the crate natively: `js/test/stats.test.mjs` runs about a thousand calls of python with their results (`js/test/fixtures/stats.json`, written by `make_stats.py` from the inputs of the oracle fixtures of the crate, its draws and its errors of every kind) through `compiler.wasm` and compares them bit for bit, the errors by kind and message.

The feature adds 441 kB to the 3.93 MB of `compiler.wasm` with PEtab and its fit (124 kB of 1.28 MB gzipped); `cargo build -p sbmlsim-wasm --no-default-features --features fit` leaves it out. The inline build of the python package carries it as `sbmlsimCore.createStats` with `sbmlsimCore.createCompiler`.

## The batched kernel

```js
import { createCompiler } from "./js/compile.mjs";
import { instantiateKernel } from "./js/kernel.mjs";

// compilerWasm: a WebAssembly.Module or the bytes of compiler.wasm
const compiler = await createCompiler({ compiler: compilerWasm });
const { wasm, description } = compiler.compileKernel(sbmlText, { driven: ["S_ext", "P_ext"] });
// the memory of the caller holds every array of the points; the kernel never grows it
const memory = new WebAssembly.Memory({ initial: 16 });
const kernel = await instantiateKernel({ wasm, mathlib, memory });
kernel.init(n, parameters, state);
// per interval: the path of every driven species of every point into its parameters
// (description.model.driven[i].value, .slope and description.model.t_ref), its accumulator to 0
const failed = kernel.advance(n, state, parameters, control, counts, context, scratch);
// the targets of the assignment rules at the time of every point, into its parameters
kernel.assign(n, state, parameters, control);
```

`compiler.compileKernel(sbml, { driven = [] })` (since 0.41.0) compiles the [batched kernel](compiling.md#a-batched-kernel) of a model (`kernel/2` since 0.44.0): the model with its RODAS4 integrator in one module, which advances any number of points per call, the cells of a tissue in a page, say. It gives `{ wasm, description }`, the `kernel.wasm` and the `kernel.json` of `sbmlsim-core compile --kernel` (with `--driven`) byte for byte (`js/test/kernel.test.mjs`), and raises a `CompileError` for a model the kernel does not integrate (events, delays, algebraic rules) or a species it cannot drive, with the message of `compile`.

`driven` (0.44.0) are the ids of the species the kernel drives, the cells coupled to the blood of a tissue, say: a driven species leaves the states and follows the path `X__value + X__slope * (t - driven__t_ref)` wherever the math reads it, and the state `X__exchange` accumulates the amount the model gave to it (negative for what it took). The drive goes through the parameters, so the arrays and the functions are those of a kernel without: before an `advance` the caller writes the path of every species and point into the parameters of the point (at the indices `value` and `slope` of `description.model.driven` and `description.model.t_ref`, the driven species and the reference time of the description of the model, which the kernel's description carries; both are absent without driven species) and writes 0 into its accumulators (`exchange`, an index of `description.state_ids`), whose values after the `advance` are the amounts exchanged over the interval. Every call computes what the kernel derives from the parameters of each point anew, so a path, and any other parameter, may change between the calls. `init` gives `X__value` the initial value of the species (where it is NaN) and `X__slope` and `driven__t_ref` 0. The description is `kernel/2`: a `kernel/1` model had no driven species.

`instantiateKernel({ wasm, mathlib, memory })` checks that the module imports only `env.memory` and functions of mathlib and exports the three functions of the contract, and instantiates it with mathlib and the `WebAssembly.Memory` of the caller. It gives `{ module, memory, init, advance, assign }`: the arguments of the functions are byte offsets into `memory`, each a multiple of 8, of arrays of `f64` with the values of the points one point after the other (the parameters, `description.parameter_ids` per point; the states, `description.state_ids`; the control `t`, `h`, `done`, `failed`; the counts; the context `t1`, `rtol`, `atol`, `h_min`, `attempts`; the scratch, `description.scratch_per_point` per point). `init` takes a parameter which is NaN as not given (it gets its SBML value and initial assignment) and writes both the parameters and the states; `advance` makes step attempts for every point which is neither done nor failed and gives the number of failed points, and leaves the targets of the assignment rules in the parameters as they are; `assign` evaluates the assignment rules of every point at its time and its states and writes their targets (`description.assigned_ids`) into its parameters, so an output which is the target of a rule of a state or of the time has its value after an advance. All three give -1 and write nothing for invalid arguments. The module exports `KERNEL_CONTRACT` as well.

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
