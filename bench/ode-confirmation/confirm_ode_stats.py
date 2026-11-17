#!/usr/bin/env python3
"""Analyze the completed fixed process experiment; effects are feature/baseline."""

import argparse
import json
from pathlib import Path

import numpy as np


def process_effect(records, names, reverse):
    """Return one normalized log-time effect per case in a process."""
    rows = {r["name"]: r for r in records}
    if len(records) != len(names) or set(names) != set(rows):
        raise ValueError("missing, duplicated, or unexpected cases")
    effects = []
    for name in names:
        r = rows[name]
        a, b = (
            np.asarray(r[k], dtype=float)
            for k in ("baseline_samples_s", "feature_samples_s")
        )
        if any(x.shape != (6,) or not np.all(np.isfinite(x) & (x > 0)) for x in (a, b)):
            raise ValueError(f"invalid samples for {name}")
        log_ratios = np.log(b.reshape(3, 2).sum(axis=1) / a.reshape(3, 2).sum(axis=1))
        effects.append(float(np.median(-log_ratios if reverse else log_ratios)))
    return effects


def family(names, forward, reverse):
    """Estimate ratios and simultaneous intervals for one experiment family."""
    groups = [np.array(forward), np.array(reverse)]
    means = np.stack([x.mean(axis=0) for x in groups])
    center = means.mean(axis=0)
    se = np.sqrt(sum(x.var(axis=0, ddof=1) / len(x) for x in groups)) / 2
    variable = se > 0
    rng = np.random.default_rng(29)
    maxima = []
    for _ in range(100):
        draws = (
            sum(
                x[rng.integers(len(x), size=(500, len(x)))].mean(axis=1) for x in groups
            )
            / 2
        )
        if variable.any():
            maxima.extend(
                np.abs((draws[:, variable] - center[variable]) / se[variable]).max(
                    axis=1
                )
            )
    critical = float(np.quantile(maxima, 0.95)) if maxima else 0.0
    return [
        {
            "name": name,
            "ratio": float(np.exp(center[i])),
            "forward": float(np.exp(means[0, i])),
            "reverse": float(np.exp(means[1, i])),
            "ci95_simultaneous": np.exp(
                [center[i] - critical * se[i], center[i] + critical * se[i]]
            ).tolist(),
            "zero_observed_variation": not bool(variable[i]),
        }
        for i, name in enumerate(names)
    ]


def main(directory):
    """Validate and analyze the completed comparison and control trials."""
    schedule = json.loads((directory / "schedule.json").read_text())
    names = sorted(schedule[0]["cases"])
    groups = {
        k: []
        for k in [
            ("AB", "forward"),
            ("AB", "reverse"),
            ("AA", "forward"),
            ("AA", "reverse"),
        ]
    }
    if len(schedule) != 32 or len(names) != 24:
        raise ValueError("expected 32 trials and 24 cases")
    for t in schedule:
        index, kind, orientation = t["index"], t["kind"], t["orientation"]
        file = directory / f"trial-{index:02d}-{kind}-{orientation}.json"
        groups[kind, orientation].append(
            process_effect(
                json.loads(file.read_text()), names, orientation == "reverse"
            )
        )
    if [len(x) for x in groups.values()] != [12, 12, 4, 4]:
        raise ValueError("incomplete or unbalanced experiment")
    return {
        kind: family(names, groups[kind, "forward"], groups[kind, "reverse"])
        for kind in ["AB", "AA"]
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    print(json.dumps(main(parser.parse_args().out), indent=2, allow_nan=False))
