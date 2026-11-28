// What the tests of the loader share: the command line of sbmlsim-core (the wasmtime host, the
// compiler), the `engine.wasm`, `mathlib.wasm` and `compiler.wasm` of its build, and compiled
// models.
//
// `SBMLSIM_CORE_CLI` is the command line to use; without it the tests build it with
// `cargo build --release -p sbmlsim-cli` (which needs `WASI_SDK_PATH`, see the README) and use
// `target/release/sbmlsim-core`. `SBMLSIM_CORE_RUNTIME` is a directory with `engine.wasm`,
// `mathlib.wasm` and `compiler.wasm`; without it the tests write those of the command line
// into a temporary one.

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { Worker } from "node:worker_threads";

import { createRuntime } from "../loader.mjs";

/** The root of the repository. */
export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

let cliPath = null;

/** The path of the command line `sbmlsim-core`, built once per process. */
export function cli() {
  if (cliPath) return cliPath;
  if (process.env.SBMLSIM_CORE_CLI) {
    cliPath = process.env.SBMLSIM_CORE_CLI;
  } else {
    execFileSync("cargo", ["build", "--release", "--quiet", "-p", "sbmlsim-cli"], {
      cwd: ROOT,
      stdio: ["ignore", "inherit", "inherit"],
    });
    cliPath = path.join(ROOT, "target", "release", "sbmlsim-core");
  }
  return cliPath;
}

/** Runs the command line and gives its stdout. */
export function runCli(args, options = {}) {
  return execFileSync(cli(), args, {
    cwd: ROOT,
    maxBuffer: 1 << 30,
    stdio: ["ignore", "pipe", "inherit"],
    ...options,
  });
}

/** The temporary directories of the tests, removed when the process exits. */
const temporary = [];
process.on("exit", () => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

/** A new temporary directory, removed when the process exits. */
export function tempDir(prefix = "sbmlsim-core-") {
  const dir = mkdtempSync(path.join(tmpdir(), prefix));
  temporary.push(dir);
  return dir;
}

let runtimeDirPath = null;

/** The directory of `engine.wasm`, `mathlib.wasm` and `compiler.wasm` of the command line. */
export function runtimeDir() {
  if (runtimeDirPath) return runtimeDirPath;
  if (process.env.SBMLSIM_CORE_RUNTIME) {
    runtimeDirPath = process.env.SBMLSIM_CORE_RUNTIME;
  } else {
    runtimeDirPath = tempDir("sbmlsim-core-runtime-");
    runCli(["runtime", "-o", runtimeDirPath]);
  }
  return runtimeDirPath;
}

/** The bytes of `engine.wasm` and `mathlib.wasm`. */
export function runtimeBytes() {
  const dir = runtimeDir();
  return {
    engine: readFileSync(path.join(dir, "engine.wasm")),
    mathlib: readFileSync(path.join(dir, "mathlib.wasm")),
  };
}

/** A runtime of the loader. */
export async function loaderRuntime(options = {}) {
  return createRuntime({ ...runtimeBytes(), ...options });
}

/** Compiles an SBML text with the command line into a new directory; gives its path. */
export function compileSbml(sbml, name = "model") {
  const dir = tempDir(`sbmlsim-core-${name}-`);
  const file = path.join(dir, "model.xml");
  writeFileSync(file, sbml);
  runCli(["compile", file, "-o", dir]);
  return dir;
}

/** `model.wasm` and the text of `model.json` of a directory. */
export function readModel(dir) {
  return {
    wasm: readFileSync(path.join(dir, "model.wasm")),
    json: readFileSync(path.join(dir, "model.json"), "utf8"),
  };
}

/** The decay model of the tests of the wasmtime host. */
export const DECAY = path.join(ROOT, "crates", "host", "tests", "decay-model");

/** The SBML document the compiled decay model was compiled from. */
export const DECAY_SBML = path.join(ROOT, "crates", "host", "tests", "decay.xml");

/** The bytes of `compiler.wasm` of the runtime, the compiler of `js/compile.mjs`. */
export function compilerWasm() {
  return readFileSync(path.join(runtimeDir(), "compiler.wasm"));
}

/**
 * Whether the prepared cases of the SBML Test Suite exist; with `SBMLSIM_CORE_REQUIRE_CASES` set
 * (the jobs of the suite in CI) their absence is an error instead of a skip.
 */
export function haveCases() {
  const present = existsSync(path.join(ROOT, "testdata", "cases", "00001", "case.json"));
  if (!present && process.env.SBMLSIM_CORE_REQUIRE_CASES) {
    throw new Error("testdata/cases is absent and SBMLSIM_CORE_REQUIRE_CASES is set");
  }
  return present;
}

/** A request of the contract `simulation/1`. */
export function request(fields = {}) {
  return {
    contract: "simulation/1",
    start: 0,
    end: 10,
    output: { uniform: { steps: 10 } },
    preinit: [],
    changes: [],
    steady_state: null,
    selections: [],
    settings: {
      relative_tolerance: 1e-6,
      absolute_tolerance: { scalar: 1e-12 },
      maximum_num_steps: 20000,
      maximum_time_step: 0,
      initial_time_step: 0,
      stiff: true,
      linear_solver: "auto",
      jacobian: "auto",
    },
    seed: 0,
    ...fields,
  };
}

/** A worker of the pool (`pool.mjs`) as a worker thread of Node. */
export function nodeWorker(url) {
  return new Worker(url);
}

/** `fetch` of `file:` URLs (Node's `fetch` has none): a `Response` of the bytes, a 404 without. */
export async function fileFetch(url) {
  try {
    return new Response(await readFile(fileURLToPath(new URL(url))));
  } catch {
    return new Response("", { status: 404 });
  }
}

/** The key of the model of the panel bundle of the tests. */
export const PANEL_MODEL = "default-5d1a3c9e0f27";

let panelBundlePath = null;

/**
 * The panel bundle of the tests (`fixtures/panel-bundle`), copied into a temporary directory with
 * its model compiled by the command line; gives the path of its `bundle.json`.
 */
export function panelBundle() {
  if (panelBundlePath) return panelBundlePath;
  const dir = tempDir("sbmlsim-core-panel-bundle-");
  cpSync(path.join(ROOT, "js", "test", "fixtures", "panel-bundle"), dir, { recursive: true });
  const model = path.join(dir, "models", PANEL_MODEL);
  runCli(["compile", path.join(model, "model.xml"), "-o", model]);
  panelBundlePath = path.join(dir, "bundle.json");
  return panelBundlePath;
}

/** The `file:` URLs of `engine.wasm` and `mathlib.wasm` of the command line. */
export function runtimeUrls() {
  const dir = runtimeDir();
  return {
    engine: pathToFileURL(path.join(dir, "engine.wasm")),
    mathlib: pathToFileURL(path.join(dir, "mathlib.wasm")),
  };
}

/** The pairs of words of the result of each call of `compiler.wasm` which writes one to `out`. */
const RESULT_PAIRS = {
  compile: 2,
  compile_with_options: 2,
  compile_kernel: 2,
  export_ode: 2,
  petab_model: 2,
  petab_open: 1,
  petab_requests: 1,
  petab_evaluate: 1,
  petab_fit_open: 1,
  petab_fit_ask: 1,
  petab_fit_tell: 1,
  stats_call: 2,
};

/**
 * The exports of an instance of `compiler.wasm` with an `allocate` and a `deallocate` which
 * count the buffers a host holds, for the checks that a call frees every buffer.
 *
 * - `live` is the buffers the host holds by their address: those of `allocate` and those of the
 *   results of the calls (the buffers at the words `out` of a call which returned);
 * - `results` counts the buffers of results the host was given, so a check that they are freed
 *   is not vacuous (a call missing from `RESULT_PAIRS` gives none);
 * - `misfreed` is the frees of a buffer the host does not hold, or of another length (a free
 *   of the null pointer or of no bytes is none);
 * - `failAt(k)` makes the `k`-th allocation from now on (1 the next) give 0, a failed one;
 * - `oddWords` makes every buffer of 8 or 16 bytes (the words of a result) start at an odd
 *   address, as a buffer of `allocate`, which has the alignment of bytes, may;
 * - `overrides` replaces exports, e.g. a `compile` which throws.
 */
export function countedExports(exports, { oddWords = false, ...overrides } = {}) {
  const live = new Map();
  const misfreed = [];
  const tally = { results: 0 };
  let failing = 0;
  const counted = { ...exports };
  counted.allocate = (length) => {
    if (failing > 0 && --failing === 0) return 0;
    const shift = oddWords && (length === 8 || length === 16) ? 1 : 0;
    const raw = exports.allocate(length + shift) >>> 0;
    live.set(raw + shift, { length, raw, shift });
    return raw + shift;
  };
  counted.deallocate = (pointer, length) => {
    // the null pointer and a length of 0 are no buffer, which `deallocate` ignores
    if (pointer === 0 || length === 0) return exports.deallocate(pointer, length);
    const buffer = live.get(pointer >>> 0);
    if (buffer === undefined || buffer.length !== length) {
      misfreed.push([pointer, length]);
      return undefined;
    }
    live.delete(pointer >>> 0);
    return exports.deallocate(buffer.raw, length + buffer.shift);
  };
  for (const [name, pairs] of Object.entries(RESULT_PAIRS)) {
    if (typeof exports[name] !== "function") continue;
    counted[name] = (...args) => {
      const status = exports[name](...args);
      const view = new DataView(exports.memory.buffer, args.at(-1) >>> 0, 8 * pairs);
      for (let k = 0; k < pairs; k += 1) {
        const [pointer, length] = [view.getUint32(8 * k, true), view.getUint32(8 * k + 4, true)];
        if (pointer !== 0 && length !== 0) {
          live.set(pointer, { length, raw: pointer, shift: 0 });
          tally.results += 1;
        }
      }
      return status;
    };
  }
  Object.assign(counted, overrides);
  return {
    exports: counted,
    live,
    misfreed,
    get results() {
      return tally.results;
    },
    failAt(k) {
      failing = k;
    },
  };
}
