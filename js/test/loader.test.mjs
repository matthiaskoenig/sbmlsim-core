// The loader: the CSV of the command line, the decay model as in wasmtime, the WASI stubs, the
// errors and the memory.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { EngineError, MAX_MEMORY_PAGES, WasiExit, formatNumber, toCsv, wasiImports } from "../loader.mjs";
import { DECAY, ROOT, loaderRuntime, readModel, request, runCli, runtimeDir } from "./helpers.mjs";

test("numbers are written as the CSV of the command line", () => {
  // the strings of Rust's `{}` of f64 (`sbmlsim_cli::csv`)
  const cases = [
    [0, "0"],
    [-0, "-0"],
    [1, "1"],
    [-2.5, "-2.5"],
    [0.1, "0.1"],
    [1 / 3, "0.3333333333333333"],
    [100, "100"],
    [1e21, "1000000000000000000000"],
    [1.5e-7, "0.00000015"],
    [123456.789, "123456.789"],
    [5e-324, `0.${"0".repeat(323)}5`],
    [1.7976931348623157e308, `17976931348623157${"0".repeat(292)}`],
    [NaN, "NaN"],
    [Infinity, "inf"],
    [-Infinity, "-inf"],
  ];
  for (const [x, text] of cases) assert.equal(formatNumber(x), text, String(x));
  // every string reads back as the same double
  for (let i = 0; i < 10000; i += 1) {
    const bits = new BigUint64Array(1);
    bits[0] = (BigInt(Math.floor(Math.random() * 2 ** 32)) << 32n) | BigInt(Math.floor(Math.random() * 2 ** 32));
    const x = new Float64Array(bits.buffer)[0];
    if (Number.isNaN(x)) continue;
    assert.ok(Object.is(Number(formatNumber(x)), x), `${x} ${formatNumber(x)}`);
  }
  assert.equal(toCsv({ columns: ["time", "y"], values: new Float64Array([0, 1, 1, 0.5]) }), "time,y\n0,1\n1,0.5\n");
});

test("the decay model gives the CSV of the wasmtime host byte for byte", async () => {
  const runtime = await loaderRuntime();
  const { wasm, json } = readModel(DECAY);
  const instance = await runtime.instantiate(wasm, json);
  const req = readFileSync(path.join(DECAY, "request.json"), "utf8");
  const csv = toCsv(instance.simulate(req));
  const expected = runCli(["simulate", DECAY, path.join(DECAY, "request.json")]).toString();
  assert.equal(csv, expected);
  // simulate.mjs writes the same
  const node = execFileSync(process.execPath, [path.join(ROOT, "js", "simulate.mjs"), DECAY, path.join(DECAY, "request.json")], {
    env: { ...process.env, SBMLSIM_CORE_RUNTIME: runtimeDir() },
  }).toString();
  assert.equal(node, expected);
  // a description as an object and a request as an object give the same
  const again = await runtime.instantiate(new WebAssembly.Module(wasm), JSON.parse(json));
  assert.equal(toCsv(again.simulate(JSON.parse(req))), expected);
});

test("the engine calls nothing of WASI while it loads and simulates", async () => {
  const runtime = await loaderRuntime();
  const { wasm, json } = readModel(DECAY);
  const instance = await runtime.instantiate(wasm, json);
  assert.equal(instance.initializeCalls, 0);
  assert.equal(instance.loadCalls, 0);
  for (let i = 0; i < 3; i += 1) instance.simulate(request({ selections: ["time", "y"] }));
  assert.throws(() => instance.simulate(request({ selections: ["nope"] })), EngineError);
  assert.equal(instance.wasiCalls(), 0);
});

test("the WASI stubs: fd_write to the console, the others throw", async () => {
  const lines = { log: [], error: [] };
  const out = { log: (l) => lines.log.push(l), error: (l) => lines.error.push(l) };
  const runtime = await loaderRuntime();
  const engine = runtime.modules.engine;
  const imported = WebAssembly.Module.imports(engine)
    .filter((i) => i.module === "wasi_snapshot_preview1")
    .map((i) => i.name)
    .sort();
  const memory = new WebAssembly.Memory({ initial: 1 });
  const wasi = wasiImports(engine, () => memory, out);
  assert.deepEqual(Object.keys(wasi.imports).sort(), imported);
  // fd_write of two iovecs, a line cut between them and an UTF-8 character cut between calls
  const bytes = new Uint8Array(memory.buffer);
  const view = new DataView(memory.buffer);
  const text = new TextEncoder().encode("hello\nwo");
  bytes.set(text, 100);
  bytes.set(new TextEncoder().encode("rld \u00e9\n"), 200);
  view.setUint32(0, 100, true);
  view.setUint32(4, text.length, true);
  view.setUint32(8, 200, true);
  view.setUint32(12, 4, true); // "rld " and then the rest
  assert.equal(wasi.imports.fd_write(1, 0, 2, 50), 0);
  assert.equal(view.getUint32(50, true), text.length + 4);
  view.setUint32(0, 204, true);
  view.setUint32(4, 1, true); // the first byte of the two of the e acute
  assert.equal(wasi.imports.fd_write(1, 0, 1, 50), 0);
  view.setUint32(0, 205, true);
  view.setUint32(4, 2, true);
  assert.equal(wasi.imports.fd_write(1, 0, 1, 50), 0);
  view.setUint32(0, 100, true);
  view.setUint32(4, 6, true);
  assert.equal(wasi.imports.fd_write(2, 0, 1, 50), 0);
  assert.equal(wasi.imports.fd_write(5, 0, 1, 50), 8);
  assert.deepEqual(lines, { log: ["hello", "world \u00e9"], error: ["hello"] });
  // an empty environment
  view.setUint32(60, 7, true);
  view.setUint32(64, 7, true);
  assert.equal(wasi.imports.environ_sizes_get(60, 64), 0);
  assert.deepEqual([view.getUint32(60, true), view.getUint32(64, true)], [0, 0]);
  assert.equal(wasi.imports.environ_get(0, 0), 0);
  // proc_exit, the clock and every other function throw
  assert.throws(() => wasi.imports.proc_exit(3), (e) => e instanceof WasiExit && e.code === 3);
  assert.throws(() => wasi.imports.clock_time_get(0, 0n, 0), /reads no clock/);
  for (const name of imported) {
    if (["fd_write", "proc_exit", "environ_get", "environ_sizes_get", "clock_time_get"].includes(name)) continue;
    assert.throws(() => wasi.imports[name](), /does not provide|draws no random/, name);
  }
  // 5 of fd_write, 4 of the other named ones, 1 of each other
  assert.equal(wasi.calls(), 9 + imported.length - 5);
});

test("errors of the engine are EngineErrors of their kind", async () => {
  const runtime = await loaderRuntime();
  const { wasm, json } = readModel(DECAY);
  const instance = await runtime.instantiate(wasm, json);
  const error = (req) => {
    try {
      instance.simulate(req);
    } catch (e) {
      return e;
    }
    assert.fail("no error");
  };
  const unknown = error(request({ selections: ["unknown"] }));
  assert.ok(unknown instanceof EngineError);
  assert.equal(unknown.kind, "request");
  assert.match(unknown.detail, /no selection unknown/);
  assert.equal(error("{").kind, "request");
  const steps = error(request({ end: 1e6, settings: { ...request().settings, maximum_num_steps: 2, stiff: false } }));
  assert.equal(steps.kind, "integration");
  assert.equal(typeof steps.flag, "number");
  // the instance simulates after an error, as a fresh one
  const fresh = await runtime.instantiate(wasm, json);
  const ok = request({ selections: ["time", "y"] });
  assert.deepEqual(instance.simulate(ok), fresh.simulate(ok));
  // a description the engine refuses
  const desc = JSON.parse(json);
  desc.states = [];
  await assert.rejects(runtime.instantiate(wasm, desc), /refuses the description of model decay/);
  await assert.rejects(runtime.instantiate(wasm, { ...JSON.parse(json), contract: "model/2" }), /contract model\/2/);
});

test("the output is aligned to 8 and the memory grows to at most 2 GiB", async () => {
  const runtime = await loaderRuntime();
  const { wasm, json } = readModel(DECAY);
  const instance = await runtime.instantiate(wasm, json);
  for (const selections of [["time"], ["time", "y"], ["y"], ["time", "y", "t_half"]]) {
    instance.simulate(request({ selections, output: { uniform: { steps: 7 } } }));
    assert.equal(instance.outputAddress() % 8, 0);
  }
  assert.equal(MAX_MEMORY_PAGES * 65536, 2 ** 31);
  assert.equal(instance.memory.buffer.byteLength % 65536, 0);
});

test("a result without memory is an error of the request", async () => {
  // a memory of at most 32 MiB: a result of 48 MB is refused by the engine (try_reserve), which
  // calls nothing of WASI and simulates the next request
  const runtime = await loaderRuntime({ maxMemoryPages: 512 });
  const { wasm, json } = readModel(DECAY);
  const instance = await runtime.instantiate(wasm, json);
  const small = request({ end: 1, selections: ["time", "y"], settings: { ...request().settings, stiff: false } });
  const first = instance.simulate(small);
  const big = { ...small, output: { uniform: { steps: 3_000_000 } } };
  assert.throws(
    () => instance.simulate(big),
    (e) => e instanceof EngineError && e.kind === "request" && /does not fit into the memory/.test(e.detail),
  );
  const steps = {
    ...small,
    end: 1e7,
    output: "integrator_steps",
    settings: { ...small.settings, maximum_time_step: 1, maximum_num_steps: 100_000_000 },
  };
  assert.throws(
    () => instance.simulate(steps),
    (e) => e instanceof EngineError && e.kind === "request" && /does not fit into the memory/.test(e.detail),
  );
  assert.equal(instance.wasiCalls(), 0);
  assert.ok(instance.memoryBytes() <= 512 * 65536);
  assert.deepEqual(instance.simulate(small), first);
});
