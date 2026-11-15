// The engine and the model run within a small native stack: a function of `model.wasm` with
// 19,001 locals (a frame of 152 kB on the stack of V8, near MAX_LOCALS = 20,000) simulates in a
// worker thread whose stack is 1 MB, with the bits of the main thread and of wasmtime.

import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { Worker } from "node:worker_threads";

import { toCsv } from "../loader.mjs";
import { compileSbml, loaderRuntime, readModel, request, runCli, runtimeBytes, tempDir } from "./helpers.mjs";

const MATH = 'xmlns="http://www.w3.org/1998/Math/MathML"';

/** `n` assignment rules `r<i> = x * (i + 1)` and their sum `total`, all selected. */
function wideModel(n) {
  const params = [];
  const rules = [];
  const terms = [];
  for (let i = 0; i < n; i += 1) {
    params.push(`<parameter id="r${i}" constant="false"/>`);
    rules.push(`<assignmentRule variable="r${i}"><math ${MATH}><apply><times/><ci>x</ci><cn>${i + 1}</cn></apply></math></assignmentRule>`);
    terms.push(`<ci>r${i}</ci>`);
  }
  return `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="wide">
<listOfParameters><parameter id="x" value="1" constant="false"/><parameter id="total" constant="false"/>${params.join("")}</listOfParameters>
<listOfRules><rateRule variable="x"><math ${MATH}><apply><minus/><ci>x</ci></apply></math></rateRule>${rules.join("")}<assignmentRule variable="total"><math ${MATH}><apply><plus/>${terms.join("")}</apply></math></assignmentRule></listOfRules>
</model></sbml>`;
}

/** The most locals of a function a module defines. */
function mostLocals(bytes) {
  let i = 8;
  const leb = () => {
    let value = 0;
    let shift = 0;
    for (;;) {
      const byte = bytes[i];
      i += 1;
      value += (byte & 0x7f) * 2 ** shift;
      if (byte < 0x80) return value;
      shift += 7;
    }
  };
  let most = 0;
  while (i < bytes.length) {
    const id = bytes[i];
    i += 1;
    const end = leb() + i;
    if (id === 10) {
      for (let n = leb(); n > 0; n -= 1) {
        const size = leb();
        const start = i;
        let locals = 0;
        for (let groups = leb(); groups > 0; groups -= 1) {
          locals += leb();
          i += 1;
        }
        most = Math.max(most, locals);
        i = start + size;
      }
    }
    i = end;
  }
  return most;
}

test("a function of 19,001 locals simulates in a worker with a stack of 1 MB", async () => {
  const dir = compileSbml(wideModel(19000), "wide");
  const { wasm, json } = readModel(dir);
  assert.ok(mostLocals(wasm) > 19000, `${mostLocals(wasm)} locals`);
  const req = request({ end: 1, selections: ["time", "x", "total", "r18999"] });
  const { engine, mathlib } = runtimeBytes();
  const result = await new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./fixtures/worker.mjs", import.meta.url), {
      workerData: { engine, mathlib, model: wasm, description: json, request: req },
      resourceLimits: { stackSizeMb: 1 },
    });
    worker.once("message", (m) => {
      resolve(m);
      worker.terminate();
    });
    worker.once("error", reject);
  });
  const main = (await (await loaderRuntime()).instantiate(wasm, json)).simulate(req);
  assert.equal(toCsv(result), toCsv(main));
  const requestFile = path.join(tempDir("sbmlsim-core-request-"), "request.json");
  writeFileSync(requestFile, JSON.stringify(req));
  assert.equal(toCsv(main), runCli(["simulate", dir, requestFile]).toString());
});
