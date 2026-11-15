// What the tests of the loader share: the command line of sbmlsim-core (the wasmtime host, the
// compiler), the `engine.wasm` and `mathlib.wasm` of its build, and compiled models.
//
// `SBMLSIM_CORE_CLI` is the command line to use; without it the tests build it with
// `cargo build --release -p sbmlsim-cli` (which needs `WASI_SDK_PATH`, see the README) and use
// `target/release/sbmlsim-core`. `SBMLSIM_CORE_RUNTIME` is a directory with `engine.wasm` and
// `mathlib.wasm`; without it the tests write those of the command line into a temporary one.

import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

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

/** A new temporary directory. */
export function tempDir(prefix = "sbmlsim-core-") {
  return mkdtempSync(path.join(tmpdir(), prefix));
}

let runtimeDirPath = null;

/** The directory of `engine.wasm` and `mathlib.wasm` of the command line. */
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
