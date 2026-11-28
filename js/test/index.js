// `node --test js/test` runs this module (the index of the directory), which imports every test
// file of the loader; `node --test "js/test/*.test.mjs"` runs them in processes of their own.
import "./loader.test.mjs";
import "./compile.test.mjs";
import "./workers.test.mjs";
import "./pool.test.mjs";
import "./panels.test.mjs";
import "./large.test.mjs";
import "./stack.test.mjs";
import "./same_bits.test.mjs";
import "./bench.test.mjs";
import "./model-policy.test.mjs";
import "./prepared.test.mjs";
import "./golden.test.mjs";
import "./inline.test.mjs";
import "./dosing.test.mjs";
import "./dde.test.mjs";
import "./plot.test.mjs";
import "./figure.test.mjs";
import "./hydrate.test.mjs";
import "./petab.test.mjs";
import "./units.test.mjs";
import "./scan.test.mjs";
import "./kernel.test.mjs";
import "./buffers.test.mjs";
