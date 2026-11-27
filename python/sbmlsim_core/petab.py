"""PEtab v2 problems read, simulated and fitted in Rust (since 0.38.0).

The crate `sbmlsim-petab` reads, simulates and fits them.

A problem is read from its YAML file and its tables, the math of its observables, noise
formulas and conditions is compiled into its model, every experiment is one simulation of the
engine and the log-likelihood, `chi2` and the log priors are the ones of the specification of
PEtab v2:

```python
from sbmlsim_core import petab

problem = petab.Problem.from_yaml("problem.yaml")
evaluation = problem.evaluate()  # at the nominal values of the parameter table
print(evaluation["llh"], evaluation["chi2"])
answer = problem.fit({"algorithm": "trust_region", "max_iterations": 200})
print(dict(zip(problem.parameters, answer["x"])), answer["objective"])
```

See the page PEtab of the documentation for what is supported.
"""

from sbmlsim_core._core import PetabProblem as Problem

__all__ = ["Problem"]
