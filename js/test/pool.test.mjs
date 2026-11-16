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

test("structured clone failures release the slot and queued jobs recover", async () => {
  const p = await pool({ workers: 1 });
  await p.addModel("toy", model.module, model.json);
  const bad = p.simulate("toy", { ...toyRequest(1), bad: () => {} });
  const good = p.simulate("toy", toyRequest(1));
  await assert.rejects(bad, /cloned/);
  assert.equal((await good).nRows, 31);
});

test("normal Node worker exits reject once and replacement runs", async () => {
  const { Worker } = await import("node:worker_threads");
  let count = 0;
  const p = await pool({ workers: 1, createWorker: (url) => count++ === 0
    ? new Worker("process.exit(0)", { eval: true }) : nodeWorker(url) });
  await p.addModel("toy", model.module, model.json);
  const bad = p.simulate("toy", toyRequest(1));
  const good = p.simulate("toy", toyRequest(1));
  await assert.rejects(bad, /exited \(0\)/);
  assert.equal((await good).nRows, 31);
});

test("deadline terminates a busy worker and replacement runs", async () => {
  const { Worker } = await import("node:worker_threads");
  let count = 0;
  const p = await pool({ workers: 1, createWorker: (url) => count++ === 0
    ? new Worker("while (true) {}", { eval: true }) : nodeWorker(url) });
  await p.addModel("toy", model.module, model.json);
  const bad = p.simulate("toy", toyRequest(1), { deadlineMs: 50 });
  const good = p.simulate("toy", toyRequest(1));
  await assert.rejects(bad, { name: "TimeoutError" });
  assert.equal((await good).nRows, 31);
});

test("soft and hard abort reject falsy reasons and later work runs", async () => {
  for (const hardAbort of [false, true]) for (const reason of [null, false, 0, ""]) {
    const p = await pool({ workers: 1 });
    await p.addModel("toy", model.module, model.json);
    const ac = new AbortController();
    const bad = p.simulate("toy", toyRequest(1), { signal: ac.signal, hardAbort });
    ac.abort(reason);
    const outcome = await bad.then(() => ({ ok: true }), (error) => ({ error }));
    assert.deepEqual(outcome, { error: reason });
    assert.equal((await p.simulate("toy", toyRequest(1))).nRows, 31);
  }
});

test("worker creation and synchronous sends reject without wedging", async () => {
  for (const failingType of ["create", "init", "model", "simulate"]) {
    let count = 0;
    const p = await pool({ workers: 1, createWorker: (url) => {
      if (count++ !== 0) return nodeWorker(url);
      if (failingType === "create") throw new Error("injected send failure");
      const worker = nodeWorker(url);
      const send = worker.postMessage.bind(worker);
      worker.postMessage = (message, ...args) => {
        if (message.type === failingType) throw new Error("injected send failure");
        return send(message, ...args);
      };
      return worker;
    } });
    await p.addModel("toy", model.module, model.json);
    await assert.rejects(p.simulate("toy", toyRequest(1)), /injected send failure/);
    assert.equal((await p.simulate("toy", toyRequest(1))).nRows, 31);
  }
});

test("model caps, eviction, removal and queue backpressure", async () => {
  const p = await pool({ workers: 1, maxModels: 2, maxWorkerModels: 1 });
  await p.addModel("a", model.module, model.json);
  await p.addModel("b", model.module, model.json);
  await assert.rejects(p.addModel("c", model.module, model.json), /model limit/);
  for (const key of ["a", "b", "a"]) assert.equal((await p.simulate(key, toyRequest(1))).nRows, 31);
  assert.equal(p.removeModel("a"), true);
  assert.equal(p.removeModel("a"), false);
  await assert.rejects(p.simulate("a", toyRequest(1)), /no model/);
  await p.addModel("a", model.module, model.json);
  assert.equal((await p.simulate("a", toyRequest(1))).nRows, 31);
  const busy = await pool({ workers: 1, maxQueuedJobs: 1, createWorker: () => ({ on() {}, postMessage() {}, terminate() {} }) });
  await busy.addModel("toy", model.module, model.json);
  const first = busy.simulate("toy", toyRequest(1));
  const second = busy.simulate("toy", toyRequest(1));
  assert.throws(() => busy.removeModel("toy"), /in use/);
  await assert.rejects(busy.simulate("toy", toyRequest(1)), /queue limit/);
  const settled = Promise.allSettled([first, second]);
  busy.close();
  assert.ok((await settled).every((outcome) => outcome.status === "rejected"));
  await assert.rejects(pool({ workers: 33 }), /maxWorkers/);
});

test("an infinite WASM RHS is interrupted by the dispatch deadline", async () => {
  const { readFileSync } = await import("node:fs");
  const bytes = new Uint8Array(readFileSync(new URL("../../crates/host/tests/decay-model/model.wasm", import.meta.url)));
  const json = readFileSync(new URL("../../crates/host/tests/decay-model/model.json", import.meta.url), "utf8");
  let pos = 8;
  const leb = () => {
    let n = 0, shift = 0, byte;
    do { byte = bytes[pos++]; n += (byte & 127) * 2 ** shift; shift += 7; } while (byte & 128);
    return n;
  };
  const encode = (n) => {
    const out = [];
    do { const byte = n & 127; n >>>= 7; out.push(byte | (n ? 128 : 0)); } while (n);
    return out;
  };
  const imported = WebAssembly.Module.imports(new WebAssembly.Module(bytes)).filter((item) => item.kind === "function").length;
  let rhs;
  const output = [...bytes.subarray(0, 8)];
  while (pos < bytes.length) {
    const start = pos;
    const id = bytes[pos++];
    const length = leb();
    const end = pos + length;
    if (id === 7) {
      const count = leb();
      for (let i = 0; i < count; i++) {
        const size = leb();
        const name = new TextDecoder().decode(bytes.subarray(pos, pos + size));
        pos += size + 1; // name and export kind
        const index = leb();
        if (name === "model_rhs") rhs = index - imported;
      }
    }
    if (id === 10) {
      const count = leb();
      assert.ok(Number.isInteger(rhs));
      const body = [...encode(count)];
      for (let i = 0; i < count; i++) {
        const size = leb();
        // An empty-stack infinite loop is valid at every function signature.
        const code = i === rhs ? [0, 3, 0x40, 0x0c, 0, 0x0b, 0x0b] : bytes.subarray(pos, pos + size);
        body.push(...encode(code.length), ...code);
        pos += size;
      }
      output.push(id, ...encode(body.length), ...body);
    } else output.push(...bytes.subarray(start, end));
    pos = end;
  }
  const looping = Uint8Array.from(output);
  assert.ok(WebAssembly.validate(looping));
  const p = await pool({ workers: 1 });
  await p.addModel("loop", looping, json);
  await p.addModel("toy", model.module, model.json);
  const bad = p.simulate("loop", request(), { deadlineMs: 250 });
  const good = p.simulate("toy", toyRequest(1));
  await assert.rejects(bad, { name: "TimeoutError" });
  assert.equal((await good).nRows, 31);
});

test("the pool forwards its per-instance memory limit to workers", async () => {
  let forwarded;
  const p = await pool({ workers: 1, maxMemoryPages: 512, createWorker: (url) => {
    const worker = nodeWorker(url);
    const send = worker.postMessage.bind(worker);
    worker.postMessage = (message, ...args) => {
      if (message.type === "init") forwarded = message.maxMemoryPages;
      return send(message, ...args);
    };
    return worker;
  } });
  await p.addModel("toy", model.module, model.json);
  assert.equal((await p.simulate("toy", toyRequest(1))).nRows, 31);
  assert.equal(forwarded, 512);
  await assert.rejects(pool({ maxMemoryPages: 0 }), /maxMemoryPages/);
});

test("the aggregate linear-memory budget derives and validates per-instance limits", async () => {
  const defaults = await pool({ workers: 8 });
  assert.equal(defaults.maxTotalMemoryBytes, 2 ** 31);
  assert.equal(defaults.maxMemoryPages, 512); // 32 MiB each, 8 workers * 8 models
  const oneModel = await pool({ workers: 8, maxModels: 1 });
  assert.equal(oneModel.maxMemoryPages, 4096);
  await assert.rejects(pool({ workers: 8, maxMemoryPages: 1024 }), /exceeds maxTotalMemoryBytes/);
  await assert.rejects(pool({ workers: 8, maxTotalMemoryBytes: 65536 }), /engine minimum/);
  const larger = await pool({ workers: 8, maxMemoryPages: 1024, maxTotalMemoryBytes: 2 ** 32 });
  assert.equal(larger.maxMemoryPages, 1024);
  const derivedLarger = await pool({ workers: 8, maxTotalMemoryBytes: 2 ** 32 });
  assert.equal(derivedLarger.maxMemoryPages, 1024);
});

test("module, description and request byte limits reject before retaining jobs", async () => {
  const capped = await pool({ workers: 1, maxModuleBytes: model.wasm.byteLength - 1 });
  await assert.rejects(capped.addModel("toy", model.wasm, model.json), /maxModuleBytes/);
  assert.equal(capped.hasModel("toy"), false);
  const short = await pool({ workers: 1, maxDescriptionBytes: 4 });
  await assert.rejects(short.addModel("toy", model.module, "€€"), /description.*byte limit/);
  const req = toyRequest(1);
  const text = JSON.stringify(req);
  const bounded = await pool({ workers: 1, maxRequestBytes: Buffer.byteLength(text) });
  await bounded.addModel("toy", model.module, model.json);
  await assert.rejects(bounded.simulate("toy", `${text}€`), /request.*byte limit/);
  await assert.rejects(bounded.simulate("toy", { ...req, extra: "€" }), /request.*byte limit/);
  assert.equal((await bounded.simulate("toy", req)).nRows, 31);
  assert.equal((await bounded.simulate("toy", text)).nRows, 31);
  await assert.rejects(createPool({ ...runtimeBytes(), maxModuleBytes: 1 }), /maxModuleBytes/);
});

test("aggregate queued bytes recover after dequeue and abort", async () => {
  const req = toyRequest(1);
  const bytes = Buffer.byteLength(JSON.stringify(req));
  const p = await pool({ workers: 1, maxQueuedBytes: bytes });
  await p.addModel("toy", model.module, model.json);
  const first = p.simulate("toy", req);
  const ac = new AbortController();
  const aborted = p.simulate("toy", req, { signal: ac.signal });
  await assert.rejects(p.simulate("toy", req), /maxQueuedBytes/);
  ac.abort();
  await assert.rejects(aborted, { name: "AbortError" });
  const queued = p.simulate("toy", req);
  assert.equal((await first).nRows, 31);
  assert.equal((await queued).nRows, 31);
  assert.equal((await p.simulate("toy", req)).nRows, 31);
});

test("aggregate model bytes include descriptions and concurrent registration rechecks", async () => {
  const bytes = model.wasm.byteLength + Buffer.byteLength(model.json);
  const p = await pool({ maxModelBytes: bytes });
  await assert.rejects(p.addModel("invalid", {}, model.json), /WASM binary/);
  const results = await Promise.allSettled([
    p.addModel("a", model.wasm, model.json),
    p.addModel("b", model.wasm, model.json),
  ]);
  assert.equal(results.filter((item) => item.status === "fulfilled").length, 1);
  const failed = results.find((item) => item.status === "rejected");
  assert.match(failed.reason.message, /maxModelBytes/);
  const key = p.hasModel("a") ? "a" : "b";
  assert.equal(p.removeModel(key), true);
  await p.addModel("c", model.wasm, model.json);
  assert.equal(p.hasModel("c"), true);
  const trusted = await pool({ maxModelBytes: Buffer.byteLength(model.json) });
  await trusted.addModel("a", model.module, model.json);
  await assert.rejects(trusted.addModel("b", model.module, model.json), /maxModelBytes/);
  trusted.removeModel("a");
  await trusted.addModel("b", model.module, model.json);
});
