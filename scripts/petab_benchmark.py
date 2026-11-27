# /// script
# requires-python = ">=3.14"
# dependencies = [
#     "sbmlsim>=0.53.0",
#     "libroadrunner",
# ]
#
# [tool.uv.sources]
# libroadrunner = { index = "roadrunner-dev" }
#
# [[tool.uv.index]]
# name = "roadrunner-dev"
# url = "https://matthiaskoenig.github.io/roadrunner-wheels/simple/"
# explicit = true
# ///
"""The problems of the PEtab benchmark collection for the tests of `crates/petab`.

    uv run scripts/petab_benchmark.py download

Downloads the pinned commit of the PEtab benchmark collection (`COMMIT`, the `BENCHMARK_COMMIT`
of sbmlsim) with the log-likelihoods of AMICI and writes every problem in PEtab v2 into
`~/.cache/sbmlsim/petab-benchmark/<commit>/v2` (`SBMLSIM_BENCHMARK_PATH` names another
directory), where `crates/petab/tests/benchmark.rs` reads them. The download is the one of
sbmlsim, `sbmlsim.fit.petab_v2.benchmark.BenchmarkCollection.load`, into the cache sbmlsim uses
as well: the collection has every problem in PEtab v1, which is converted with
`petab.v2.petab1to2` of python together with the simulations of the collection, and a problem
which is in the cache already is not converted again.
"""

import argparse
import sys

from sbmlsim.fit.petab_v2.benchmark import BENCHMARK_COMMIT, BenchmarkCollection

#: the commit of the collection the tests of `crates/petab` compare with
COMMIT = "c44d5ede9bf979db5aa8ae4e94fc31fba9776675"


def download() -> int:
    """Download and convert the collection, once."""
    if BENCHMARK_COMMIT != COMMIT:
        print(
            f"error: the installed sbmlsim pins the collection at {BENCHMARK_COMMIT}, the "
            f"tests at {COMMIT}: update COMMIT here and in crates/petab/tests/benchmark.rs "
            "with the baseline deliberately",
            file=sys.stderr,
        )
        return 1
    collection = BenchmarkCollection.load(COMMIT)
    names = collection.problem_names()
    converted = [n for n in names if collection.problem(n).v2_dir.is_dir()]
    print(
        f"the PEtab benchmark collection {COMMIT} is in {collection.path}: "
        f"{len(converted)} of {len(names)} problems in PEtab v2"
    )
    return 0


def main() -> int:
    """Run the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("command", choices=["download"])
    parser.parse_args()
    return download()


if __name__ == "__main__":
    sys.exit(main())
