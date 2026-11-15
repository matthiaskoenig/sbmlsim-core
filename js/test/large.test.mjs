// `large_synthetic_model_instantiates_in_node`: a generated SBML of 20,000 species in chains of
// mass action reactions compiles (its functions split below the limits of V8), validates,
// instantiates in Node and simulates 1 time unit with KLU (the linear solver `auto`), with the
// bits of the wasmtime host.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { MAX_MEMORY_PAGES, toCsv } from "../loader.mjs";
import { chainModel } from "./chains.mjs";
import { compileSbml, loaderRuntime, request, runCli, tempDir } from "./helpers.mjs";

/** The sizes of the bodies of the functions a module defines. */
function functionSizes(bytes) {
  const leb = (at) => {
    let value = 0;
    let shift = 0;
    for (;;) {
      const byte = bytes[at.i];
      at.i += 1;
      value += (byte & 0x7f) * 2 ** shift;
      if (byte < 0x80) return value;
      shift += 7;
    }
  };
  const at = { i: 8 };
  while (at.i < bytes.length) {
    const id = bytes[at.i];
    at.i += 1;
    const size = leb(at);
    const end = at.i + size;
    if (id === 10) {
      const sizes = [];
      for (let n = leb(at); n > 0; n -= 1) {
        const body = leb(at);
        sizes.push(body);
        at.i += body;
      }
      return sizes;
    }
    at.i = end;
  }
  return [];
}

const seconds = (start) => ((performance.now() - start) / 1000).toFixed(2);

test("large_synthetic_model_instantiates_in_node", async (t) => {
  const sbml = chainModel(100, 200);
  let start = performance.now();
  const dir = compileSbml(sbml, "chains");
  const compileTime = seconds(start);
  const wasm = readFileSync(path.join(dir, "model.wasm"));
  const json = readFileSync(path.join(dir, "model.json"), "utf8");
  const desc = JSON.parse(json);
  assert.equal(desc.states.length, 20000);
  assert.ok(WebAssembly.validate(wasm), "model.wasm validates");
  const sizes = functionSizes(wasm);
  // the functions are split: more than the 14 of model/1, none above 1 MB
  assert.ok(sizes.length > 14, `${sizes.length} functions`);
  assert.ok(Math.max(...sizes) <= 1_000_000, `${Math.max(...sizes)} bytes`);
  const runtime = await loaderRuntime();
  start = performance.now();
  const instance = await runtime.instantiate(wasm, json);
  const instantiateTime = seconds(start);
  const req = request({
    end: 1,
    output: { uniform: { steps: 10 } },
    selections: ["time", "S0_0", "S0_1", "S0_199", "S99_0", "S99_1"],
  });
  start = performance.now();
  const result = instance.simulate(req);
  const simulateTime = seconds(start);
  assert.equal(result.nRows, 11);
  assert.ok(result.values[6 * 10 + 1] < 1 && result.values[6 * 10 + 2] > 0);
  assert.equal(instance.wasiCalls(), 0);
  assert.ok(instance.memoryBytes() <= MAX_MEMORY_PAGES * 65536);
  // the wasmtime host gives the same CSV
  const requestFile = path.join(tempDir("sbmlsim-core-request-"), "request.json");
  writeFileSync(requestFile, JSON.stringify(req));
  start = performance.now();
  const wasmtime = runCli(["simulate", dir, requestFile]).toString();
  const wasmtimeTime = seconds(start);
  assert.equal(toCsv(result), wasmtime);
  t.diagnostic(
    `20,000 species, ${desc.jacobian.analytic ? "analytic" : Object.keys(desc.jacobian)[0]} Jacobian: compile ${compileTime} s, ` +
      `model.wasm ${(wasm.length / 1e6).toFixed(2)} MB in ${sizes.length} functions (largest ${Math.max(...sizes)} bytes), ` +
      `instantiate ${instantiateTime} s, simulate 1 time unit ${simulateTime} s in Node; ` +
      `wasmtime (sbmlsim-core simulate, with its load) ${wasmtimeTime} s`,
  );
});
