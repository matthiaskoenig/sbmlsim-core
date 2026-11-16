// The dosing protocols of a panel (`run.dosing` of a bundle): the doses after the first one are
// increments of the changes of the request, at the times of `doseTimes`, and a dose at an output
// time leaves the state after it in the curve, as the service of sbmlsim interpolates it.
//
// The panel `repeat` is the panel `amount` of the bundle of the tests with a knob of the first
// dose `pulse` (on the species `A`) and a knob of the interval `every` (in minutes, 0 a single
// dose); the run ends at 30 min on 6 output steps, so the doses of an interval of 10 min fall on
// the output times 10 and 20.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { after, before, test } from "node:test";

import { createRuntime } from "../loader.mjs";
import { MAX_DOSES, createPanelEndpoint, doseTimes } from "../panels.mjs";
import { createPool } from "../pool.mjs";
import { PANEL_MODEL, nodeWorker, panelBundle, readModel, runtimeBytes } from "./helpers.mjs";

const BASE = "local:toy";

let files;
let instance;
let calls;
let endpoint;
let pool;

/** The bundle of the tests with the panel `repeat`, a dosing protocol of `A`. */
function withDosing(bundle) {
  const answers = bundle.answers[""];
  const amount = answers.panels.amount;
  const declaration = structuredClone(amount.declaration);
  declaration.panel.id = "repeat";
  declaration.axes.push(
    { id: "pulse", label: "pulse", target: "A", unit: null, kind: "range", min: 0, max: 2, steps: 3, choices: [], default: 1 },
    {
      id: "every",
      label: "every",
      target: null,
      unit: "min",
      kind: "choice",
      min: null,
      max: null,
      steps: 5,
      choices: [
        { label: "single", value: 0 },
        { label: "every 10 min", value: 10 },
      ],
      default: 10,
    },
  );
  const run = structuredClone(amount.run);
  run.x = Array.from({ length: 7 }, (_, i) => i * 5);
  run.knobs.pulse = { target: "A", factor: 1, parameter: null, unit: null };
  run.knobs.every = { target: null, factor: null, parameter: null, unit: "min" };
  run.dosing = [
    { target: "A", dose: "pulse", interval: "every", interval_factor: 1, duration: null, duration_factor: null },
  ];
  answers.panels.repeat = { declaration, data: amount.data, run };
  return bundle;
}

before(async () => {
  const bundleFile = panelBundle();
  const dir = path.join(path.dirname(bundleFile), "models", PANEL_MODEL);
  const bundle = withDosing(JSON.parse(readFileSync(bundleFile, "utf8")));
  files = {
    "bundle.json": JSON.stringify(bundle),
    [`models/${PANEL_MODEL}/model.wasm`]: readFileSync(path.join(dir, "model.wasm")),
    [`models/${PANEL_MODEL}/model.json`]: readFileSync(path.join(dir, "model.json")),
  };
  const { wasm, json } = readModel(dir);
  instance = await (await createRuntime(runtimeBytes())).instantiate(wasm, json);
  const real = await createPool({ ...runtimeBytes(), workers: 1, createWorker: nodeWorker });
  calls = [];
  pool = {
    addModel: (...args) => real.addModel(...args),
    hasModel: (key) => real.hasModel(key),
    async simulate(key, request, options) {
      calls.push(request);
      return real.simulate(key, request, options);
    },
    close: () => real.close(),
  };
  endpoint = createPanelEndpoint({
    bundle: "https://bundle.invalid/bundle.json",
    pool,
    fetch: async (url) => {
      const name = String(url).slice("https://bundle.invalid/".length);
      return name in files ? new Response(files[name]) : new Response("", { status: 404 });
    },
  });
});

after(() => {
  endpoint?.close();
  pool?.close();
});

/** A simulation of the panel `repeat`. */
async function simulate(body) {
  const response = await endpoint(`${BASE}/api/models/toy/panels/repeat/simulate`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return { status: response.status, body: await response.json() };
}

test("the times of the doses are the ones of sbmlsim", () => {
  assert.deepEqual(doseTimes(0, null, 100), []);
  assert.deepEqual(doseTimes(24, null, 72), [24, 48]);
  assert.deepEqual(doseTimes(24, 48, 100), [24]);
  assert.deepEqual(doseTimes(12, 48, 30), [12, 24]);
  assert.equal(doseTimes(1e-9, null, 1).length, MAX_DOSES);
});

test("the doses after the first one are increments of the changes", async () => {
  calls.length = 0;
  const { status, body } = await simulate({ values: { pulse: 1.5, every: 10 } });
  assert.equal(status, 200);
  assert.equal(calls.length, 1);
  const request = calls[0];
  assert.deepEqual(request.changes, [
    { time: 10, values: [], increments: [["A", 1.5]] },
    { time: 20, values: [], increments: [["A", 1.5]] },
  ]);
  assert.ok(request.preinit.some(([id, value]) => id === "A" && value === 1.5));
  // the engine has two rows at 10 and at 20; the curve has the one after the dose
  const direct = instance.simulate(request);
  assert.equal(direct.nRows, 9);
  const k = direct.columns.length;
  const b = direct.columns.indexOf("B");
  const after = [0, 1, 3, 4, 6, 7, 8].map((r) => direct.values[r * k + b]);
  assert.deepEqual(body.curves.B[0], after);
  assert.equal(body.curves.B[0].length, 7);
});

test("a single dose has no changes and a grid over the interval doses each point", async () => {
  calls.length = 0;
  const single = await simulate({ values: { pulse: 1.5, every: 0 } });
  assert.deepEqual(calls[0].changes, []);
  calls.length = 0;
  const grid = await simulate({ values: { pulse: 1.5 }, grid: { every: 2 } });
  assert.equal(grid.status, 200);
  assert.deepEqual(
    calls.map((request) => request.changes.length),
    [0, 2],
  );
  assert.deepEqual(grid.body.curves.B[0], single.body.curves.B[0]);
});
