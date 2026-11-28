// The scan of sbmlsim in the JavaScript host (`scan.mjs`): the planner of `compiler.wasm` driven
// ask/tell, its simulations run by the loader or the pool.
//
// Every oracle fixture of `testdata/scan` (written by sbmlsim from its own runs of the engine
// core) runs end to end: the request of the fixture, the model compiled again from its SBML by
// `compiler.wasm`, the simulations on the engine of the page and the observables of the host in
// JavaScript, compared with the `result` of the fixture under the rule of
// `crates/scan/tests/fixtures.rs`: the native values by their bits, the values interpolated
// onto a grid and the observables within 4 units in the last place, `NaN` equal to `NaN`.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { createCompiler } from "../compile.mjs";
import { createPool } from "../pool.mjs";
import { SCAN_CONTRACT, SCAN_RESULT_CONTRACT, ScanError, openScan, runScan } from "../scan.mjs";
import { ROOT, compilerWasm, loaderRuntime, nodeWorker, runtimeBytes } from "./helpers.mjs";

const TESTDATA = path.join(ROOT, "testdata", "scan");
const CASES = readdirSync(TESTDATA)
  .filter((name) => name.endsWith(".json"))
  .map((name) => name.slice(0, -5))
  .sort();

/** The units in the last place a derived value (a grid, an observable) may differ by. */
const ULPS = 4;

let compilerPromise = null;
/** The compiler of the tests, one instance of `compiler.wasm`. */
function compiler() {
  compilerPromise ??= createCompiler({ compiler: compilerWasm() });
  return compilerPromise;
}

let runtimePromise = null;
/** The runtime of the loader of the tests. */
function runtime() {
  runtimePromise ??= loaderRuntime();
  return runtimePromise;
}

const models = new Map();
/** The model of a fixture, compiled again from its SBML once. */
async function model(name) {
  if (!models.has(name)) {
    const sbml = readFileSync(path.join(TESTDATA, "models", name), "utf8");
    models.set(name, (await compiler()).compile(sbml));
  }
  return models.get(name);
}

/** The fixture of a case. */
function fixture(name) {
  return JSON.parse(readFileSync(path.join(TESTDATA, `${name}.json`), "utf8"));
}

/** The sum of numpy over a one dimensional array (`np.add.reduce`), to the bit. */
function numpySum(values) {
  const pairwise = (a) => {
    const n = a.length;
    if (n < 8) {
      let res = -0.0;
      for (const x of a) res += x;
      return res;
    }
    if (n <= 128) {
      const r = a.slice(0, 8);
      let i = 8;
      for (; i < n - (n % 8); i += 8) for (let j = 0; j < 8; j += 1) r[j] += a[i + j];
      let res = r[0] + r[1] + (r[2] + r[3]) + (r[4] + r[5] + (r[6] + r[7]));
      for (; i < n; i += 1) res += a[i];
      return res;
    }
    let n2 = Math.floor(n / 2);
    n2 -= n2 % 8;
    return pairwise(a.slice(0, n2)) + pairwise(a.slice(n2));
  };
  if (values.length === 0) return 0;
  return values[0] + pairwise(Array.from(values.slice(1)));
}

/** The functions of the host of the fixtures (`scripts/scan_fixtures.py` of sbmlsim). */
const FUNCTIONS = {
  // np.trapezoid(values["[C]"], time)
  "scripts.scan_fixtures:auc_of_c": (time, columns) => {
    const c = columns["[C]"];
    const pieces = [];
    for (let i = 1; i < time.length; i += 1) pieces.push(((time[i] - time[i - 1]) * (c[i] + c[i - 1])) / 2);
    return numpySum(pieces);
  },
  // 2.0 * values["[C]"]
  "scripts.scan_fixtures:doubled_c": (time, columns) => Float64Array.from(columns["[C]"], (x) => 2 * x),
};

/** The host of the custom observables of a request, as sbmlsim calls them. */
function observer(request) {
  const functions = Object.fromEntries(
    request.observables.filter((o) => o.type === "Custom").map((o) => [o.id, FUNCTIONS[o.function]]),
  );
  return async (question) => {
    const values = {};
    for (const id of question.observables) {
      values[id] = question.points.map((point) => functions[id](point.time, point.columns));
    }
    return { values };
  };
}

/** A number of a fixture: `null` is NaN, `"inf"` and `"-inf"` the infinities. */
function number(value) {
  if (value === null) return NaN;
  if (value === "inf") return Infinity;
  if (value === "-inf") return -Infinity;
  return value;
}

const view = new DataView(new ArrayBuffer(8));
/** The next double above a positive finite one. */
function nextUp(x) {
  view.setFloat64(0, x);
  view.setBigUint64(0, view.getBigUint64(0) + 1n);
  return view.getFloat64(0);
}

/** The distance of two numbers in units in the last place of the larger, 0 for equal bits and two NaN. */
function ulps(a, b) {
  if (Object.is(a, b) || (Number.isNaN(a) && Number.isNaN(b))) return 0;
  if (!Number.isFinite(a) || !Number.isFinite(b)) return Infinity;
  const larger = Math.max(Math.abs(a), Math.abs(b));
  return Math.abs(a - b) / (nextUp(larger) - larger);
}

/** The largest distance of two arrays in ulps. */
function largest(values, want) {
  assert.equal(values.length, want.length);
  let worst = 0;
  for (let k = 0; k < values.length; k += 1) worst = Math.max(worst, ulps(values[k], number(want[k])));
  return worst;
}

/** Compares a result with the `result` of its fixture, see the module. */
function compare(name, result, expected, request) {
  const observed = request.observables.map((o) => o.id);
  const gridded = request.time !== null;
  const derived = (variable) => gridded || observed.some((id) => variable === id || variable.startsWith(`${id}.`));
  assert.equal(result.contract, SCAN_RESULT_CONTRACT);
  assert.deepEqual(
    result.sizes.map(({ name: n, size }) => [n, size]),
    expected.sizes,
  );
  const attrs = expected.attrs;
  assert.deepEqual(result.dims, attrs.dims);
  assert.deepEqual(
    result.coords.map((c) => c.name),
    expected.order.coords,
  );
  assert.deepEqual(
    result.variables.map((v) => v.name),
    expected.order.variables,
  );
  for (const coord of result.coords) {
    const want = expected.coords[coord.name];
    if (want === undefined) continue;
    assert.deepEqual(coord.dims, want.dims, coord.name);
    assert.equal(coord.unit, want.unit, coord.name);
    const [[kind, values]] = Object.entries(coord.values);
    if (kind === "numbers") {
      assert.ok(values instanceof Float64Array);
      assert.equal(largest(values, want.values), 0, coord.name);
    } else {
      assert.deepEqual(values, want.values, coord.name);
    }
  }
  for (const variable of result.variables) {
    const want = expected.variables[variable.name];
    if (want === undefined) continue;
    assert.ok(variable.values instanceof Float64Array, variable.name);
    assert.deepEqual(variable.dims, want.dims, variable.name);
    assert.deepEqual(variable.shape, want.shape, variable.name);
    assert.equal(variable.unit, want.unit, variable.name);
    const distance = largest(variable.values, want.values);
    const allowed = derived(variable.name) ? ULPS : 0;
    assert.ok(distance <= allowed, `${name}: ${variable.name} differs by ${distance} ulps`);
  }
  assert.deepEqual(result.units, Object.entries(attrs.units));
  assert.ok(result.changes instanceof Float64Array);
  assert.equal(largest(result.changes, attrs.changes), 0);
  const texts =
    result.errors === null ? undefined : result.errors.map((e) => (e.point ? `${e.point}: ${e.message}` : e.message));
  assert.deepEqual(texts, attrs.errors);
  if (attrs.block === undefined) {
    assert.equal(result.block, null);
  } else {
    const { start, stop, shape, dims, coords } = result.block;
    assert.deepEqual({ start, stop, shape, dims }, {
      start: attrs.block.start,
      stop: attrs.block.stop,
      shape: attrs.block.shape,
      dims: attrs.block.dims,
    });
    assert.deepEqual(coords, Object.entries(attrs.block.coords));
  }
}

/** Two results are the same, every array by its bits. */
function sameResult(a, b) {
  assert.deepEqual(
    a.variables.map((v) => v.name),
    b.variables.map((v) => v.name),
  );
  for (let k = 0; k < a.variables.length; k += 1) {
    const x = new Uint8Array(a.variables[k].values.buffer, a.variables[k].values.byteOffset, a.variables[k].values.byteLength);
    const y = new Uint8Array(b.variables[k].values.buffer, b.variables[k].values.byteOffset, b.variables[k].values.byteLength);
    assert.deepEqual(x, y, a.variables[k].name);
  }
  assert.deepEqual(a.errors, b.errors);
}

test("there is a fixture per case", () => {
  assert.equal(CASES.length, 25);
});

for (const name of CASES) {
  test(`fixture ${name}`, async () => {
    const f = fixture(name);
    const options = { compiler: await compiler(), runtime: await runtime(), model: await model(f.model), observe: observer(f.request) };
    if (f.error !== null) {
      await assert.rejects(runScan(f.request, options), (error) => {
        assert.ok(error instanceof ScanError);
        assert.equal(error.kind, "point");
        assert.equal(error.message, f.error.message);
        assert.equal(typeof error.point, "number");
        return true;
      });
      return;
    }
    const result = await runScan(f.request, options);
    compare(name, result, f.result, f.request);
  });
}

test("the workers of a pool change no bit", async () => {
  const results = [];
  for (const workers of [1, 4]) {
    const pool = await createPool({ ...runtimeBytes(), workers, createWorker: nodeWorker });
    try {
      for (const name of ["scan_2d_values", "custom_observable"]) {
        const f = fixture(name);
        results.push([
          name,
          await runScan(f.request, { compiler: await compiler(), pool, model: await model(f.model), observe: observer(f.request) }),
        ]);
      }
    } finally {
      pool.close();
    }
  }
  sameResult(results[0][1], results[2][1]);
  sameResult(results[1][1], results[3][1]);
  const f = fixture("scan_2d_values");
  sameResult(results[0][1], await runScan(f.request, { compiler: await compiler(), runtime: await runtime(), model: await model(f.model) }));
});

test("onBatch reports the points which ran", async () => {
  const f = fixture("points_block");
  const calls = [];
  await runScan(f.request, {
    compiler: await compiler(),
    runtime: await runtime(),
    model: await model(f.model),
    onBatch: (done, total) => calls.push([done, total]),
  });
  assert.ok(calls.length > 0);
  assert.deepEqual(calls.at(-1), [3, 3]);
});

test("ask and tell of an open scan is the run", async () => {
  const f = fixture("custom_observable");
  const loaded = await model(f.model);
  const scan = openScan(f.request, { compiler: await compiler(), model: loaded });
  const instance = await (await runtime()).instantiate(loaded.wasm, loaded.description);
  const observe = observer(f.request);
  try {
    for (;;) {
      const round = scan.ask();
      if (round.done) break;
      if (round.simulate) {
        assert.ok(round.simulate.every((r) => r.contract === "simulation/1"));
        scan.tell(round.simulate.map((r) => instance.simulate(r)));
      } else {
        const point = round.observe.points[0];
        assert.ok(point.time instanceof Float64Array);
        assert.ok(Object.values(point.columns).every((c) => c instanceof Float64Array));
        scan.tellObserved(await observe(round.observe));
      }
    }
    const result = scan.result();
    sameResult(result, await runScan(f.request, { compiler: await compiler(), runtime: await runtime(), model: loaded, observe }));
  } finally {
    scan.close();
  }
});

test("a request which does not fit the model is a ScanError of the kind request", async () => {
  const f = fixture("steps_plain");
  const request = structuredClone(f.request);
  assert.equal(request.contract, SCAN_CONTRACT);
  request.scan.dimensions = [
    { id: "d", kind: "values", labels: [0], values: { no_such_entity: [1] }, simulations: {}, models: {}, at: null, design: null, coordinates: {} },
  ];
  await assert.rejects(
    runScan(request, { compiler: await compiler(), runtime: await runtime(), model: await model(f.model) }),
    (error) => error instanceof ScanError && error.kind === "request" && error.point === null && error.message.includes("no_such_entity"),
  );
});

test("custom observables need an observe function", async () => {
  const f = fixture("custom_observable");
  await assert.rejects(
    runScan(f.request, { compiler: await compiler(), runtime: await runtime(), model: await model(f.model) }),
    (error) => error instanceof ScanError && error.kind === "request" && error.message.includes("observe"),
  );
});

test("a failed host function fails its point alone", async () => {
  const f = fixture("custom_observable");
  const request = { ...structuredClone(f.request), on_error: "flag" };
  const observe = observer(request);
  const result = await runScan(request, {
    compiler: await compiler(),
    runtime: await runtime(),
    model: await model(f.model),
    observe: async (question) => ({
      ...(await observe(question)),
      errors: [{ observable: question.observables[0], index: question.points[0].index, message: "ValueError: no" }],
    }),
  });
  const status = result.variables.find((v) => v.name === "status");
  assert.deepEqual(Array.from(status.values), [1, 0]);
  assert.ok(result.errors[0].message.endsWith(": ValueError: no"));
});

test("an error of the pool ends the round: runScan rejects once and no lane simulates after it", async () => {
  const f = fixture("scan_2d_values");
  const loaded = await model(f.model);
  const instance = await (await runtime()).instantiate(loaded.wasm, loaded.description);
  const events = [];
  const terminated = new Error("the pool is terminated");
  // a pool whose second simulation fails as a pool which was closed fails: no EngineError, so
  // no failure of a point; the first one answers after it
  const pool = {
    async addModel() {},
    async simulate(key, request) {
      const k = events.filter((e) => e === "simulate").length;
      events.push("simulate");
      await new Promise((resolve) => setTimeout(resolve, k === 0 ? 20 : 0));
      if (k === 1) throw terminated;
      return instance.simulate(request);
    },
    removeModel() {
      events.push("removeModel");
    },
  };
  let rejections = 0;
  await assert.rejects(
    runScan(f.request, { compiler: await compiler(), pool, model: loaded, poolWindow: 2 }).catch((error) => {
      rejections += 1;
      throw error;
    }),
    (error) => error === terminated,
  );
  // the lanes which were still running had their chance to take another request
  await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(rejections, 1);
  assert.deepEqual(events, ["simulate", "simulate", "removeModel"]);
});

/** The fold model compiled with a text of its SBML replaced. */
async function foldModel(old = "", replacement = "") {
  const sbml = readFileSync(path.join(TESTDATA, "models", "scan_fold.xml"), "utf8").replace(old, replacement);
  return (await compiler()).compile(sbml);
}

/** A scan of the fold model over `kb`, with a dimension of models of `keys` when given. */
function foldRequest(keys) {
  const request = structuredClone(fixture("failing_point_raise").request);
  const rate = { id: "rate", kind: "values", labels: [0, 1], values: { kb: [0, 0.1] }, simulations: {}, models: {}, at: null, design: null, coordinates: {} };
  request.scan.dimensions = [rate];
  if (keys !== undefined) {
    request.scan.dimensions.push({
      id: "model",
      kind: "models",
      labels: keys,
      values: {},
      simulations: {},
      models: Object.fromEntries(keys.map((key) => [key, "<sbml/>"])),
      at: null,
      design: null,
      coordinates: {},
    });
  }
  return request;
}

test("a dimension of models runs every model on its own engine, on a runtime and on a pool", async () => {
  const fast = await foldModel();
  const slow = await foldModel('id="k" value="0.2"', 'id="k" value="0.4"');
  const models = { slow, fast };
  const request = foldRequest(["fast", "slow"]);
  const result = await runScan(request, { compiler: await compiler(), runtime: await runtime(), models });
  assert.deepEqual(result.dims, ["rate", "model"]);
  for (const [m, key] of ["fast", "slow"].entries()) {
    const alone = await runScan(foldRequest(), { compiler: await compiler(), runtime: await runtime(), model: models[key] });
    for (const variable of alone.variables) {
      const all = result.variables.find((v) => v.name === variable.name);
      const n = variable.values.length / 2;
      for (const r of [0, 1]) {
        const got = all.values.slice((2 * r + m) * n, (2 * r + m + 1) * n);
        assert.deepEqual(Array.from(got), Array.from(variable.values.slice(r * n, (r + 1) * n)), `${variable.name} ${key}`);
      }
    }
  }
  const pool = await createPool({ ...runtimeBytes(), workers: 2, createWorker: nodeWorker });
  try {
    sameResult(result, await runScan(request, { compiler: await compiler(), pool, models }));
  } finally {
    pool.close();
  }
  const scan = openScan(request, { compiler: await compiler(), models });
  try {
    const round = scan.ask();
    assert.equal(round.model, "fast");
    assert.deepEqual(round.finished, []);
  } finally {
    scan.close();
  }
  // the dimension of models first: `fast` is finished at the first round of `slow`, and its
  // model is removed from the pool then, before the scan ends
  const first = structuredClone(request);
  first.scan.dimensions.reverse();
  const seen = [];
  const ordered = openScan(first, { compiler: await compiler(), models });
  try {
    const instances = {};
    for (;;) {
      const round = ordered.ask();
      if (round.done) break;
      seen.push([round.model, round.finished]);
      instances[round.model] ??= await (await runtime()).instantiate(models[round.model].wasm, models[round.model].description);
      ordered.tell(round.simulate.map((r) => instances[round.model].simulate(r)));
    }
  } finally {
    ordered.close();
  }
  assert.deepEqual(seen, [["fast", []], ["slow", ["fast"]]]);
  const removed = [];
  const watched = await createPool({ ...runtimeBytes(), workers: 2, createWorker: nodeWorker });
  try {
    const removeModel = watched.removeModel.bind(watched);
    watched.removeModel = (key) => {
      removed.push(key);
      return removeModel(key);
    };
    const result = await runScan(first, { compiler: await compiler(), pool: watched, models });
    assert.equal(result.dims.join(","), "model,rate");
  } finally {
    watched.close();
  }
  assert.equal(removed.length, 2);
  await assert.rejects(
    runScan(request, { compiler: await compiler(), runtime: await runtime(), model: fast, models }),
    (error) => error instanceof ScanError && error.kind === "request" && error.message.includes("replaces the model of the run"),
  );
});
