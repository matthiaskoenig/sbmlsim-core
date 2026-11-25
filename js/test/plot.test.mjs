// The renderer in a JavaScript host: the bytes of the native renderer, the time of a redraw, the
// pointer in data units and the messages of a refused spec.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

import { Plot, PlotError, loadPlot } from "../plot.mjs";
import { ROOT, runtimeDir } from "./helpers.mjs";

const SPECS = path.join(ROOT, "crates", "plot", "tests", "specs");
const GOLDEN = path.join(ROOT, "crates", "plot", "tests", "golden");
const wasm = () => path.join(runtimeDir(), "plot.wasm");
const spec = (name) => JSON.parse(readFileSync(path.join(SPECS, `${name}.json`), "utf8"));

const SIMPLE = {
  contract: "figure/1",
  width: 300,
  height: 200,
  grid: { rows: 1, columns: 1 },
  plots: [
    {
      id: "p",
      row: 0,
      column: 0,
      x: { label: "t" },
      y: { label: "c" },
      elements: [{ id: "c", kind: "curve", x: [0, 1, 2], y: [1, 2, 4] }],
    },
  ],
};

test("same bytes as native", async () => {
  const plot = await loadPlot(wasm());
  const names = readdirSync(SPECS)
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.slice(0, -5));
  assert.ok(names.length > 0);
  for (const name of names) {
    const expected = readFileSync(path.join(GOLDEN, `${name}.svg`), "utf8");
    assert.equal(plot.render(spec(name)), expected, name);
  }
});

test("a source is a path, bytes or an ArrayBuffer", async () => {
  const bytes = readFileSync(wasm());
  const expected = (await loadPlot(wasm())).render(SIMPLE);
  assert.equal((await loadPlot(new Uint8Array(bytes))).render(SIMPLE), expected);
  const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  assert.equal((await loadPlot(buffer)).render(JSON.stringify(SIMPLE)), expected);
});

test("render under 16 ms", async () => {
  const plot = await loadPlot(wasm());
  const figure = spec("benchmark");
  plot.render(figure);
  const times = [];
  for (let i = 0; i < 20; i += 1) {
    const start = performance.now();
    plot.render(figure);
    times.push(performance.now() - start);
  }
  times.sort((a, b) => a - b);
  const median = (times[9] + times[10]) / 2;
  const p95 = times[18];
  console.log(`# render median ${median.toFixed(2)} ms, p95 ${p95.toFixed(2)} ms`);
  assert.ok(median < 16, `median ${median} ms`);
  assert.ok(p95 < 16, `p95 ${p95} ms`);
});

test("pick in data units", async () => {
  const plot = await loadPlot(wasm());
  const geometry = plot.geometry(SIMPLE);
  assert.ok(geometry.plots.length === 1);
  assert.equal(plot.pick(SIMPLE, -50, -50), null);
  // scan the panel for a pixel which picks the middle point
  let hit = null;
  for (let x = 0; x < 300 && !hit; x += 2) {
    for (let y = 0; y < 200 && !hit; y += 2) hit = plot.pick(SIMPLE, x, y, 3);
  }
  assert.ok(hit, "some pixel picks a point");
  assert.equal(hit.plot, "p");
  assert.equal(hit.element, "c");
  assert.ok([1, 2, 4].includes(hit.values.y) && [0, 1, 2].includes(hit.values.x), JSON.stringify(hit));
  assert.equal(hit.values.y, [1, 2, 4][hit.index]);
});

test("invalid spec throws with the path", async () => {
  const plot = await loadPlot(wasm());
  const bad = structuredClone(SIMPLE);
  bad.plots[0].elements[0].y = [1, 2];
  assert.throws(() => plot.render(bad), (error) => error.name === "PlotError" && /plots/.test(error.message));
  assert.throws(() => plot.render("{}"), /: /);
  // a refusal leaves the instance usable
  assert.ok(plot.render(SIMPLE).startsWith("<svg"));
});

test("a file URL is a path in Node", async () => {
  const url = pathToFileURL(wasm());
  const expected = (await loadPlot(wasm())).render(SIMPLE);
  assert.equal((await loadPlot(url)).render(SIMPLE), expected);
  assert.equal((await loadPlot(url.href)).render(SIMPLE), expected);
});

test("prepared figure equals the unprepared answers", async () => {
  const plot = await loadPlot(wasm());
  for (const name of readdirSync(SPECS).map((f) => f.slice(0, -5))) {
    const figure = plot.prepare(spec(name));
    assert.equal(figure.svg(), readFileSync(path.join(GOLDEN, `${name}.svg`), "utf8"), name);
    assert.deepEqual(figure.geometry(), plot.geometry(spec(name)), name);
    const panel = figure.geometry().plots[0].rect;
    const [x, y] = [panel.x + panel.width / 2, panel.y + panel.height / 2];
    assert.deepEqual(figure.pick(x, y, 30), plot.pick(spec(name), x, y, 30), name);
    figure.dispose();
  }
});

test("prepared figure picks without re-parsing", async () => {
  const plot = await loadPlot(wasm());
  const figure = plot.prepare(spec("benchmark"));
  const panel = figure.geometry().plots[0].rect;
  const [x, y] = [panel.x + panel.width / 2, panel.y + panel.height / 2];
  figure.pick(x, y);
  const times = [];
  for (let i = 0; i < 50; i += 1) {
    const start = performance.now();
    figure.pick(x + i, y, 8);
    times.push(performance.now() - start);
  }
  times.sort((a, b) => a - b);
  const median = times[25];
  console.log(`# prepared pick median ${median.toFixed(3)} ms`);
  assert.ok(median < 0.5, `median ${median} ms`);
  figure.dispose();
});

test("a disposed figure throws and a refused spec is no handle", async () => {
  const plot = await loadPlot(wasm());
  const figure = plot.prepare(SIMPLE);
  figure.dispose();
  figure.dispose();
  assert.throws(() => figure.svg(), /disposed/);
  assert.throws(() => figure.pick(1, 1), /disposed/);
  assert.throws(() => plot.prepare("{}"), (error) => error.name === "PlotError");
  assert.ok(plot.render(SIMPLE).startsWith("<svg"));
});

test("a failure of any kind marks the plot broken", async () => {
  const memory = new WebAssembly.Memory({ initial: 1 });
  let freed = 0;
  const exports = {
    memory,
    allocate: () => 16,
    deallocate: () => {
      freed += 1;
    },
    render() {
      throw new Error("plot.wasm called WASI fd_write");
    },
    geometry: () => 1,
    pick: () => 1,
  };
  const plot = new Plot(exports, () => "panicked at src/x.rs:1");
  assert.throws(() => plot.render(SIMPLE), /fd_write.*panicked at src\/x\.rs:1.*unusable/s);
  assert.equal(plot.broken, true);
  assert.equal(freed, 0, "nothing is freed in an instance which failed");
  assert.throws(() => plot.geometry(SIMPLE), /unusable/);
  // a RangeError (stack overflow) and a failing allocate are failures as well
  for (const failing of [
    { ...exports, render() { throw new RangeError("Maximum call stack size exceeded"); } },
    { ...exports, allocate() { throw new Error("out of memory"); } },
  ]) {
    const other = new Plot(failing);
    assert.throws(() => other.render(SIMPLE), /unusable/);
    assert.equal(other.broken, true);
  }
});

test("buffers are freed when a spec is refused", async () => {
  const memory = new WebAssembly.Memory({ initial: 1 });
  const freed = [];
  const plot = new Plot({
    memory,
    allocate: (n) => (n === 8 ? 8 : 64),
    deallocate: (p, n) => freed.push([p, n]),
    render: () => 0,
  });
  assert.throws(() => plot.render(SIMPLE), PlotError);
  assert.equal(freed.length, 3, "the result, out and the input");
  assert.equal(plot.broken, false);
});

test("a trap makes the plot unusable", async () => {
  // a panic of the module is a `WebAssembly.RuntimeError` out of its export: the exports here
  // are a memory and a `render` which traps
  const memory = new WebAssembly.Memory({ initial: 1 });
  const plot = new Plot({
    memory,
    allocate: () => 16,
    deallocate() {},
    render() {
      throw new WebAssembly.RuntimeError("unreachable");
    },
    geometry: () => 1,
    pick: () => 1,
  });
  assert.throws(() => plot.render(SIMPLE), /failed \(unreachable\)/);
  assert.throws(() => plot.render(SIMPLE), /unusable/);
  assert.throws(() => plot.geometry(SIMPLE), /unusable/);
  // another Plot is not affected
  assert.ok((await loadPlot(wasm())).render(SIMPLE).startsWith("<svg"));
});
