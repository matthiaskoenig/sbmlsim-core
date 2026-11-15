// The panel endpoint (`panels.mjs`) on the bundle of `fixtures/panel-bundle`: the routes, the
// checks of a simulation, the grid in C order, the shared runs, the abort of a request, a point
// which does not integrate, and the curves with the bits of a simulation on the main thread.
//
// The model of the bundle (`fixtures/panel-bundle/models/*/model.xml`) is compiled by the command
// line when the tests start. Its knobs: `dose`, a range knob in g whose target `dose` is in mg (a
// factor of 1000); `renal`, a range knob without a target, whose parameter `panel_knob__renal`
// the initial assignment of the derived target `k` reads (0 does not integrate); `speed`, a
// choice knob with the target `kbase` and the parameter `panel_knob__speed`. The curve `[A]` is
// delivered in µM (a factor of 1000), `B` as it is; `x` is the time in hours.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { after, before, test } from "node:test";

import { createRuntime } from "../loader.mjs";
import {
  CORE_VERSION,
  ENGINE,
  canonicalJson,
  createPanelEndpoint,
  knobValues,
  sha256Hex,
  sha256HexFallback,
} from "../panels.mjs";
import { createPool } from "../pool.mjs";
import { PANEL_MODEL, ROOT, fileFetch, nodeWorker, panelBundle, readModel, runtimeBytes, runtimeUrls } from "./helpers.mjs";

const BASE = "local:toy";

let bundleFile;
let bundle;
let endpoint;
let instance;
const closing = [];

/** An endpoint of the bundle of the tests on worker threads; closed after the tests. */
function create(options = {}) {
  const created = createPanelEndpoint({
    bundle: pathToFileURL(bundleFile),
    runtime: runtimeUrls(),
    workers: 2,
    createWorker: nodeWorker,
    fetch: fileFetch,
    ...options,
  });
  closing.push(created);
  return created;
}

/** The status and the parsed body of a request of an endpoint. */
async function call(fetcher, route, init = {}) {
  const response = await fetcher(`${BASE}${route}`, init);
  assert.equal(response.headers.get("content-type"), "application/json");
  return { status: response.status, body: await response.json(), response };
}

/** A simulation of a panel. */
function simulate(fetcher, panel, body = {}, init = {}) {
  return call(fetcher, `/api/models/toy/panels/${encodeURIComponent(panel)}/simulate`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    ...init,
  });
}

/** The pre-initialization of the bundle at the values of the knobs, sorted by id. */
function preinit({ dose = 0.01, renal = 100, speed = 0.1 } = {}) {
  return [
    ["c", 1.5],
    ["dose", dose * 1000],
    ["kbase", speed],
    ["panel_knob__renal", renal],
    ["panel_knob__speed", speed],
  ];
}

/** The settings of the model of the bundle with its tolerance per state (`A`, then `B`). */
function settings(absolute = { per_state: [1e-14, 1e-10] }) {
  return { ...bundle.models[PANEL_MODEL].settings, absolute_tolerance: absolute };
}

/** A simulation of the bundle on the main thread. */
function mainThread(knobs, absolute) {
  return instance.simulate({
    contract: "simulation/1",
    start: 0,
    end: 30,
    output: { uniform: { steps: 6 } },
    preinit: preinit(knobs),
    changes: [],
    steady_state: null,
    selections: ["B", "[A]"],
    settings: settings(absolute),
  });
}

/** The column of a result, times a factor. */
function column(result, name, factor = 1) {
  const j = result.columns.indexOf(name);
  return Array.from({ length: result.nRows }, (_, r) => result.values[r * result.columns.length + j] * factor);
}

/** The bits of numbers. */
const bits = (numbers) => Array.from(new BigUint64Array(new Float64Array(numbers).buffer));

/**
 * A pool which counts the simulations it is asked for and keeps their signals; `hold()` keeps
 * them from starting until `release()`.
 */
async function countingPool() {
  const pool = await createPool({ ...runtimeBytes(), workers: 2, createWorker: nodeWorker });
  const calls = [];
  let gate = Promise.resolve();
  let open = () => {};
  return {
    calls,
    hold() {
      gate = new Promise((resolve) => {
        open = resolve;
      });
    },
    release: () => open(),
    pool: {
      addModel: (...args) => pool.addModel(...args),
      hasModel: (key) => pool.hasModel(key),
      async simulate(key, request, options) {
        calls.push({ request, signal: options?.signal });
        await gate;
        return pool.simulate(key, request, options);
      },
      close: () => pool.close(),
    },
  };
}

/** Waits for the timers and the I/O of a few milliseconds. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 20));

before(async () => {
  bundleFile = panelBundle();
  bundle = JSON.parse(readFileSync(bundleFile, "utf8"));
  endpoint = create();
  const { wasm, json } = readModel(path.join(path.dirname(bundleFile), "models", PANEL_MODEL));
  const runtime = await createRuntime(runtimeBytes());
  instance = await runtime.instantiate(wasm, json);
});

after(() => {
  for (const created of closing) created.close();
});

test("the version of the engine is the one of the package", () => {
  const pkg = JSON.parse(readFileSync(path.join(ROOT, "js", "package.json"), "utf8"));
  assert.equal(CORE_VERSION, pkg.version);
  assert.equal(ENGINE, `sbmlsim-core ${pkg.version}`);
});

test("the values of a knob are the ones of Knob.values of sbmlsim", () => {
  const [dose, renal, speed] = bundle.answers[""].panels.amount.declaration.axes;
  assert.deepEqual(knobValues(renal, 4), [0, 50, 100, 150]);
  assert.deepEqual(knobValues(renal, 1), [0]);
  assert.deepEqual(knobValues(renal, 2), [0, 150]);
  assert.deepEqual(knobValues(dose), [0.005, 0.01, 0.015, 0.02]);
  // the weighted form keeps a decimal grid exact
  assert.deepEqual(knobValues({ kind: "range", min: 0.2, max: 1.0 }, 5), [0.2, 0.4, 0.6, 0.8, 1.0]);
  assert.deepEqual(knobValues(speed, 7), [0.05, 0.1, 0.2]);
  assert.throws(() => knobValues(renal, 0), /needs a point/);
});

test("the canonical JSON has sorted keys and ASCII only, as the one of sbmlsim, and its sha256", async () => {
  assert.equal(canonicalJson({ b: [1, 0.5, null], a: { d: "µM", c: true } }), '{"a":{"c":true,"d":"\\u00b5M"},"b":[1,0.5,null]}');
  // json.dumps of Python escapes a character outside the BMP as its surrogates
  assert.equal(canonicalJson("\u{1F600}"), '"\\ud83d\\ude00"');
  assert.equal(canonicalJson({ a: undefined, b: 1 }), '{"b":1}');
  assert.equal(await sha256Hex(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
  assert.equal(await sha256Hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  for (const text of ["", "abc", "µM", "x".repeat(55), "x".repeat(56), "x".repeat(64), "y".repeat(1000)]) {
    assert.equal(sha256HexFallback(text), await sha256Hex(text), `${text.length} characters`);
  }
});

test("the app, the declarations and the data of the bundle", async () => {
  const answers = bundle.answers;
  for (const query of ["", "?variant=default"]) {
    const variant = query ? "default" : "";
    const app = await call(endpoint, `/api/models/toy/app${query}`);
    assert.equal(app.status, 200);
    assert.deepEqual(app.body, answers[variant].app);
    for (const panel of ["amount", "conc"]) {
      const declaration = await call(endpoint, `/api/models/toy/panels/${panel}${query}`);
      assert.equal(declaration.status, 200);
      assert.deepEqual(declaration.body, answers[variant].panels[panel].declaration);
      const data = await call(endpoint, `/api/models/toy/panels/${panel}/data${query}`);
      assert.equal(data.status, 200);
      assert.deepEqual(data.body, answers[variant].panels[panel].data);
    }
  }
  // any URL whose path has /api/, a Request, and an id which is encoded
  const https = await endpoint("https://models.example.org/sub/api/models/toy/panels/amount?x=1#top", {});
  assert.equal((await https.json()).panel.id, "amount");
  const request = await endpoint(new Request("https://models.example.org/api/models/toy/panels/%61mount/data"));
  assert.equal((await request.json()).contract, "paneldata/1");
});

test("the errors of the routes", async () => {
  const expect = async (route, status, detail, init = {}) => {
    const { status: got, body, response } = await call(endpoint, route, init);
    assert.equal(got, status, route);
    assert.equal(typeof body.detail, "string");
    assert.match(body.detail, detail, route);
    return response;
  };
  await expect("/api/models/toy", 404, /^Not Found$/);
  await expect("/api/models/toy/panels/amount/other", 404, /^Not Found$/);
  await expect("/other", 404, /^Not Found$/);
  await expect("/api/models/other/app", 404, /'other' is no model of this service/);
  await expect("/api/models/toy/panels/none", 404, /'none' is no panel of the model 'toy'\.$/);
  await expect("/api/models/toy/panels/none/data?variant=default", 404, /in its variant 'default'/);
  await expect("/api/models/toy/app?variant=female", 422, /no variant 'female'; its variants are \['default'\]/);
  await expect("/api/models/toy/panels/amount?variant=female", 422, /no variant 'female'/);
  await expect(`/api/models/toy/app?variant=${"v".repeat(129)}`, 422, /at most 128 characters/);
  const post = await expect("/api/models/toy/app", 405, /^Method Not Allowed$/, { method: "POST", body: "{}" });
  assert.equal(post.headers.get("allow"), "GET");
  const get = await expect("/api/models/toy/panels/amount/simulate", 405, /^Method Not Allowed$/);
  assert.equal(get.headers.get("allow"), "POST");
  // a model without an app
  const appless = create({
    fetch: async (url) => {
      if (!String(url).endsWith("bundle.json")) return fileFetch(url);
      const data = structuredClone(bundle);
      for (const answer of Object.values(data.answers)) answer.app = null;
      return new Response(JSON.stringify(data));
    },
  });
  const none = await call(appless, "/api/models/toy/app");
  assert.equal(none.status, 404);
  assert.match(none.body.detail, /declares no app/);
  // a bundle which does not load, and loads at the next request when it is there
  let present = false;
  const later = create({ fetch: (url) => (present ? fileFetch(url) : Promise.resolve(new Response("", { status: 404 }))) });
  const missing = await call(later, "/api/models/toy/app");
  assert.equal(missing.status, 500);
  assert.match(missing.body.detail, /could not be loaded: .*answers 404/);
  present = true;
  assert.equal((await call(later, "/api/models/toy/app")).status, 200);
  // a bundle of another contract
  const other = create({ fetch: async () => new Response(JSON.stringify({ contract: "panelbundle/2" })) });
  assert.match((await call(other, "/api/models/toy/app")).body.detail, /of the contract panelbundle\/2, not panelbundle\/1/);
});

test("a simulation at the defaults has the bits of the main thread", async () => {
  const { status, body } = await simulate(endpoint, "amount");
  assert.equal(status, 200);
  const run = bundle.answers[""].panels.amount.run;
  assert.equal(body.contract, "panel/1");
  assert.deepEqual(body.x, run.x);
  assert.deepEqual(body.dims, []);
  assert.deepEqual(body.units, { x: "hr", B: "mmol" });
  assert.deepEqual(body.status, { ok: true, points: 1, failed: [] });
  assert.deepEqual(Object.keys(body.curves), ["B"]);
  assert.equal(body.curves.B.length, 1);
  const expected = mainThread({});
  assert.deepEqual(bits(body.curves.B[0]), bits(column(expected, "B")));
  // the tolerance per state is the one the endpoint gives the engine: the scalar one differs
  assert.notDeepEqual(bits(column(mainThread({}, { scalar: 1e-10 }), "B")), bits(column(expected, "B")));
  const { provenance } = body;
  assert.deepEqual(
    { ...provenance, seconds: 0, created: "" },
    { model: "toy", panel: "amount", model_sha256: run.model_sha256, variant: "default", engine: ENGINE, sbmlsim: "0.24.1.dev0", seconds: 0, created: "" },
  );
  assert.ok(provenance.seconds >= 0);
  assert.ok(!Number.isNaN(Date.parse(provenance.created)));
  assert.match(body.spec_hash, /^[0-9a-f]{64}$/);
  // the curve of [A] in µM
  const conc = await simulate(endpoint, "conc", { values: { dose: 0.02 } });
  assert.deepEqual(conc.body.units, { x: "hr", "[A]": "µM" });
  assert.deepEqual(bits(conc.body.curves["[A]"][0]), bits(column(mainThread({ dose: 0.02 }), "[A]", 1000)));
});

test("the spec hash is the one of the resolved request", async () => {
  const hash = async (body, panel = "amount") => (await simulate(endpoint, panel, body)).body.spec_hash;
  const defaults = await hash({});
  assert.equal(await hash({ values: { dose: 0.01, renal: 100, speed: 0.1 } }), defaults);
  assert.notEqual(await hash({ variant: "default" }), defaults);
  assert.notEqual(await hash({ values: { dose: 0.02 } }), defaults);
  assert.notEqual(await hash({}, "conc"), defaults);
  // a value of a knob of the grid is not part of it
  assert.equal(await hash({ grid: { dose: 2 }, values: { dose: 0.02 } }), await hash({ grid: { dose: 2 } }));
});

test("a grid over two knobs spans their product in C order", async () => {
  const { status, body } = await simulate(endpoint, "amount", { grid: { speed: 3, dose: 2 }, values: { renal: 50 } });
  assert.equal(status, 200);
  // the axes in the order of the knobs of the declaration, whatever the order of the body
  assert.deepEqual(body.dims, [
    { id: "dose", label: "Dose", unit: "g", values: [0.005, 0.02] },
    { id: "speed", label: "Speed", unit: "1/min", values: [0.05, 0.1, 0.2] },
  ]);
  assert.deepEqual(body.status, { ok: true, points: 6, failed: [] });
  assert.equal(body.curves.B.length, 6);
  let index = 0;
  for (const dose of [0.005, 0.02]) {
    for (const speed of [0.05, 0.1, 0.2]) {
      const expected = column(mainThread({ dose, speed, renal: 50 }), "B");
      assert.deepEqual(bits(body.curves.B[index]), bits(expected), `point ${index}`);
      index += 1;
    }
  }
});

test("a point which does not integrate is a row of null", async () => {
  const { status, body } = await simulate(endpoint, "conc", { grid: { dose: 2, renal: 4 } });
  assert.equal(status, 200);
  assert.deepEqual(body.dims[1].values, [0, 50, 100, 150]);
  assert.deepEqual(body.status, { ok: false, points: 8, failed: [0, 4] });
  for (const [i, row] of body.curves["[A]"].entries()) {
    if (i % 4 === 0) assert.deepEqual(row, Array(7).fill(null));
    else assert.ok(row.every((value) => typeof value === "number"), `point ${i}`);
  }
  const single = await simulate(endpoint, "conc", { values: { renal: 0 } });
  assert.deepEqual(single.body.status, { ok: false, points: 1, failed: [0] });
});

test("a simulation is checked as the service checks it", async () => {
  const refused = async (body, detail, panel = "amount") => {
    const answer = await call(endpoint, `/api/models/toy/panels/${panel}/simulate`, { method: "POST", body: typeof body === "string" ? body : JSON.stringify(body) });
    assert.equal(answer.status, 422, JSON.stringify(body));
    assert.match(answer.body.detail, detail, JSON.stringify(body));
  };
  await refused({ values: { volume: 1 } }, /^\['volume'\] are no knobs of the panel 'amount', which has \['dose', 'renal', 'speed'\]\.$/);
  await refused({ grid: { volume: 2 } }, /\['volume'\] are no knobs/);
  await refused({ values: { dose: 0.5 } }, /The value 0.5 of the knob 'dose' of the panel 'amount' is outside the bounds \[0.005, 0.02\]/);
  await refused({ values: { speed: 0.3 } }, /is none of the choices \[0.05, 0.1, 0.2\]/);
  await refused({ grid: { speed: 2 } }, /A grid of 2 points over the knob 'speed' .* runs its 3 choices/);
  await refused({ grid: { dose: 0 } }, /no whole number of at least 1/);
  await refused({ grid: { dose: 1.5 } }, /no whole number/);
  await refused({ grid: { dose: 65 } }, /over the limit of 64 points/);
  await refused({ grid: { dose: 8, renal: 9 } }, /The grid of 72 points is over the limit of 64 points/);
  await refused({ values: { dose: "1" } }, /no finite number/);
  await refused({ values: [] }, /no mapping of knob ids/);
  await refused({ values: Object.fromEntries(Array.from({ length: 9 }, (_, i) => [`k${i}`, 1])) }, /names 9 knobs, over the limit of 8/);
  await refused({ values: { ["k".repeat(129)]: 1 } }, /longer than 128 characters/);
  await refused({ points: 3 }, /unknown fields \['points'\]/);
  await refused({ variant: "female" }, /no variant 'female'/);
  await refused({ variant: 3 }, /no text/);
  await refused("{", /no JSON/);
  await refused("[]", /no JSON object/);
  // a value of a range knob between its stops, and a renal function at a named stop
  assert.equal((await simulate(endpoint, "amount", { values: { renal: 37.5 } })).status, 200);
  assert.equal((await simulate(endpoint, "amount", { values: { renal: 30 } })).status, 200);
  const unknownPanel = await simulate(endpoint, "none");
  assert.equal(unknownPanel.status, 404);
});

test("the panels of one state share one run", async () => {
  const { pool, calls } = await countingPool();
  const shared = create({ pool });
  closing.push(pool);
  // two panels at once: one run in flight
  const [amount, conc] = await Promise.all([
    simulate(shared, "amount", { grid: { dose: 2 } }),
    simulate(shared, "conc", { grid: { dose: 2 } }),
  ]);
  assert.equal(amount.status, 200);
  assert.equal(conc.status, 200);
  assert.equal(calls.length, 2);
  // the run of the selections of the model, sorted by id, with the tolerance per state
  assert.deepEqual(calls[0].request.selections, ["B", "[A]"]);
  assert.deepEqual(calls[0].request.preinit, preinit({ dose: 0.005 }));
  assert.deepEqual(calls[0].request.settings.absolute_tolerance, { per_state: [1e-14, 1e-10] });
  assert.deepEqual(calls[0].request.output, { uniform: { steps: 6 } });
  // a run which finished is kept: the same state again, also by its explicit values
  await simulate(shared, "conc", { grid: { dose: 2 }, values: { renal: 100 } });
  assert.equal(calls.length, 2);
  // another state is another run
  await simulate(shared, "amount", { values: { dose: 0.02 } });
  assert.equal(calls.length, 3);
  // the curves of the panels are the ones of their own runs
  assert.deepEqual(bits(amount.body.curves.B[1]), bits(column(mainThread({ dose: 0.02 }), "B")));
  // closing the endpoint leaves a pool it was given open
  shared.close();
  assert.equal((await simulate(shared, "amount", { values: { dose: 0.015 } })).status, 200);
});

test("a request which aborts is rejected and its run is dropped when nobody waits for it", async () => {
  const { pool, calls, hold, release } = await countingPool();
  const shared = create({ pool });
  closing.push(pool);
  await simulate(shared, "amount"); // loads the model
  // one request which aborts alone: its points are dropped
  calls.length = 0;
  hold();
  const controller = new AbortController();
  const aborted = simulate(shared, "amount", { grid: { dose: 8, renal: 8 } }, { signal: controller.signal });
  await settle();
  assert.equal(calls.length, 64);
  controller.abort();
  await assert.rejects(aborted, { name: "AbortError" });
  assert.ok(calls.every((c) => c.signal.aborted));
  release();
  // the same state again is a new run
  calls.length = 0;
  const again = await simulate(shared, "amount", { grid: { dose: 8, renal: 8 } });
  assert.equal(again.body.status.points, 64);
  assert.deepEqual(again.body.status.failed, [0, 8, 16, 24, 32, 40, 48, 56]);
  assert.equal(calls.length, 64);
  assert.ok(calls.every((c) => !c.signal.aborted));
  // two requests of one run, one aborts: the other gets its answer
  calls.length = 0;
  hold();
  const first = new AbortController();
  const waiting = simulate(shared, "amount", { grid: { dose: 4, renal: 8 } }, { signal: first.signal });
  const staying = simulate(shared, "conc", { grid: { dose: 4, renal: 8 } });
  await settle();
  first.abort(new Error("superseded"));
  await assert.rejects(waiting, /superseded/);
  release();
  assert.equal((await staying).body.status.points, 32);
  assert.equal(calls.length, 32);
  assert.ok(calls.every((c) => !c.signal.aborted));
  // a signal which aborted before
  await assert.rejects(simulate(shared, "amount", {}, { signal: AbortSignal.abort() }), { name: "AbortError" });
});
