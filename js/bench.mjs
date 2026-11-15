#!/usr/bin/env node
// `node js/bench.mjs <export-dir> [--json FILE] [--only a,b] [--rounds N] [--warmups N]
// [--reps N]`: the benchmark of `sbmlsim-core bench` in Node, with the models and requests of
// `sbmlsim-core bench bench/models.toml --export <export-dir>` (`js/bench-core.mjs`).
//
// `engine.wasm` and `mathlib.wasm` are read from `$SBMLSIM_CORE_RUNTIME`, else from `js/runtime/`
// (`sbmlsim-core runtime -o js/runtime`, the runtime of the same build as the command line).

import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { runBench } from "./bench-core.mjs";

const usage =
  "usage: node bench.mjs <export-dir> [--json FILE] [--only a,b] [--rounds N] [--warmups N] [--reps N]";

function parseArgs(argv) {
  const options = { dir: null, json: null, only: null, rounds: null, warmups: null, reps: null };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag.startsWith("--")) {
      const value = argv[i + 1];
      i += 1;
      if (value === undefined) return null;
      const key = flag.slice(2);
      if (!(key in options) || key === "dir") return null;
      options[key] = key === "only" ? value.split(",") : key === "json" ? value : Number(value);
    } else if (options.dir === null) {
      options.dir = flag;
    } else {
      return null;
    }
  }
  return options.dir === null ? null : options;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args) {
    console.error(usage);
    return 2;
  }
  const runtimeDir =
    process.env.SBMLSIM_CORE_RUNTIME ?? path.join(path.dirname(fileURLToPath(import.meta.url)), "runtime");
  const manifest = JSON.parse(await readFile(path.join(args.dir, "bench.json"), "utf8"));
  const result = await runBench({
    host: "node",
    manifest,
    readBytes: (file) => readFile(path.join(args.dir, file)),
    readText: (file) => readFile(path.join(args.dir, file), "utf8"),
    engine: await readFile(path.join(runtimeDir, "engine.wasm")),
    mathlib: await readFile(path.join(runtimeDir, "mathlib.wasm")),
    now: () => performance.now(),
    log: (line) => console.error(line),
    only: args.only,
    rounds: args.rounds ?? manifest.rounds,
    warmups: args.warmups ?? manifest.warmups,
    reps: args.reps ?? manifest.reps,
  });
  result.node = process.version;
  for (const model of result.models) {
    for (const c of model.configs) {
      console.log(
        `${model.name.padEnd(14)} ${String(model.states).padStart(6)} ${c.solver.padStart(6)} ${c.jacobian.padStart(10)} ${c.output.padStart(6)}` +
          ` ${(c.median_s * 1e3).toFixed(4).padStart(10)} ms  steps ${c.stats.steps}  inst ${(model.instantiate_s * 1e3).toFixed(2)} ms` +
          (c.repeat_bitwise ? "" : "  NOT BITWISE"),
      );
    }
  }
  if (args.json) {
    await mkdir(path.dirname(args.json), { recursive: true });
    await writeFile(args.json, JSON.stringify(result, null, 1));
  }
  return result.models.every((m) => m.configs.every((c) => c.repeat_bitwise)) ? 0 : 1;
}

process.exitCode = await main();
