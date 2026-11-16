import assert from "node:assert/strict";
import path from "node:path";
import { before, test } from "node:test";
import { createRuntime, PreparedSimulation } from "../loader.mjs";
import { createPool } from "../pool.mjs";
import { DECAY, PANEL_MODEL, panelBundle, readModel, runtimeBytes, nodeWorker, request } from "./helpers.mjs";

let runtime, model;
const bits = ({ values }) => Array.from(new BigUint64Array(values.buffer, values.byteOffset, values.length));
before(async () => {
  runtime = await createRuntime(runtimeBytes());
  model = readModel(DECAY);
});

test("prepared requests preserve bits, changes, steady state, ownership and recovery", async () => {
  const instance = await runtime.instantiate(model.wasm, model.json);
  const other = await runtime.instantiate(model.wasm, model.json);
  for (const steady_state of [null, { preinit: [["k", 0.5]], absolute_tolerance: 1e-9, relative_tolerance: 1e-7, max_time: 1e6 }]) {
    const template = request({ changes: [{ time: 4, values: [["y", 2]] }], steady_state });
    const plan = instance.prepare(template, { varying: ["k", "y"] });
    assert.ok(plan instanceof PreparedSimulation);
    assert.equal(plan.nValues, 2);
    for (const values of [[0.25, 3], [0.9, 1], [0.25, 3]]) {
      const concrete = structuredClone(template);
      concrete.preinit = [["y", values[1]]];
      if (steady_state) concrete.steady_state.preinit[0][1] = values[0];
      else concrete.preinit.push(["k", values[0]]);
      assert.deepEqual(bits(instance.simulatePrepared(plan, values)), bits(other.simulate(concrete)));
    }
    for (const values of [[], [NaN, 1], [Infinity, 1], [1, -Infinity]]) assert.throws(() => instance.simulatePrepared(plan, values), /finite values/);
    assert.throws(() => other.simulatePrepared(plan, [1, 1]), /foreign/);
    assert.ok(instance.simulatePrepared(plan, [0.5, 1]).nRows >= 11);
    plan.close(); plan.close();
    assert.throws(() => instance.simulatePrepared(plan, [0.5, 1]), /closed/);
  }
  const fixed = instance.prepare(request());
  assert.equal(fixed.nValues, 0);
  assert.deepEqual(bits(instance.simulatePrepared(fixed)), bits(other.simulate(request())));
  fixed.close();
  const bounded = Array.from({ length: 32 }, () => instance.prepare(request()));
  assert.throws(() => instance.prepare(request()), /limit/);
  bounded.forEach((plan) => plan.close());
  instance.exports = { ...instance.exports, engine_prepare: undefined };
  assert.throws(() => instance.prepare(request()), (e) => e.name === "UnsupportedError" && e.kind === "unsupported");
  assert.equal(instance.simulate(request()).nRows, 11);
});

test("prepared concentration overrides preserve the template quantity", async () => {
  const m = readModel(path.join(path.dirname(panelBundle()), "models", PANEL_MODEL));
  const instance = await runtime.instantiate(m.wasm, m.json);
  const template = request({ preinit: [["[A]", 3]], selections: ["A", "[A]"] });
  const plan = instance.prepare(template, { varying: ["[A]"] });
  for (const value of [1, 5, 1]) {
    const actual = instance.simulatePrepared(plan, [value]);
    assert.deepEqual(bits(actual), bits(instance.simulate({ ...template, preinit: [["[A]", value]] })));
  }
  plan.close();
});

test("pool prepared vectors reuse plans across workers and model eviction", async () => {
  const messages = [];
  const createWorker = (url) => {
    const worker = nodeWorker(url);
    const post = worker.postMessage.bind(worker);
    worker.postMessage = (message, ...rest) => { messages.push(message); return post(message, ...rest); };
    return worker;
  };
  const pool = await createPool({ ...runtime.modules, workers: 2, maxWorkerModels: 1, createWorker });
  const foreign = await createPool({ ...runtime.modules, workers: 1, createWorker: nodeWorker });
  try {
    await pool.addModel("decay", model.wasm, model.json);
    await pool.addModel("other", model.wasm, model.json);
    const plan = await pool.prepare("decay", request(), { varying: ["k"] });
    assert.equal(plan.nValues, 1);
    const expected = await runtime.instantiate(model.wasm, model.json);
    const values = [0.25, 0.5, 1, 0.25];
    const results = await Promise.all(values.map((value) => pool.simulatePrepared(plan, [value])));
    results.forEach((result, i) => assert.deepEqual(bits(result), bits(expected.simulate(request({ preinit: [["k", values[i]]] })))));
    const callerValues = new Float64Array([0.25]);
    const snapshot = pool.simulatePrepared(plan, callerValues);
    callerValues[0] = 0.9;
    assert.deepEqual(bits(await snapshot), bits(expected.simulate(request({ preinit: [["k", 0.25]] }))));
    assert.equal(callerValues[0], 0.9);
    assert.equal(callerValues.byteLength, 8);
    await pool.simulatePrepared(plan, [0.5]);
    const raw = messages.filter((message) => message.type === "simulatePrepared");
    assert.ok(raw.some((message) => message.request === undefined && message.values instanceof Float64Array));
    await Promise.all([pool.simulate("other", request()), pool.simulate("other", request())]);
    assert.deepEqual(bits(await pool.simulatePrepared(plan, [0.5])), bits(expected.simulate(request())));
    await assert.rejects(foreign.simulatePrepared(plan, [0.5]), /foreign/);
    await assert.rejects(pool.simulatePrepared(plan, [NaN]), /finite/);
    await assert.rejects(pool.simulatePrepared(plan, []), /finite/);
    const controller = new AbortController(); controller.abort();
    await assert.rejects(pool.simulatePrepared(plan, [0.5], { signal: controller.signal }), /abort/i);
    plan.close(); plan.close();
    await assert.rejects(pool.simulatePrepared(plan, [0.5]), /closed/);
    const stale = await pool.prepare("decay", request());
    assert.equal(pool.removeModel("decay"), true);
    await pool.addModel("decay", model.wasm, model.json);
    await assert.rejects(pool.simulatePrepared(stale), /closed/);
  } finally { pool.close(); foreign.close(); }
});
