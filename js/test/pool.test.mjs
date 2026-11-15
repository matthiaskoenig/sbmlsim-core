// The pool of workers (`pool.mjs`) on worker threads of Node: the bits of the main thread, an
// error of the engine which rejects only its request, a queued request which an abort drops, and
// the close of a pool.

import assert from "node:assert/strict";
import path from "node:path";
import { after, before, test } from "node:test";

import { EngineError, createRuntime } from "../loader.mjs";
import { createPool, defaultWorkers } from "../pool.mjs";
import { PANEL_MODEL, nodeWorker, panelBundle, readModel, request, runtimeBytes } from "./helpers.mjs";

let model;
let runtime;
const pools = [];

/** A pool of worker threads; closed after the tests. */
async function pool(options = {}) {
  const created = await createPool({ ...runtime.modules, workers: 2, createWorker: nodeWorker, ...options });
  pools.push(created);
  return created;
}

/** A request of the model of the panel bundle at a dose and a renal function (0 does not integrate). */
function toyRequest(dose, renal = 100) {
  return request({
    end: 30,
    output: { uniform: { steps: 30 } },
    preinit: [
      ["dose", dose],
      ["panel_knob__renal", renal],
    ],
    selections: ["A", "B", "[A]", "k"],
    settings: { ...request().settings, relative_tolerance: 1e-10, absolute_tolerance: { per_state: [1e-14, 1e-12] } },
  });
}

before(async () => {
  const dir = path.join(path.dirname(panelBundle()), "models", PANEL_MODEL);
  model = readModel(dir);
  runtime = await createRuntime(runtimeBytes());
  model.module = await WebAssembly.compile(model.wasm);
});

after(() => {
  for (const p of pools) p.close();
});

/** The bits of the values of a result. */
const bits = (values) => Array.from(new BigUint64Array(values.buffer, values.byteOffset, values.length));

test("the pool gives the bits of the main thread", async () => {
  const p = await pool();
  await p.addModel("toy", model.module, model.json);
  const main = await runtime.instantiate(model.module, model.json);
  const doses = [1, 2, 5, 10, 20, 50];
  const results = await Promise.all(doses.map((dose) => p.simulate("toy", toyRequest(dose))));
  doses.forEach((dose, i) => {
    const expected = main.simulate(toyRequest(dose));
    assert.deepEqual(results[i].columns, expected.columns);
    assert.equal(results[i].nRows, 31);
    assert.ok(results[i].values instanceof Float64Array);
    assert.deepEqual(bits(results[i].values), bits(expected.values));
  });
  // a model given as bytes and a description as an object
  await p.addModel("bytes", model.wasm, JSON.parse(model.json));
  const again = await p.simulate("bytes", JSON.stringify(toyRequest(10)));
  assert.deepEqual(bits(again.values), bits(main.simulate(toyRequest(10)).values));
  await assert.rejects(p.addModel("toy", model.module, model.json), /has a model toy already/);
  await assert.rejects(p.simulate("none", toyRequest(1)), /has no model none/);
});

test("an error of the engine rejects only its request", async () => {
  const p = await pool({ workers: 1 });
  await p.addModel("toy", model.module, model.json);
  const [first, failed, last] = await Promise.allSettled([
    p.simulate("toy", toyRequest(1)),
    p.simulate("toy", toyRequest(1, 0)),
    p.simulate("toy", toyRequest(2)),
  ]);
  assert.equal(first.status, "fulfilled");
  assert.equal(last.status, "fulfilled");
  assert.equal(failed.status, "rejected");
  assert.ok(failed.reason instanceof EngineError);
  assert.equal(failed.reason.kind, "integration");
  assert.equal(failed.reason.flag, -4);
  assert.equal(failed.reason.time, 0);
  // the worker runs on
  assert.equal((await p.simulate("toy", toyRequest(3))).nRows, 31);
});

test("a queued request whose signal aborts is dropped, a running one is discarded", async () => {
  const posted = [];
  const counting = (url) => {
    const worker = nodeWorker(url);
    const postMessage = worker.postMessage.bind(worker);
    worker.postMessage = (message, transfer) => {
      if (message.type === "simulate") posted.push(message.id);
      return postMessage(message, transfer);
    };
    return worker;
  };
  const p = await pool({ workers: 1, createWorker: counting });
  await p.addModel("toy", model.module, model.json);
  const running = new AbortController();
  const queued = new AbortController();
  const first = p.simulate("toy", toyRequest(1), { signal: running.signal });
  const second = p.simulate("toy", toyRequest(2), { signal: queued.signal });
  const third = p.simulate("toy", toyRequest(3));
  queued.abort(new Error("not needed"));
  running.abort();
  await assert.rejects(second, /not needed/);
  await assert.rejects(first, { name: "AbortError" });
  assert.equal((await third).nRows, 31);
  // the first ran, the second never did
  assert.equal(posted.length, 2);
  // a signal which aborted before
  await assert.rejects(p.simulate("toy", toyRequest(1), { signal: AbortSignal.abort() }), { name: "AbortError" });
});

test("a closed pool rejects its requests and takes no more", async () => {
  const p = await pool({ workers: 1 });
  await p.addModel("toy", model.module, model.json);
  const pending = [p.simulate("toy", toyRequest(1)), p.simulate("toy", toyRequest(2))];
  p.close();
  for (const result of await Promise.allSettled(pending)) {
    assert.equal(result.status, "rejected");
    assert.match(result.reason.message, /the pool is closed/);
  }
  await assert.rejects(p.simulate("toy", toyRequest(1)), /the pool is closed/);
});

test("a pool takes at least one worker", async () => {
  await assert.rejects(createPool({ ...runtime.modules, workers: 0, createWorker: nodeWorker }), /at least one worker/);
  assert.ok(defaultWorkers() >= 1 && defaultWorkers() <= 8);
});
