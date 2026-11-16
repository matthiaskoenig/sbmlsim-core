// `same_bits_as_wasmtime`: every case of the SBML Test Suite which passes gives in Node the CSV
// of the wasmtime host byte for byte, with the same engine.wasm, mathlib.wasm and model.wasm.
//
// Bitwise equality is that of the CSV of the command line (`sbmlsim_cli::csv`), which writes
// every double as the shortest decimal which reads back as it, `-0` included: two CSVs are equal
// exactly when the results are equal bit for bit, every NaN equal to every NaN. The NaN payloads
// are not part of the equality: WebAssembly does not fix the payload of the NaN an operation
// gives (wasmtime and V8 give the NaN of the hardware, which differs between x86-64 and
// aarch64), and canonicalizing them in wasmtime would not canonicalize them in V8. The test
// compares the raw values too and reports NaNs of other payloads, which do not fail it.
//
// The cases come from `sbmlsim-core testsuite <cases> --export <dir>`: `SBMLSIM_CORE_EXPORT` is
// such a directory; without it the test exports the prepared cases of `testdata/cases` itself,
// and is skipped without them.

import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { createRuntime, toCsv } from "../loader.mjs";
import { ROOT, haveCases, runCli, tempDir } from "./helpers.mjs";

/** The directory of the exported cases, `null` without. */
function exported() {
  if (process.env.SBMLSIM_CORE_EXPORT) return process.env.SBMLSIM_CORE_EXPORT;
  if (!haveCases()) return null;
  const dir = tempDir("sbmlsim-core-export-");
  runCli(["testsuite", path.join(ROOT, "testdata", "cases"), "--export", dir]);
  return dir;
}

test("same_bits_as_wasmtime", async (t) => {
  const dir = exported();
  if (!dir) {
    t.skip("no prepared cases (uv run scripts/prepare_testsuite.py --out testdata/cases)");
    return;
  }
  const baseline = JSON.parse(readFileSync(path.join(ROOT, "testdata", "testsuite_baseline.json"), "utf8"));
  const runtime = await createRuntime({
    engine: readFileSync(path.join(dir, "engine.wasm")),
    mathlib: readFileSync(path.join(dir, "mathlib.wasm")),
  });
  const cases = readdirSync(path.join(dir, "cases")).sort();
  assert.equal(cases.length, baseline.n_passed, "every case which passes is exported");
  const differ = [];
  let steady = 0;
  let nans = 0;
  let otherPayloads = 0;
  let wasiCalls = 0;
  const started = performance.now();
  for (const id of cases) {
    const caseDir = path.join(dir, "cases", id);
    const read = (name) => readFileSync(path.join(caseDir, name));
    const instance = await runtime.instantiate(read("model.wasm"), read("model.json").toString());
    const result = instance.simulate(read("request.json").toString());
    if (toCsv(result) !== read("wasmtime.csv").toString()) differ.push(`${id}: the result`);
    const plan = instance.prepare(read("request.json").toString());
    const prepared = instance.simulatePrepared(plan);
    if (toCsv(prepared) !== toCsv(result) || !Buffer.from(prepared.values.buffer).equals(Buffer.from(result.values.buffer))) {
      differ.push(`${id}: the prepared simulation`);
    }
    plan.close();
    // the raw values: NaNs of other payloads are reported, not failed
    const raw = read("wasmtime.f64");
    const theirs = new BigUint64Array(raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength));
    const ours = new BigUint64Array(result.values.buffer);
    for (let i = 0; i < ours.length; i += 1) {
      if (Number.isNaN(result.values[i])) {
        nans += 1;
        if (ours[i] !== theirs[i]) otherPayloads += 1;
      }
    }
    // a second simulation of the instance is the first
    if (toCsv(instance.simulate(read("request.json").toString())) !== toCsv(result)) {
      differ.push(`${id}: the second simulation`);
    }
    if (existsSync(path.join(caseDir, "steady.json"))) {
      steady += 1;
      const fresh = await runtime.instantiate(read("model.wasm"), read("model.json").toString());
      if (toCsv(fresh.simulate(read("steady.json").toString())) !== read("steady.csv").toString()) {
        differ.push(`${id}: the steady state`);
      }
      const steadyPlan = fresh.prepare(read("steady.json").toString());
      if (toCsv(fresh.simulatePrepared(steadyPlan)) !== read("steady.csv").toString()) {
        differ.push(`${id}: the prepared steady state`);
      }
      steadyPlan.close();
      wasiCalls += fresh.wasiCalls();
    }
    wasiCalls += instance.wasiCalls();
  }
  t.diagnostic(
    `${cases.length} cases (${steady} with a steady state) in ${((performance.now() - started) / 1000).toFixed(1)} s: ` +
      `${differ.length} differ; ${nans} NaNs, ${otherPayloads} of other payloads`,
  );
  assert.deepEqual(differ.slice(0, 20), []);
  assert.equal(wasiCalls, 0);
});
