"""Freeze, measure and compare ordinary ODE performance across the DDE change.

Run freeze before timing. Run measure once with the release feature benchmark
driver. The output directory holds every input, trial and raw timing sample.
"""

import argparse
import hashlib
import importlib.util
import json
import math
import os
import random
import statistics
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SUPPORT = ROOT / "bench" / "dde-performance"
STATS = ROOT / "bench" / "ode-confirmation" / "confirm_ode_stats.py"
BASELINE = ROOT / "target" / "dde-baseline"
FILES = ("model.wasm", "model.json")
RUNTIME = ("engine.wasm", "mathlib.wasm")
SEED = 29


def digest(path):
    """Return the SHA-256 of one frozen file."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, data):
    """Write deterministic, finite JSON."""
    path.write_text(json.dumps(data, indent=2, sort_keys=True, allow_nan=False) + "\n")


def schedule(names):
    """Balance comparison and control orientations in randomized groups."""
    rng = random.Random(SEED)
    ab = ["forward", "reverse"] * 12
    aa = ["forward", "reverse"] * 4
    rng.shuffle(ab)
    rng.shuffle(aa)
    trials = []
    for _group in range(8):
        batch = [("AB", ab.pop()) for _ in range(3)] + [("AA", aa.pop())]
        rng.shuffle(batch)
        for family, orientation in batch:
            order = list(names)
            rng.shuffle(order)
            hosts = ["native", "wasm"]
            rng.shuffle(hosts)
            trials.append(
                {
                    "family": family,
                    "orientation": orientation,
                    "order": order,
                    "hosts": hosts,
                }
            )
    return trials


def baseline_files():
    """List the driver, compiler and runtime used for the baseline."""
    return [BASELINE / name for name in ("sbmlsim-core", "ode_performance")] + [
        BASELINE / "runtime" / name for name in RUNTIME
    ]


def freeze(out, cpu):
    """Compile baseline models and freeze the matrix and protocol."""
    if out.exists():
        raise ValueError(f"input directory already exists: {out}")
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=out.parent) as tmp:
        work = Path(tmp) / "inputs"
        work.mkdir()
        archive = json.loads((ROOT / "bench/ode-performance-data.json").read_text())
        sources = {}
        for item in archive["model_artifacts"]:
            name = item["directory"]
            model = work / "models" / name
            model.mkdir(parents=True)
            if "sbml" in item:
                source = model / "source.xml"
                source.write_text(item["sbml"])
            elif name in ("hctz", "midazolam"):
                source = ROOT / f"target/bench-models/sbmlsim-{name}.xml"
            else:
                source = ROOT / item["sbml_source"]
            sources[name] = digest(source)
            subprocess.run(
                [BASELINE / "sbmlsim-core", "compile", source, "-o", model], check=True
            )
            desc = json.loads((model / "model.json").read_text())
            if desc["id"] != item["id"] or len(desc["states"]) != item["states"]:
                raise ValueError(f"wrong benchmark model for {name}")
        jobs = archive["scenarios"]
        for job in jobs:
            job["directory"] = "models/" + job["directory"]
        write_json(work / "jobs.json", jobs)
        names = [j["name"].replace("/wasm/", "/") for j in jobs if not j["native"]]
        native = [j["name"].replace("/native/", "/") for j in jobs if j["native"]]
        if (
            len(names) != 24
            or set(names) != set(native)
            or len(set(native)) != len(native)
        ):
            raise ValueError("expected 24 matching scenarios per host")
        write_json(work / "schedule.json", schedule(names))
        inputs = [path for path in work.rglob("*") if path.is_file()]
        manifest = {
            "baseline_commit": "d904f63c03d1c99d05e75ffdfec90b2c226db8be",
            "cpu": cpu,
            "source_sha256": sources,
            "inputs": {str(p.relative_to(work)): digest(p) for p in inputs},
            "repository_files": {
                str(p.relative_to(ROOT)): digest(p)
                for p in [
                    *baseline_files(),
                    STATS,
                    SUPPORT / "protocol.md",
                    Path(__file__).resolve(),
                ]
            },
        }
        write_json(work / "manifest.json", manifest)
        work.rename(out)


def measure(out, feature, results):
    """Measure each paired trial and analyze all 48 scenarios together."""
    manifest = json.loads((out / "manifest.json").read_text())
    for base, group in [(out, "inputs"), (ROOT, "repository_files")]:
        for name, expected in manifest[group].items():
            if digest(base / name) != expected:
                raise ValueError(f"changed frozen input: {name}")
    results.mkdir(parents=True, exist_ok=False)
    feature_runtime = results / "feature-runtime"
    subprocess.run([feature, "runtime", feature_runtime], check=True)
    write_json(
        results / "feature.json",
        {
            "driver": digest(feature),
            "inputs": digest(out / "manifest.json"),
            "runtime": {f: digest(feature_runtime / f) for f in RUNTIME},
        },
    )
    trials = json.loads((out / "schedule.json").read_text())
    jobs = json.loads((out / "jobs.json").read_text())
    env = dict(
        os.environ,
        SBMLSIM_ODE_BENCH_BLOCKS="3",
        SBMLSIM_ODE_BENCH_CACHE=str(results / "cache"),
    )
    baseline = BASELINE / "ode_performance"
    cpu = str(manifest["cpu"])
    reference = {}
    effects = {}
    for index, trial in enumerate(trials, 1):
        per_host = {}
        for host in trial["hosts"]:
            prefix = results / f"trial-{index:02}-{host}"
            selected = {
                j["name"].replace(f"/{host}/", "/"): j
                for j in jobs
                if j["native"] == (host == "native")
            }
            ordered = [
                dict(selected[name], directory=str(out / selected[name]["directory"]))
                for name in trial["order"]
            ]
            jobfile = prefix.with_suffix(".jobs.json")
            write_json(jobfile, ordered)
            reverse = trial["orientation"] == "reverse"
            is_ab = trial["family"] == "AB"

            def invoke(args, label, prefix=prefix, ordered=ordered):
                path = prefix.with_suffix(f".{label}.json")
                with path.open("w") as data, path.with_suffix(".log").open("w") as log:
                    subprocess.run(
                        ["taskset", "-c", cpu, *args],
                        env=env,
                        stdout=data,
                        stderr=log,
                        check=True,
                    )
                records = json.loads(path.read_text())
                if [r["name"] for r in records] != [j["name"] for j in ordered]:
                    raise ValueError(f"unexpected scenarios in {path}")
                return records

            if host == "wasm":
                runtimes = [
                    BASELINE / "runtime",
                    feature_runtime if is_ab else BASELINE / "runtime",
                ]
                if reverse:
                    runtimes.reverse()
                rows = invoke([feature, "paired", jobfile, *runtimes], "paired")
                per_host[host] = {
                    r["name"]: effect
                    for r, effect in zip(
                        rows,
                        stats.process_effect(rows, [r["name"] for r in rows], reverse),
                        strict=True,
                    )
                }
            else:
                binaries = [baseline, feature if is_ab else baseline]
                if reverse:
                    binaries.reverse()
                first, second = [
                    invoke([binary, jobfile], str(i))
                    for i, binary in enumerate(binaries)
                ]
                per_host[host] = {}
                for a, b in zip(first, second, strict=True):
                    for row in (a, b):
                        values = row["samples_s"]
                        fingerprint = [row["result_hash"], row["stats"]]
                        if (
                            len(row["stats"]) != 18
                            or reference.setdefault(row["name"], fingerprint)
                            != fingerprint
                        ):
                            raise ValueError(
                                f"native result bits or counters differ: {row['name']}"
                            )
                        if len(values) != 9 or not all(
                            math.isfinite(v) and v > 0 for v in values
                        ):
                            raise ValueError("invalid native samples")
                    effect = math.log(
                        statistics.median(b["samples_s"])
                        / statistics.median(a["samples_s"])
                    )
                    per_host[host][a["name"]] = -effect if reverse else effect
        names = sorted([*per_host["native"], *per_host["wasm"]])
        effects.setdefault((trial["family"], trial["orientation"]), []).append(
            [
                per_host["native" if "/native/" in name else "wasm"][name]
                for name in names
            ]
        )
        print(f"finished trial {index}/{len(trials)}", flush=True)
    summary = {
        k: stats.family(names, effects[k, "forward"], effects[k, "reverse"])
        for k in ("AB", "AA")
    }
    summary["native_signatures"] = reference
    summary["strict_zero_overhead_supported"] = all(
        row["ci95_simultaneous"][1] <= 1 and not row["zero_observed_variation"]
        for row in summary["AB"]
    ) and all(
        row["ci95_simultaneous"][0] <= 1 <= row["ci95_simultaneous"][1]
        and not row["zero_observed_variation"]
        for row in summary["AA"]
    )
    write_json(results / "summary.json", summary)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "measure"))
    parser.add_argument("--out", type=Path, default=ROOT / "target/dde-perf")
    parser.add_argument(
        "--results", type=Path, default=ROOT / "target/dde-perf-results"
    )
    parser.add_argument(
        "--feature", type=Path, default=ROOT / "target/release/examples/ode_performance"
    )
    parser.add_argument("--cpu", type=int, default=0)
    args = parser.parse_args()
    if args.action == "freeze":
        freeze(args.out.resolve(), args.cpu)
    else:
        spec = importlib.util.spec_from_file_location("ode_stats", STATS)
        stats = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(stats)
        measure(args.out.resolve(), args.feature.resolve(), args.results.resolve())
