import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { createRuntime, validateModelBytes } from "../loader.mjs";
import { createPool } from "../pool.mjs";

const header = [0, 97, 115, 109, 1, 0, 0, 0];
const memory = [2, 15, 1, 3, 101, 110, 118, 6, 109, 101, 109, 111, 114, 121, 2, 0, 1];
const fixtures = {
  mutable: [...header, ...memory, 6, 6, 1, 0x7f, 1, 0x41, 0, 0x0b],
  table: [...header, ...memory, 4, 4, 1, 0x70, 0, 1],
  memory: [...header, 5, 3, 1, 0, 1],
  start: [...header, 1, 4, 1, 0x60, 0, 0, ...memory, 3, 2, 1, 0, 8, 1, 0, 10, 4, 1, 2, 0, 0x0b],
  data: [...header, ...memory, 11, 6, 1, 0, 0x41, 0, 0x0b, 0],
  references: [...header, 1, 5, 1, 0x60, 1, 0x6f, 0, ...memory],
};

test("byte-input model policy rejects state, tables, starts and data before instantiation", async () => {
  const engine = readFileSync(new URL("../runtime/engine.wasm", import.meta.url));
  const mathlib = readFileSync(new URL("../runtime/mathlib.wasm", import.meta.url));
  const description = readFileSync(new URL("../../crates/host/tests/decay-model/model.json", import.meta.url), "utf8");
  const runtime = await createRuntime({ engine, mathlib });
  const pool = await createPool({ engine, mathlib });
  try {
    for (const [name, input] of Object.entries(fixtures)) {
      const bytes = Uint8Array.from(input);
      assert.ok(WebAssembly.validate(bytes), name);
      assert.throws(() => validateModelBytes(bytes), /model.wasm/);
      await assert.rejects(runtime.instantiate(bytes, description), /model.wasm/);
      await assert.rejects(pool.addModel(name, bytes, description), /model.wasm/);
      assert.equal(pool.hasModel(name), false);
    }
    const immutable = Uint8Array.from(fixtures.mutable);
    immutable[immutable.length - 4] = 0;
    assert.ok(WebAssembly.validate(immutable));
    validateModelBytes(immutable);
  } finally { pool.close(); }
});
