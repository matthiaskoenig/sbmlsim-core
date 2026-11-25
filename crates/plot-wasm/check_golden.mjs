// The same bytes in a JavaScript host: runs `plot.wasm` in Node on every fixture of the renderer
// (`crates/plot/tests/specs/*.json`) and compares the SVG with its golden file
// (`crates/plot/tests/golden/<name>.svg`), which the native tests of `sbmlsim-plot` compare with
// `render_svg`. So a figure a page draws with the module is the one of the python package, byte
// for byte.
//
//   node crates/plot-wasm/check_golden.mjs target/wasm32-wasip1/wasm/sbmlsim_plot_wasm.wasm
//
// The module imports five functions of WASI preview1. `random_get` is called once per instance,
// for the seeds of the hash maps of Charton (ahash), which decide no byte of a result: every
// fixture is rendered by an instance whose `random_get` gives zeros and by one which gives the
// random bytes of `crypto.getRandomValues`, and both must give the golden bytes. The other four
// are called at most on a panic, and their stubs throw.

import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const WASI = "wasi_snapshot_preview1";
const here = path.dirname(fileURLToPath(import.meta.url));
const specs = path.join(here, "..", "plot", "tests", "specs");
const golden = path.join(here, "..", "plot", "tests", "golden");

const [wasmPath] = process.argv.slice(2);
if (!wasmPath) {
  console.error("usage: node check_golden.mjs <plot.wasm>");
  process.exit(2);
}
const module = new WebAssembly.Module(readFileSync(wasmPath));

/** An instance of the module whose `random_get` fills a buffer with `fill(bytes)`. */
function instantiate(fill) {
  let memory = null;
  const wasi = {
    random_get(pointer, len) {
      fill(new Uint8Array(memory.buffer, pointer, len));
      return 0;
    },
  };
  const imports = {};
  for (const { module: from, name, kind } of WebAssembly.Module.imports(module)) {
    if (from !== WASI || kind !== "function") {
      throw new Error(`plot.wasm imports ${from}.${name} (${kind}), which is no function of WASI`);
    }
    imports[name] =
      wasi[name] ??
      (() => {
        throw new Error(`plot.wasm called WASI ${name}`);
      });
  }
  const instance = new WebAssembly.Instance(module, { [WASI]: imports });
  memory = instance.exports.memory;
  instance.exports._initialize?.();
  return instance.exports;
}

/** Renders a spec through the C ABI: status and text of the result, every buffer freed. */
function render(exports, spec) {
  const input = new TextEncoder().encode(spec);
  const pointer = exports.allocate(input.length);
  if (pointer === 0 && input.length > 0) throw new Error("plot.wasm gave no buffer");
  new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
  const out = exports.allocate(8);
  const status = exports.render(pointer, input.length, out);
  // the two words of `out`, little endian, at whatever alignment `allocate` gave
  const words = new DataView(exports.memory.buffer, out, 8);
  const [resultPointer, resultLength] = [words.getUint32(0, true), words.getUint32(4, true)];
  const text = new TextDecoder("utf-8", { fatal: true }).decode(
    new Uint8Array(exports.memory.buffer, resultPointer, resultLength),
  );
  exports.deallocate(resultPointer, resultLength);
  exports.deallocate(out, 8);
  exports.deallocate(pointer, input.length);
  return { status, text };
}

/** Renders through a prepared figure: `prepare`, `render_prepared`, `release`. */
function renderPrepared(exports, spec) {
  const input = new TextEncoder().encode(spec);
  const pointer = exports.allocate(input.length);
  new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
  const out = exports.allocate(8);
  const handle = exports.prepare(pointer, input.length, out);
  exports.deallocate(pointer, input.length);
  const read = (status) => {
    const words = new DataView(exports.memory.buffer, out, 8);
    const [resultPointer, resultLength] = [words.getUint32(0, true), words.getUint32(4, true)];
    const text = new TextDecoder("utf-8", { fatal: true }).decode(
      new Uint8Array(exports.memory.buffer, resultPointer, resultLength),
    );
    exports.deallocate(resultPointer, resultLength);
    return { status, text };
  };
  let result;
  if (handle === 0) {
    result = read(0);
  } else {
    result = read(exports.render_prepared(handle, out));
    exports.release(handle);
  }
  exports.deallocate(out, 8);
  return result;
}

const sources = [
  ["zero seeds", instantiate((bytes) => bytes.fill(0))],
  ["random seeds", instantiate((bytes) => crypto.getRandomValues(bytes))],
];
const names = readdirSync(specs)
  .filter((name) => name.endsWith(".json"))
  .map((name) => name.slice(0, -".json".length))
  .sort();
if (names.length === 0) {
  console.error(`no fixtures in ${specs}`);
  process.exit(1);
}
let failures = 0;
for (const name of names) {
  const spec = readFileSync(path.join(specs, `${name}.json`), "utf8");
  const expected = readFileSync(path.join(golden, `${name}.svg`), "utf8");
  for (const [seeds, exports] of sources) {
    for (const [how, run] of [
      ["", render],
      [" prepared", renderPrepared],
    ]) {
    const { status, text } = run(exports, spec);
    if (status !== 1) {
      console.error(`${name} (${seeds}${how}): refused: ${text}`);
      failures += 1;
    } else if (text !== expected) {
      let at = 0;
      while (at < text.length && text[at] === expected[at]) at += 1;
      console.error(`${name} (${seeds}${how}): differs from golden/${name}.svg at character ${at}`);
      failures += 1;
    } else {
      console.log(`${name} (${seeds}${how}): the golden bytes (${Buffer.byteLength(text)} bytes)`);
    }
    }
  }
}
if (failures > 0) {
  console.error(`${failures} of ${names.length * sources.length * 2} renderings differ`);
  process.exit(1);
}
console.log(`plot.wasm renders all ${names.length} fixtures to their golden bytes`);
