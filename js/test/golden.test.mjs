// The golden files of the wire formats (`testdata/golden`, see `crates/host/tests/golden.rs`) in
// the loader: the description the command line writes is the golden one, the golden request
// gives the CSV of the wasmtime host byte for byte, and the loader refuses a description, a
// request and an `engine.wasm` of another contract or format, and a model without a function of
// the contract, as the wasmtime host does.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { ENGINE_OUTPUT_FORMAT, EngineError, createRuntime, toCsv } from "../loader.mjs";
import { ROOT, loaderRuntime, readModel, runCli, runtimeBytes, tempDir } from "./helpers.mjs";

const GOLDEN = path.join(ROOT, "testdata", "golden");

/** The golden model compiled by the command line. */
function goldenModel() {
  const dir = tempDir("sbmlsim-core-golden-");
  runCli(["compile", path.join(GOLDEN, "model.xml"), "-o", dir]);
  return { dir, ...readModel(dir) };
}

test("the golden description and request in the loader", async () => {
  const { dir, wasm, json } = goldenModel();
  assert.equal(json, readFileSync(path.join(GOLDEN, "model.json"), "utf8"));
  const runtime = await loaderRuntime();
  const instance = await runtime.instantiate(wasm, json);
  const requestPath = path.join(GOLDEN, "request.json");
  const req = readFileSync(requestPath, "utf8");
  const expected = runCli(["simulate", dir, requestPath]).toString();
  assert.equal(toCsv(instance.simulate(req)), expected);
  // without a seed: the seed 1 of the golden request
  const unseeded = JSON.parse(req);
  delete unseeded.seed;
  assert.equal(toCsv(instance.simulate(unseeded)), expected);
});

test("another contract is refused by the loader", async () => {
  const { wasm, json } = goldenModel();
  const runtime = await loaderRuntime();
  await assert.rejects(
    runtime.instantiate(wasm, json.replace('"model/2"', '"model/1"')),
    /contract model\/1, not model\/2/,
  );
  const instance = await runtime.instantiate(wasm, json);
  const req = readFileSync(path.join(GOLDEN, "request.json"), "utf8").replace('"simulation/1"', '"simulation/2"');
  assert.throws(
    () => instance.simulate(req),
    (e) => e instanceof EngineError && e.kind === "request" && /simulation\/2/.test(e.detail),
  );
});

test("GMRES matches wasmtime bitwise and reports iterative counters", async () => {
  const { dir, wasm, json } = goldenModel();
  const runtime = await loaderRuntime();
  const instance = await runtime.instantiate(wasm, json);
  const request = JSON.parse(readFileSync(path.join(GOLDEN, "request.json"), "utf8"));
  request.settings.stiff = true;
  request.settings.linear_solver = "gmres";
  for (const jacobian of ["analytic", "difference_quotient"]) {
    request.settings.jacobian = jacobian;
    const requestPath = path.join(dir, "gmres.json");
    writeFileSync(requestPath, JSON.stringify(request));
    const expected = runCli(["simulate", dir, requestPath]).toString();
    assert.equal(toCsv(instance.simulate(request)), expected);
    const stats = instance.stats();
    for (const name of ["linear_iterations", "jacobian_vector_evals", "preconditioner_evals", "preconditioner_solves"]) {
      assert.ok(stats[name] > 0, name);
    }
    assert.equal(toCsv(instance.simulate(request)), expected);
    assert.deepEqual(instance.stats(), stats);
    // the words after the counts are the bits of a float, which the loader reads as one
    assert.ok(Number.isInteger(stats.steps), "steps");
    assert.ok(stats.last_order >= 1, "last_order");
    for (const name of ["actual_initial_time_step", "last_time_step", "current_time_step", "current_time"]) {
      assert.ok(stats[name] > 0 && stats[name] < 1e6, `${name}: ${stats[name]}`);
    }
    assert.ok(stats.current_time >= request.end, "current_time");
  }
});

test("an engine of another output format is refused by the loader", async () => {
  assert.equal(ENGINE_OUTPUT_FORMAT, 2);
  const { engine, mathlib } = runtimeBytes();
  const needle = Buffer.from('"output_format":2}');
  const at = engine.indexOf(needle);
  assert.ok(at > 0, "engine.wasm declares its output format");
  const other = Buffer.from(engine);
  other[at + needle.length - 2] = "1".charCodeAt(0);
  await assert.rejects(createRuntime({ engine: other, mathlib }), /output format 1, this loader reads 2/);
});

test("a model without a function of the contract is refused as in the wasmtime host", async () => {
  const { wasm, json } = goldenModel();
  // the export model_rhs renamed to model_rhx (the same length)
  const needle = Buffer.from("model_rhs");
  const at = wasm.indexOf(needle);
  assert.ok(at > 0);
  const renamed = Buffer.from(wasm);
  renamed[at + needle.length - 1] = "x".charCodeAt(0);
  const runtime = await loaderRuntime();
  await assert.rejects(runtime.instantiate(renamed, json), (e) => {
    assert.ok(!(e instanceof WebAssembly.LinkError), "no raw LinkError");
    assert.match(e.message, /^engine\.wasm does not instantiate with the model: /);
    return true;
  });
});
