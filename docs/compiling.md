# Compiling a model

The command line compiles an SBML Level 3 file (Version 1 or 2) into a model directory: `model.wasm`, the compiled model, and `model.json`, its description (`model/2`, see [Simulating](simulating.md#the-description-model2)).

```bash
sbmlsim-core compile model.xml -o model-dir            # target/release/sbmlsim-core of a build
cargo run -p sbmlsim-cli -- compile crates/host/tests/decay.xml -o decay-model
```

| option | what it does |
|---|---|
| `-o`, `--output <DIR>` | the directory to write `model.wasm` and `model.json` into |
| `--no-optimize` | without the optimizer: the same results, a larger and slower module |

## What the compiler reads

The reader (`crates/sbml`) takes SBML Level 3 Version 2 core. A package with `required="false"` (layout, render, fbc annotations, ...) is ignored, and a package whose `required` attribute is `"true"` is refused (`the required package comp is not supported`): `comp` must be flattened before, a model read without its submodels is another model. A required package whose elements only annotate the model is read as well, its elements ignored as those of an optional one, so far `distrib`: it carries the uncertainty of a parameter (`<distrib:uncertainty>`, which the models of pkdb_models write for every fitted parameter: the confidence interval of the profile likelihood, the standard error of the Fisher information and the distribution of the draws), and the mathematics of the model is the same without it. The package is read by its namespace URI, never by its prefix, and only the elements of an uncertainty are read of it (`listOfUncertainties`, `uncertainty`, `listOfUncertParameters`, `uncertParameter`, `uncertSpan`, the whole of the package as libsbml implements it): any other element of that namespace is refused with its name (`<drawFromDistribution> of the namespace ... is not supported`), not ignored, so that no element which computes is dropped on the way to a deterministic simulation. A draw is therefore refused in either shape it has, whatever `required` says: the `<csymbol>` of a distribution (`http://www.sbml.org/sbml/symbols/distrib/normal`) in a math of the model is an unknown csymbol, as any math the reader does not understand, and the `<distrib:drawFromDistribution>` element of the drafts of the package is an element of the namespace which is no uncertainty. Units pass through as text and nothing is computed with them: a unit definition becomes a unit expression which a unit registry such as pint parses (`mmol/min` as `0.001*mole*(60*second)**-1`), the field `units` of `model.json`.

A feature the engine does not simulate as sbmlsim does is refused with its element (`algebraic rule is not supported (algebraicRule #0)`), and so is a model which is not valid SBML (an unknown identifier, a cycle of assignment rules). In the SBML Test Suite the refusals are the algebraic rules, the `delay` csymbol and the fast reactions (see [Test suite and benchmark](testsuite.md)). Missing math is handled as libroadrunner 2.11 handles it: a rule or an initial assignment without math is ignored, an event without a trigger is ignored, a delay or a priority without math is none; a kinetic law without math, a call of a function definition without math and an empty `<math>` are refused.

Compilation also bounds function expansion, generated discontinuity roots and analytic derivatives before large intermediate expressions are constructed. Deep alias chains and expanding function substitutions return a compile error. Wide XOR conditions use sequential temporaries instead of duplicating the accumulated expression. If analytic differentiation exceeds its construction budget, `auto` uses the colored difference quotient and records the reason; an explicitly requested analytic Jacobian is refused. An explicitly requested difference quotient skips analytic derivative construction.

## What the compiler writes

`model.wasm` imports the memory of the engine (`env.memory`) and the functions of `mathlib.wasm` it calls (`math.*`), and exports the functions of `model/2`: the defaults and the initialization of the model, the right hand side, the Jacobian, the root functions of the triggers and of the discontinuities of the math, the functions of the events, the selections and the sizes of the vectors. The expressions are emitted as written, without fast math, reassociation or folding of constants (`q / 49` is one division by `49`), with the meaning libroadrunner gives every operator; the optimizer folds constants, applies only rewrites which are exact in binary64 and computes the subexpressions of the parameters alone once per change of the parameters (the constants of the model), so it gives the bits of the code as written.

The states are the vector of libroadrunner: first the targets of the rate rules in the order of the rules, then every species which reactions change in the order of the species. A species is stored as its amount (the selection `S`; `[S]` is its concentration), except a species of a concentration which is the target of a rule, which stores its concentration, as libroadrunner does.

The Jacobian is analytic, on its sparsity pattern. A model whose math has no derivative the compiler writes (the description names the operator and the element as the `reason`) gets the colored sparse difference quotient instead, which the engine computes; so does an analytic Jacobian with an entry which is not finite at a call (e.g. of `sqrt(x)` at `x = 0`).

## Preparing a model

A model of another level or version or with `comp` is prepared with `scripts/prepare_model.py` first (libsbml, run with uv):

```bash
uv run scripts/prepare_model.py model.xml -o prepared.xml
sbmlsim-core compile prepared.xml -o model-dir
```

- `comp` is flattened.
- The elements and annotations of `distrib` are stripped, which the compiler does not need (it reads the package and ignores them, see above): the uncertainties of `distrib` (`<distrib:listOfUncertainties>` of the parameters of a fit) are metadata and do not change a simulation. A distribution function of `distrib` in a math (`normal(0, 1)` as a csymbol) would change it, so such a model is refused with a non-zero exit, naming the element (`initialAssignment of x (normal)`), as the compiler refuses it.
- The model is converted to L3V2.
- The rest of the model (non-required packages such as `fbc` and `layout`, notes, annotations) is kept.

With `--bench bench/models.toml` the script prepares every model of the benchmark from its `source` into its `sbml`, with the absolute tolerance of every state as roadrunner scales the scalar one (`<name>.atol.json`), see [Test suite and benchmark](testsuite.md#benchmark).

## The compiler in a browser

The compiler crates (`sbmlsim-abi`, `sbmlsim-sbml`, `sbmlsim-ir`, `sbmlsim-codegen`, `sbmlsim-compiler`) do no file system or OS access, so they build for `wasm32-unknown-unknown`, which CI checks:

```bash
cargo build -p sbmlsim-compiler --target wasm32-unknown-unknown --release
```
