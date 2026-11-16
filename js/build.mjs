// Builds the inline build of the JavaScript host (`inline.mjs`) into the python package:
//
//   npm ci && node build.mjs            # writes python/sbmlsim_core/web/sbmlsim-core.js
//   node build.mjs --check              # fails if the committed build is not the one of the sources
//
// The worker (`worker.mjs` with the loader) is built first as one classic script, whose source the
// build of `inline.mjs` embeds (`WORKER_SOURCE`); both are IIFEs without `import.meta` (the URL of a
// module, which a classic script has not, is a constant of no origin, `sbmlsim-core.invalid`, which
// the inline endpoint never fetches). The build is committed, so a build of the python package
// needs no Node; the job `web` of CI checks it with `--check`.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.join(HERE, "..", "python", "sbmlsim_core", "web", "sbmlsim-core.js");

/** The options both builds share: one classic script for browsers of the engine's requirements. */
const COMMON = {
  bundle: true,
  write: false,
  format: "iife",
  platform: "browser",
  target: ["es2022"],
  legalComments: "none",
  charset: "utf8",
  define: { "import.meta.url": JSON.stringify("https://sbmlsim-core.invalid/") },
  logLevel: "warning",
  // the paths of the comments of the bundle are relative to this directory, whatever the cwd
  absWorkingDir: HERE,
};

/** The text of the one output of a build. */
async function text(options) {
  const result = await build({ ...COMMON, ...options });
  return result.outputFiles[0].text;
}

const worker = await text({ entryPoints: [path.join(HERE, "worker.mjs")] });
const inline = await text({
  entryPoints: [path.join(HERE, "inline.mjs")],
  globalName: "sbmlsimCore",
  define: { ...COMMON.define, __SBMLSIM_CORE_WORKER__: JSON.stringify(worker) },
});
const { version } = JSON.parse(readFileSync(path.join(HERE, "package.json"), "utf8"));
const output =
  `// sbmlsim-core ${version}, the inline build of its JavaScript host (js/inline.mjs, js/build.mjs).\n` +
  "// Generated, do not edit. MIT AND BSD-3-Clause AND LGPL-2.1-or-later, see LICENSE and THIRD_PARTY.md.\n" +
  inline;

if (process.argv.includes("--check")) {
  let committed = "";
  try {
    committed = readFileSync(OUT, "utf8");
  } catch {
    // missing: not the build of the sources
  }
  if (committed !== output) {
    console.error(`${path.relative(process.cwd(), OUT)} is not the build of the sources: run 'node js/build.mjs'`);
    process.exit(1);
  }
} else {
  writeFileSync(OUT, output);
  console.log(`${path.relative(process.cwd(), OUT)}: ${output.length} bytes`);
}
