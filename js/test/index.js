// `node --test js/test` runs this module (the index of the directory), which imports every test
// file of the loader; `node --test "js/test/*.test.mjs"` runs them in processes of their own.
import "./loader.test.mjs";
import "./workers.test.mjs";
import "./large.test.mjs";
import "./stack.test.mjs";
import "./same_bits.test.mjs";
import "./bench.test.mjs";
