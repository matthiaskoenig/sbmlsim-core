// `hydrateFigures` on a static report: every figure becomes interactive, a broken spec leaves its
// static SVG and warns once.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { test } from "node:test";

import { parseHTML } from "linkedom";

import { hydrateFigures } from "../hydrate.mjs";
import { ROOT, runtimeDir } from "./helpers.mjs";

const SPECS = path.join(ROOT, "crates", "plot", "tests", "specs");
const wasm = () => readFileSync(path.join(runtimeDir(), "plot.wasm"));

const figure = (key, spec) =>
  `<figure data-figure="${key}"><svg class="sbmlsim-figure"><text>static ${key}</text></svg>` +
  `<script type="application/json" data-figure-spec="${key}">${spec.replaceAll("</", "<\\/")}</script></figure>`;

function report(...figures) {
  const { document, window } = parseHTML(`<!doctype html><html><head></head><body>${figures.join("")}</body></html>`);
  window.requestAnimationFrame = () => 0;
  window.requestIdleCallback = () => 0;
  return document;
}

test("hydrates_every_figure", async () => {
  const grid = readFileSync(path.join(SPECS, "grid.json"), "utf8");
  const bench = readFileSync(path.join(SPECS, "benchmark.json"), "utf8");
  const document = report(figure("a", grid), figure("b", bench));
  const handles = await hydrateFigures(document, { wasm: wasm() });
  assert.equal(handles.length, 2);
  for (const key of ["a", "b"]) {
    const svg = document.querySelector(`figure[data-figure="${key}"] svg`);
    assert.ok(svg.querySelector("[data-element]"), "the rendered figure replaced the static one");
    assert.ok(!svg.textContent.includes("static"));
    assert.ok(document.querySelector(`script[data-figure-spec="${key}"]`), "the spec stays");
  }
});

test("broken_spec_keeps_static_svg", async () => {
  const grid = readFileSync(path.join(SPECS, "grid.json"), "utf8");
  const document = report(figure("good", grid), figure("bad", '{"contract": "figure/1"'), figure("worse", "{}"));
  const warnings = [];
  const original = console.warn;
  console.warn = (...args) => warnings.push(args.join(" "));
  let handles;
  try {
    handles = await hydrateFigures(document, { wasm: wasm() });
  } finally {
    console.warn = original;
  }
  assert.equal(handles.length, 1);
  assert.match(document.querySelector('figure[data-figure="bad"] svg').textContent, /static bad/);
  assert.match(document.querySelector('figure[data-figure="worse"] svg').textContent, /static worse/);
  assert.equal(warnings.filter((w) => w.includes("bad")).length, 1);
  assert.equal(warnings.length, 2);
});

test("a_page_without_figures_loads_nothing", async () => {
  assert.deepEqual(await hydrateFigures(report(), { wasm: undefined }), []);
});

test("a_second_call_attaches_nothing_again", async () => {
  const grid = readFileSync(path.join(SPECS, "grid.json"), "utf8");
  const document = report(figure("a", grid));
  const first = await hydrateFigures(document, { wasm: wasm() });
  assert.equal(first.length, 1);
  assert.equal(document.querySelector("figure").getAttribute("data-hydrated"), "");
  assert.deepEqual(await hydrateFigures(document, { wasm: wasm() }), []);
  assert.equal(document.querySelectorAll("figure > svg").length, 1);
});

test("the_generated_page_script_hydrates_a_report", async (t) => {
  const python = path.join(ROOT, ".venv", "bin", "python");
  let text;
  try {
    text = execFileSync(python, ["-c", "from sbmlsim_core import web; print(web.figure_script())"], {
      encoding: "utf8",
      maxBuffer: 1 << 28,
    });
  } catch {
    t.skip("the python package is not installed in .venv");
    return;
  }
  const grid = readFileSync(path.join(SPECS, "grid.json"), "utf8");
  const { document, window } = parseHTML(
    `<!doctype html><html><head></head><body>${figure("a", grid)}</body></html>`,
  );
  window.requestAnimationFrame = () => 0;
  window.requestIdleCallback = () => 0;
  window.atob = (data) => Buffer.from(data, "base64").toString("latin1");
  const context = vm.createContext(window);
  context.console = console;
  context.URL = URL;
  context.TextDecoder = TextDecoder;
  context.TextEncoder = TextEncoder;
  context.WebAssembly = WebAssembly;
  context.Uint8Array = Uint8Array;
  vm.runInContext(text, context);
  for (let i = 0; i < 100 && !document.querySelector("figure[data-hydrated]"); i++) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  const svg = document.querySelector("figure svg");
  assert.ok(document.querySelector("figure[data-hydrated]"));
  assert.ok(svg.querySelector("[data-element]"));
  assert.ok(!svg.textContent.includes("static"));
});
