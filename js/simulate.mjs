#!/usr/bin/env node
// `node js/simulate.mjs <model-dir> <request.json> [-o out.csv]`: simulates a compiled model (the
// directory holds `model.wasm` and `model.json`) with a request of the contract `simulation/1`
// in Node, with the loader `loader.mjs`, and writes the result as the CSV of
// `sbmlsim-core simulate` (byte for byte, `sbmlsim_cli::csv`), to `out.csv` or else to stdout.
//
// `engine.wasm` and `mathlib.wasm` are read from the directory `$SBMLSIM_CORE_RUNTIME`, else
// from `js/runtime/` next to this script (written by `sbmlsim-core runtime -o js/runtime`).
// Errors go to stderr with the exit code 1; a usage error has the exit code 2.

import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { createRuntime, toCsv } from "./loader.mjs";

const usage = "usage: node simulate.mjs <model-dir> <request.json> [-o out.csv]";

function parseArgs(argv) {
  const positional = [];
  let output = null;
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === "-o" || argv[i] === "--output") {
      output = argv[i + 1];
      i += 1;
      if (output === undefined) return null;
    } else {
      positional.push(argv[i]);
    }
  }
  return positional.length === 2 ? { modelDir: positional[0], request: positional[1], output } : null;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args) {
    console.error(usage);
    return 2;
  }
  const runtimeDir =
    process.env.SBMLSIM_CORE_RUNTIME ??
    path.join(path.dirname(fileURLToPath(import.meta.url)), "runtime");
  const read = (file) => readFile(file).catch((e) => Promise.reject(new Error(`cannot read ${file}: ${e.message}`)));
  const [engine, mathlib, modelWasm, modelJson, request] = await Promise.all([
    read(path.join(runtimeDir, "engine.wasm")),
    read(path.join(runtimeDir, "mathlib.wasm")),
    read(path.join(args.modelDir, "model.wasm")),
    read(path.join(args.modelDir, "model.json")),
    read(args.request),
  ]);
  const runtime = await createRuntime({ engine, mathlib });
  const instance = await runtime.instantiate(modelWasm, modelJson.toString("utf8"));
  const csv = toCsv(instance.simulate(request.toString("utf8")));
  if (args.output) await writeFile(args.output, csv);
  else process.stdout.write(csv);
  return 0;
}

main().then(
  (code) => {
    process.exitCode = code;
  },
  (error) => {
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  },
);
