// The renderer in a JavaScript host: the bytes of the native renderer, the time of a redraw, the
// pointer in data units and the messages of a refused spec.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { Plot, loadPlot } from "../plot.mjs";
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
  console.log(`# render median ${median.toFixed(2)} ms`);
  assert.ok(median < 16, `median ${median} ms`);
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
  assert.throws(() => plot.render(SIMPLE), /trapped/);
  assert.throws(() => plot.render(SIMPLE), /unusable/);
  assert.throws(() => plot.geometry(SIMPLE), /unusable/);
  // another Plot is not affected
  assert.ok((await loadPlot(wasm())).render(SIMPLE).startsWith("<svg"));
});
