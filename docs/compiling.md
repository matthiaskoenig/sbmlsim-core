# Compiling a model

The command line compiles an SBML file of any level and version into a model directory: `model.wasm`, the compiled model, and `model.json`, its description (`model/2`, see [Simulating](simulating.md#the-description-model2)).

```bash
sbmlsim-core compile model.xml -o model-dir            # target/release/sbmlsim-core of a build
cargo run -p sbmlsim-cli -- compile crates/host/tests/decay.xml -o decay-model
```

| option | what it does |
|---|---|
| `-o`, `--output <DIR>` | the directory to write `model.wasm` and `model.json` into |
| `--no-optimize` | without the optimizer: the same results, a larger and slower module |
| `--no-parameter-jacobian` | omit analytic parameter derivatives and their cached constants for ordinary simulations; forward sensitivities use numerical parameter derivatives (0.26.0) |

<svg class="core-diagram" viewBox="0 12 840 304" xmlns="http://www.w3.org/2000/svg" role="img"
     aria-label="A model of any SBML level and version goes into the compiler as it stands, prepared with prepare_model.py only when it carries comp. The compiler takes SBML core of every level, reading the distrib package as annotation only, and writes model.wasm, which imports the memory of the engine and the functions of mathlib and exports the contract model/2, and model.json, which describes the states and selections, the units as text and the Jacobian with the reason when it is not analytic. A required package, a fast reaction, an unknown identifier, a cycle of assignment rules and a draw of distrib are refused, each named with the element which caused it.">
  <defs>
    <marker id="cp-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="currentColor"/>
    </marker>
  </defs>
  <g class="cd-step">
    <rect x="16" y="48" width="132" height="46" rx="6"/>
    <text x="82" y="70">any SBML</text>
    <text class="cd-sub" x="82" y="86">L1, L2, L3, comp</text>
  </g>
  <line x1="154" y1="71" x2="186" y2="71" marker-end="url(#cp-arrow)"/>
  <g class="cd-step">
    <rect x="192" y="48" width="150" height="46" rx="6"/>
    <text x="267" y="70">prepare_model.py</text>
    <text class="cd-sub" x="267" y="86">libsbml, for comp</text>
  </g>
  <line x1="348" y1="71" x2="380" y2="71" marker-end="url(#cp-arrow)"/>
  <g class="cd-step">
    <rect x="386" y="48" width="132" height="46" rx="6"/>
    <text x="452" y="70">SBML core</text>
    <text class="cd-sub" x="452" y="86">any level, flat</text>
  </g>
  <line x1="524" y1="71" x2="556" y2="71" marker-end="url(#cp-arrow)"/>
  <g class="cd-step cd-active">
    <rect x="562" y="48" width="150" height="46" rx="6"/>
    <text x="637" y="70">compiler</text>
    <text class="cd-sub" x="637" y="86">crates/sbml</text>
  </g>
  <line x1="637" y1="100" x2="637" y2="148" marker-end="url(#cp-arrow)"/>
  <line x1="365" y1="124" x2="637" y2="124"/>
  <line x1="365" y1="124" x2="365" y2="148" marker-end="url(#cp-arrow)"/>
  <text class="cd-contract" x="652" y="118">model/2</text>
  <g class="cd-step">
    <rect x="240" y="154" width="250" height="88" rx="6"/>
    <text x="365" y="176">model.wasm</text>
    <text class="cd-sub" x="365" y="198">imports env.memory</text>
    <text class="cd-sub" x="365" y="216">imports math.*</text>
    <text class="cd-sub" x="365" y="234">exports model/2</text>
  </g>
  <g class="cd-step">
    <rect x="512" y="154" width="250" height="88" rx="6"/>
    <text x="637" y="176">model.json</text>
    <text class="cd-sub" x="637" y="198">states, selections</text>
    <text class="cd-sub" x="637" y="216">units as text</text>
    <text class="cd-sub" x="637" y="234">Jacobian, with a reason</text>
  </g>
  <text class="cd-label" x="16" y="258">refused, each with the element which caused it</text>
  <g class="cd-step">
    <rect x="16" y="266" width="808" height="42" rx="6"/>
    <text class="cd-sub" x="420" y="284">a required package (comp) &#183; a fast reaction</text>
    <text class="cd-sub" x="420" y="300">an unknown identifier &#183; a cycle of assignment rules &#183; a draw of distrib</text>
  </g>
</svg>

## Algebraic rules

Models with algebraic rules use SUNDIALS IDA. The compiler matches each equation to
an eligible nonconstant variable and appends these unknowns to the state vector.
Variables determined by assignment rules, rate rules or reactions retain their
existing roles. An overdetermined algebraic system is a compilation error. IDA integrates
regular index-1 systems, whose algebraic equations have a nonsingular Jacobian
with respect to the matched unknowns along the solution.

Algebraic variables are determined by their constraints. Their SBML initial values
are guesses for the consistency solve; changes and pre-initialization target the
independent variables. Constraints are solved before initial output and event
triggers, and after changes and event assignments. Pure algebraic models are
supported as well. Initial assignments and species concentration conversions use
the consistent algebraic values.

Both analytic and difference Jacobians and the dense and KLU linear solvers are
available. IDA uses BDF regardless of the request's `stiff` setting. The optional
`n_algebraic` field of `model.json` gives the length of the algebraic suffix; it is
absent for ODE models.

## What the compiler reads

The reader (`crates/sbml`) takes SBML core of every level and version: Level 1 Versions 1 and 2, Level 2 Versions 1 to 5 and Level 3 Versions 1 and 2, all read into the model of Level 3, so nothing behind the reader knows the level of the file (the defaults of the older levels, their built-in units, their `<stoichiometryMath>` and the infix formulas of Level 1 included). A package with `required="false"` (layout, render, fbc annotations, ...) is ignored, and a package whose `required` attribute is `"true"` is refused (`the required package comp is not supported`): `comp` must be flattened before, a model read without its submodels is another model. A required package whose elements only annotate the model is read as well, its elements ignored as those of an optional one, so far `distrib`: it carries the uncertainty of a parameter (`<distrib:uncertainty>`, which the models of pkdb_models write for every fitted parameter: the confidence interval of the profile likelihood, the standard error of the Fisher information and the distribution of the draws), and the mathematics of the model is the same without it. The package is read by its namespace URI, never by its prefix, and only the elements of an uncertainty are read of it (`listOfUncertainties`, `uncertainty`, `listOfUncertParameters`, `uncertParameter`, `uncertSpan`, the whole of the package as libsbml implements it): any other element of that namespace is refused with its name (`<drawFromDistribution> of the namespace ... is not supported`), not ignored, so that no element which computes is dropped on the way to a deterministic simulation. A draw is therefore refused in either shape it has, whatever `required` says: the `<csymbol>` of a distribution (`http://www.sbml.org/sbml/symbols/distrib/normal`) in a math of the model is an unknown csymbol, as any math the reader does not understand, and the `<distrib:drawFromDistribution>` element of the drafts of the package is an element of the namespace which is no uncertainty. Units pass through as text and nothing is computed with them: a unit definition becomes a unit expression which a unit registry such as pint parses (`mmol/min` as `0.001*mole*(60*second)**-1`), the field `units` of `model.json`. The reader keeps the units of a definition as the document writes them as well (the kind, the exponent, the scale and the multiplier of every `<unit>`), which a conversion of the units of a model needs; the compiler itself uses the expression.

An unsupported feature is refused with its element, and so is a model which is not valid SBML (an unknown identifier, a cycle of assignment rules). In the SBML Test Suite the remaining refusals are the fast reactions (see [Test suite](testsuite.md)). Missing math is handled as libroadrunner 2.11 handles it: a rule or an initial assignment without math is ignored, an event without a trigger is ignored, a delay or a priority without math is none; a kinetic law without math, a call of a function definition without math and an empty `<math>` are refused.

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
- The model is converted to L3V2, which the compiler no longer needs, since the reader takes every level, and which keeps a prepared model comparable to the ones of earlier runs.
- The rest of the model (non-required packages such as `fbc` and `layout`, notes, annotations) is kept.

With `--bench bench/models.toml` the script prepares every model of the benchmark from its `source` into its `sbml`, with the absolute tolerance of every state as roadrunner scales the scalar one (`<name>.atol.json`), see [Running the benchmark](benchmarks.md#running-the-benchmark-against-roadrunner).

## The compiler in a browser

The compiler crates (`sbmlsim-abi`, `sbmlsim-sbml`, `sbmlsim-ir`, `sbmlsim-codegen`, `sbmlsim-compiler`) do no file system or OS access, so they build for `wasm32-unknown-unknown`, which CI checks:

```bash
cargo build -p sbmlsim-compiler --target wasm32-unknown-unknown --release
```

`sbmlsim-wasm` (`crates/wasm`) is that build with a C ABI, `compiler.wasm` of the release, and `sbmlsim-core/compile` its JavaScript host ([JavaScript](javascript.md#the-compiler)):

```js
import { createCompiler } from "sbmlsim-core/compile";

const compiler = await createCompiler({ compiler: compilerWasm });
const { wasm, description } = compiler.compile(sbmlText);
```

A page compiles a model of any level and version this way and runs it with the engine, so a model which a reader brings needs no server. The module is `sbmlsim-core/runtime/compiler.wasm` of the npm package, `compiler.wasm` of `sbmlsim-core runtime -o DIR` and the key `compiler` of `sbmlsim_core.runtime_wasm()` of the python package. The [live simulation](javascript.md#a-live-simulation-in-this-page) of this documentation compiles its SBML with it.
