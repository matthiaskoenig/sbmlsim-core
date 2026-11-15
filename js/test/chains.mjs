// The generated SBML of the large synthetic model (`large.test.mjs`).

const MATH = 'xmlns="http://www.w3.org/1998/Math/MathML"';

/** SBML of `chains` chains of `length` species, `S<c>_<i> <-> S<c>_<i+1>` by mass action. */
export function chainModel(chains, length) {
  const species = [];
  const reactions = [];
  for (let c = 0; c < chains; c += 1) {
    for (let i = 0; i < length; i += 1) {
      species.push(
        `<species id="S${c}_${i}" compartment="cell" initialConcentration="${i === 0 ? 1 : 0}" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>`,
      );
      if (i + 1 < length) {
        reactions.push(
          `<reaction id="R${c}_${i}" reversible="true"><listOfReactants><speciesReference species="S${c}_${i}" stoichiometry="1" constant="true"/></listOfReactants>` +
            `<listOfProducts><speciesReference species="S${c}_${i + 1}" stoichiometry="1" constant="true"/></listOfProducts>` +
            `<kineticLaw><math ${MATH}><apply><times/><ci>cell</ci><apply><minus/><apply><times/><ci>kf</ci><ci>S${c}_${i}</ci></apply><apply><times/><ci>kr</ci><ci>S${c}_${i + 1}</ci></apply></apply></apply></math></kineticLaw></reaction>`,
        );
      }
    }
  }
  return `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="chains">
<listOfCompartments><compartment id="cell" size="1" constant="true"/></listOfCompartments>
<listOfSpecies>${species.join("\n")}</listOfSpecies>
<listOfParameters><parameter id="kf" value="2" constant="true"/><parameter id="kr" value="0.5" constant="true"/></listOfParameters>
<listOfReactions>${reactions.join("\n")}</listOfReactions>
</model></sbml>`;
}
