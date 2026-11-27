# sbmlsim-sbml

Reader of [SBML](https://sbml.org) core documents, every level and version, into the model of
level 3. It is the reader of [sbmlsim-core](https://github.com/matthiaskoenig/sbmlsim-core), the
simulation engine, and a crate of its own: it reads a document and gives a plain data model of
its elements and their MathML, nothing else.

- every level and version of SBML core: a document of level 1, 2 or 3 gives the same model, with
  the built-in units of level 1 and 2 defined and the infix formulas of level 1 parsed
- no file system and no network access, no `unsafe`, two dependencies
  ([roxmltree](https://crates.io/crates/roxmltree) and
  [thiserror](https://crates.io/crates/thiserror)), and it builds for
  `wasm32-unknown-unknown`
- it validates nothing: what the elements mean is the decision of what reads the model

```rust
let document = sbmlsim_sbml::read(
    r#"<?xml version="1.0" encoding="UTF-8"?>
       <sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
         <model id="decay">
           <listOfParameters>
             <parameter id="y" value="10" constant="false"/>
             <parameter id="k" value="0.1" constant="true"/>
           </listOfParameters>
           <listOfRules>
             <rateRule variable="y">
               <math xmlns="http://www.w3.org/1998/Math/MathML">
                 <apply><minus/><apply><times/><ci>k</ci><ci>y</ci></apply></apply>
               </math>
             </rateRule>
           </listOfRules>
         </model>
       </sbml>"#,
)?;
assert_eq!((document.level, document.version), (3, 2));
assert_eq!(document.model.parameters.len(), 2);
assert_eq!(document.model.rules.len(), 1);
# Ok::<(), sbmlsim_sbml::ReadError>(())
```

A composed model (the package `comp`) is flattened (since 0.40.0) into the model of SBML core it means, with
the ids libSBML gives the elements of its submodels (`submodel__id`); `read_with` reads the
documents of its external model definitions through a function of the caller, and
`flatten_comp` gives the flattened document as text. A document which requires another package
the reader does not know is refused instead of read in part: a model read without what the
package computes is another model.

## License

MIT, as the rest of sbmlsim-core.
