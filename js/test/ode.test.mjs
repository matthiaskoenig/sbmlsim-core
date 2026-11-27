// Actual import-free compiler.wasm exports, compared with complete legacy files.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createCompiler, CompileError } from "../compile.mjs";
import { compilerWasm } from "./helpers.mjs";

const root = new URL("../../", import.meta.url);
const wasm = await WebAssembly.compile(process.env.SBMLSIM_ODE_WASM
  ? readFileSync(process.env.SBMLSIM_ODE_WASM) : compilerWasm());
assert.deepEqual(WebAssembly.Module.imports(wasm), []);
const version = JSON.parse(readFileSync(new URL("js/package.json", root), "utf8")).version;

for (const label of ["demo", "repressilator", "events", "variable_compartment"]) {
  test(`all source exports of ${label} match native semantics`, async () => {
    const compiler = await createCompiler({ compiler: wasm });
    const source = readFileSync(new URL(`testdata/ode/${label}.xml`, root));
    for (const format of ["python", "diffrax", "julia", "r", "markdown", "latex", "typst"]) {
      for (const complete of [false, true]) {
        const result = compiler.exportOde(source, format, { simulator: complete, standalone: complete });
        const expected = readFileSync(new URL(`testdata/ode/${label}.${format}_${complete}.txt`, root), "utf8")
          .replaceAll("sbmlode 0.3.3", `sbmlsim-core ${version}`);
        assert.equal(result.content, expected, `${format}, complete=${complete}`);
      }
    }
  });
}

test("invalid exports do not corrupt the compiler instance", async () => {
  const compiler = await createCompiler({ compiler: wasm });
  assert.throws(() => compiler.exportOde("<bad/>", "python"), CompileError);
  const source = readFileSync(new URL("testdata/ode/events.xml", root));
  const exported = compiler.exportOde(source, "diffrax", { simulator: false });
  assert.equal(exported.filename, "events_model.py");
  assert.ok(exported.content.includes("import jax"));
});


test("export input types and options fail before entering WASM", async () => {
  const compiler = await createCompiler({ compiler: wasm });
  const source = readFileSync(new URL("testdata/ode/events.xml", root));
  assert.throws(() => compiler.exportOde(123, "python"), TypeError);
  assert.throws(() => compiler.exportOde(source, "python", { simulator: "false" }), TypeError);
  assert.throws(() => compiler.exportOde(source, "python", { documents: { "a.xml": 1 } }), TypeError);
  assert.throws(() => compiler.exportOde(source, "python", { maxExpandedElements: -1 }), TypeError);
  assert.ok(compiler.exportOde(source, "python").content.includes("import numpy"));
});
