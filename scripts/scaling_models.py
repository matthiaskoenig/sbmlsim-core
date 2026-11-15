# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Write the models of the scaling benchmark of sbmlsim-core (the threshold of KLU).

    uv run scripts/scaling_models.py --out target/bench-models/scaling
    sbmlsim-core bench target/bench-models/scaling/scaling.toml --jacobians analytic --json ...

Two families of SBML L3V2 models of a growing number of states, which `sbmlsim-core bench`
simulates with the dense solver and with KLU to find the number of states above which KLU is
faster (`KLU_THRESHOLD` of `crates/engine/src/cvode.rs`, bench/RESULTS.md):

- `chain_<n>`: a chain of `n / 4` units as the sinusoidal units of the machine perfusion model,
  each with a substrate `A` and a product `D` carried by the flow from the unit before, a fast
  reversible Michaelis-Menten conversion `A <-> B`, `B + C -> D` and a slow regeneration
  `D -> C` (a banded Jacobian, about 3 entries per row, stiff).
- `star_<n>`: a whole body model as the PBPK models (HCTZ, midazolam): a venous pool and an
  arterial pool coupled to `(n - 2) / 3` organs, each with its blood, its tissue (exchanged by a
  partition) and a metabolite formed by Michaelis-Menten kinetics (an arrow head Jacobian: the
  pools couple to every organ).

Every model is one compartment of size 1 (the absolute tolerance is the scalar), simulated from
0 to 50 with 201 output points, rtol 1e-6 and atol 1e-12 as the models of `bench/models.toml`.
The file `scaling.toml` lists them for `sbmlsim-core bench`.
"""

from __future__ import annotations

import argparse
from pathlib import Path

HEADER = """<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="{id}">
    <listOfCompartments>
      <compartment id="cell" spatialDimensions="3" size="1" constant="true"/>
    </listOfCompartments>
"""


def species(sid: str, initial: float, boundary: bool = False) -> str:
    """A species of the compartment."""
    flag = "true" if boundary else "false"
    return (
        f'      <species id="{sid}" compartment="cell" initialConcentration="{initial}" '
        f'hasOnlySubstanceUnits="false" boundaryCondition="{flag}" constant="{flag}"/>\n'
    )


def parameter(pid: str, value: float) -> str:
    """A constant parameter."""
    return f'      <parameter id="{pid}" value="{value}" constant="true"/>\n'


def math(mathml: str) -> str:
    """A MathML element."""
    return f'<math xmlns="http://www.w3.org/1998/Math/MathML">{mathml}</math>'


def ci(x: str) -> str:
    """An identifier."""
    return f"<ci> {x} </ci>"


def apply(op: str, *args: str) -> str:
    """An operation."""
    return f"<apply><{op}/>{''.join(args)}</apply>"


def mm(vmax: str, km: str, s: str) -> str:
    """Michaelis-Menten `vmax * s / (km + s)`."""
    return apply("divide", apply("times", ci(vmax), ci(s)), apply("plus", ci(km), ci(s)))


def reaction(rid: str, reactants: list[str], products: list[str], law: str) -> str:
    """An irreversible reaction (or a reversible one when the law can be negative)."""
    def refs(tag: str, ids: list[str]) -> str:
        if not ids:
            return ""
        inner = "".join(
            f'<speciesReference species="{s}" stoichiometry="1" constant="true"/>' for s in ids
        )
        return f"<{tag}>{inner}</{tag}>"

    return (
        f'      <reaction id="{rid}" reversible="true">'
        f"{refs('listOfReactants', reactants)}{refs('listOfProducts', products)}"
        f"<kineticLaw>{math(law)}</kineticLaw></reaction>\n"
    )


def chain(n_units: int) -> str:
    """The chain of `n_units` units, `4 n_units` states."""
    out = [HEADER.format(id=f"chain_{4 * n_units}"), "    <listOfSpecies>\n"]
    out.append(species("A_in", 1.0, boundary=True))
    out.append(species("D_in", 0.0, boundary=True))
    for i in range(n_units):
        out += [species(f"A{i}", 0.0), species(f"B{i}", 0.0), species(f"C{i}", 1.0), species(f"D{i}", 0.0)]
    out.append("    </listOfSpecies>\n    <listOfParameters>\n")
    for pid, value in [("q", 0.2), ("Vf", 50.0), ("Vr", 20.0), ("Km", 0.5), ("k2", 5.0), ("V3", 0.5), ("K3", 0.2)]:
        out.append(parameter(pid, value))
    out.append("    </listOfParameters>\n    <listOfReactions>\n")
    for i in range(n_units):
        a_prev, d_prev = ("A_in", "D_in") if i == 0 else (f"A{i - 1}", f"D{i - 1}")
        out.append(reaction(f"flowA{i}", [a_prev] if i else [], [f"A{i}"], apply("times", ci("q"), ci(a_prev))))
        out.append(reaction(f"flowD{i}", [d_prev] if i else [], [f"D{i}"], apply("times", ci("q"), ci(d_prev))))
        out.append(reaction(f"conv{i}", [f"A{i}"], [f"B{i}"], apply("minus", mm("Vf", "Km", f"A{i}"), mm("Vr", "Km", f"B{i}"))))
        out.append(reaction(f"bind{i}", [f"B{i}", f"C{i}"], [f"D{i}"], apply("times", ci("k2"), ci(f"B{i}"), ci(f"C{i}"))))
        out.append(reaction(f"regen{i}", [f"D{i}"], [f"C{i}"], mm("V3", "K3", f"D{i}")))
    last = n_units - 1
    out.append(reaction("outA", [f"A{last}"], [], apply("times", ci("q"), ci(f"A{last}"))))
    out.append(reaction("outD", [f"D{last}"], [], apply("times", ci("q"), ci(f"D{last}"))))
    out.append("    </listOfReactions>\n  </model>\n</sbml>\n")
    return "".join(out)


def star(n_organs: int) -> str:
    """The whole body model of `n_organs` organs, `3 n_organs + 2` states."""
    out = [HEADER.format(id=f"star_{3 * n_organs + 2}"), "    <listOfSpecies>\n"]
    # an initial venous concentration of 1 instead of a dose
    out += [species("Cve", 1.0), species("Car", 0.0)]
    for i in range(n_organs):
        out += [species(f"Cb{i}", 0.0), species(f"Ct{i}", 0.0), species(f"M{i}", 0.0)]
    out.append("    </listOfSpecies>\n    <listOfParameters>\n")
    out += [parameter("Qlung", 50.0), parameter("CLr", 0.3)]
    for i in range(n_organs):
        out += [
            parameter(f"Q{i}", 0.5 + (i % 7) * 0.4),
            parameter(f"PS{i}", 1.0 + (i % 5) * 2.0),
            parameter(f"Kp{i}", 0.5 + (i % 3) * 1.5),
            parameter(f"Vm{i}", 0.1 * (i % 4)),
            parameter(f"Km{i}", 0.05),
            parameter(f"ke{i}", 0.2),
        ]
    out.append("    </listOfParameters>\n    <listOfReactions>\n")
    out.append(reaction("lung", ["Cve"], ["Car"], apply("times", ci("Qlung"), apply("minus", ci("Cve"), ci("Car")))))
    out.append(reaction("renal", ["Car"], [], apply("times", ci("CLr"), ci("Car"))))
    for i in range(n_organs):
        out.append(reaction(f"in{i}", ["Car"], [f"Cb{i}"], apply("times", ci(f"Q{i}"), ci("Car"))))
        out.append(reaction(f"out{i}", [f"Cb{i}"], ["Cve"], apply("times", ci(f"Q{i}"), ci(f"Cb{i}"))))
        out.append(
            reaction(
                f"part{i}",
                [f"Cb{i}"],
                [f"Ct{i}"],
                apply("times", ci(f"PS{i}"), apply("minus", ci(f"Cb{i}"), apply("divide", ci(f"Ct{i}"), ci(f"Kp{i}")))),
            )
        )
        out.append(reaction(f"met{i}", [f"Ct{i}"], [f"M{i}"], mm(f"Vm{i}", f"Km{i}", f"Ct{i}")))
        out.append(reaction(f"elim{i}", [f"M{i}"], [], apply("times", ci(f"ke{i}"), ci(f"M{i}"))))
    out.append("    </listOfReactions>\n  </model>\n</sbml>\n")
    return "".join(out)


CHAIN_UNITS = [1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16, 20, 24, 32, 48, 64, 96, 128]
STAR_ORGANS = [1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 20, 24, 32, 42, 64, 84, 128, 170]


def main() -> int:
    """Run the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path("target/bench-models/scaling"))
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    toml = ["# the scaling benchmark, written by scripts/scaling_models.py\n"]
    models = [(f"chain_{4 * u}", chain(u)) for u in CHAIN_UNITS]
    models += [(f"star_{3 * o + 2}", star(o)) for o in STAR_ORGANS]
    for name, sbml in models:
        (args.out / f"{name}.xml").write_text(sbml, encoding="utf-8")
        toml.append(
            f'\n[models.{name}]\nsource = "scripts/scaling_models.py"\nsbml = "{name}.xml"\n'
            "end = 50.0\npoints = 201\nrelative_tolerance = 1e-6\nabsolute_tolerance = 1e-12\n"
        )
    (args.out / "scaling.toml").write_text("".join(toml), encoding="utf-8")
    print(f"{len(models)} models in {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
