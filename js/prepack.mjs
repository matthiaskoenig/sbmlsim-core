// The check before `npm pack` and `npm publish` (the script `prepack` of package.json): the
// package ships the runtime of a build of the core and the license of the repository, which are
// no files of the source of `js/`. `runtime/` is written by `sbmlsim-core runtime -o js/runtime`
// (engine.wasm, mathlib.wasm and the notices of their third parties, THIRD_PARTY.md), and the
// license is copied from the root of the repository. The version of the package is the one of
// the workspace of the crates and of `CORE_VERSION` of panels.mjs, so a release names one
// version everywhere.

import { copyFile, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "..");

const fail = (message) => {
  console.error(`prepack: ${message}`);
  process.exit(1);
};

for (const name of ["engine.wasm", "mathlib.wasm", "THIRD_PARTY.md"]) {
  if (!existsSync(path.join(here, "runtime", name))) {
    fail(`runtime/${name} is missing: run 'sbmlsim-core runtime -o js/runtime' of a release build first`);
  }
}

const { version } = JSON.parse(await readFile(path.join(here, "package.json"), "utf8"));
const cargo = await readFile(path.join(root, "Cargo.toml"), "utf8");
const workspace = /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(cargo)?.[1];
if (workspace !== version) {
  fail(`the version ${version} of package.json is not the version ${workspace} of Cargo.toml`);
}
const { CORE_VERSION } = await import("./panels.mjs");
if (CORE_VERSION !== version) {
  fail(`the version ${version} of package.json is not CORE_VERSION ${CORE_VERSION} of panels.mjs`);
}

await copyFile(path.join(root, "LICENSE"), path.join(here, "LICENSE"));
