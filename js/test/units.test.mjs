// The units of pint in the JavaScript host (`units.mjs`): the registry of `sbmlsim-units` in
// `compiler.wasm`, held against the corpus of sbmlsim (`testdata/units_corpus.json`, the factors
// and dimensions of `sbmlsim.units.ureg`) as the Rust and the python tests hold it.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { createCompiler } from "../compile.mjs";
import { UnitError, createUnits } from "../units.mjs";
import { ROOT, compilerWasm } from "./helpers.mjs";

const CORPUS = JSON.parse(readFileSync(path.join(ROOT, "testdata", "units_corpus.json"), "utf8"));

test("every expression of the corpus is the factor and the dimensions of pint", async () => {
  const units = await createUnits(await createCompiler({ compiler: compilerWasm() }));
  assert.equal(CORPUS.length, 177);
  for (const { expression, factor_to_root: factor, dimensions } of CORPUS) {
    const parsed = units.parse(expression);
    assert.ok(Math.abs(parsed.factor - factor) <= 1e-15 * Math.abs(factor), expression);
    // the bits of pint, as in Rust: the same registry in wasm
    assert.equal(parsed.factor, factor, expression);
    assert.deepEqual(parsed.dimensions, dimensions, expression);
  }
  units.close();
});

test("factors, conversions and definitions", async () => {
  // the bytes of compiler.wasm as well as a compiler of compile.mjs
  const units = await createUnits(compilerWasm());
  assert.equal(units.factor("l", "ml"), 1000);
  assert.equal(units.factor("mg/min", "g/hr"), 0.06);
  assert.deepEqual(Array.from(units.convert([1, 2], "hr", "min")), [60, 120]);
  assert.deepEqual(units.parse("percent"), { factor: 0.01, dimensions: {} });
  units.define("IU_insulin", "0.0347 * mg");
  units.define("IU_insulin", "0.0347 * mg");
  assert.equal(units.factor("IU_insulin", "ug"), 34.7);
  assert.throws(() => units.define("IU_insulin", "0.045 * mg"), UnitError);
  units.close();
  assert.throws(() => units.factor("l", "ml"), /closed/);
});

test("errors name the units", async () => {
  const units = await createUnits(compilerWasm());
  assert.throws(
    () => units.factor("mg", "ml"),
    (err) =>
      err instanceof UnitError &&
      err.message === "cannot convert 'mg' ([mass]) to 'ml' ([length] ** 3)",
  );
  assert.throws(() => units.factor("degC", "K"), /degree_Celsius/);
  assert.throws(() => units.parse("mg/foo"), /'foo'/);
  assert.throws(() => units.parse("m**0.5"), /power 0.5/);
  units.close();
});

test("a registry with the units of a model", async () => {
  const units = await createUnits(compilerWasm());
  const model = units.withModelUnits({
    units: { k: "mg_per_min", "[S]": "mmol/l" },
    unitIds: { mg_per_min: "mg/min" },
  });
  assert.equal(model.factor("mg_per_min/kg", "mg/min/kg"), 1);
  assert.equal(model.unitOf("k"), "mg_per_min");
  assert.equal(model.unitOf("[S]"), "mmol/l");
  assert.equal(model.unitOf("x"), null);
  // the registry it was copied from is as it was
  assert.throws(() => units.factor("mg_per_min", "mg/min"), /'mg_per_min'/);
  assert.equal(units.unitOf("k"), null);
  model.close();
  assert.equal(units.factor("l", "ml"), 1000);
  units.close();
});
