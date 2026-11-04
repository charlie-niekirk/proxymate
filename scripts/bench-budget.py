#!/usr/bin/env python3
"""Fails when a criterion bench's mean is over its budget (R2).

Run `cargo bench -p pm-core --bench ingest` first (CI sets PM_BENCH_QUICK=1),
then `scripts/bench-budget.py [crates/pm-core/benches/budget.json]`.
The budgets are ceilings for a CI runner, not the targets in docs/PLAN.md:
they catch a regression of the kind R2 fixed, not a few percent of noise.
"""

import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def criterion_dir():
    target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
    return os.path.join(target, "criterion")


def mean_ms(bench):
    group, name = bench.split("/", 1)
    path = os.path.join(criterion_dir(), group, name, "new", "estimates.json")
    try:
        with open(path) as f:
            return json.load(f)["mean"]["point_estimate"] / 1e6
    except FileNotFoundError:
        return None


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "crates/pm-core/benches/budget.json")
    with open(path) as f:
        budgets = {k: v for k, v in json.load(f).items() if not k.startswith("_")}
    failed = False
    width = max(len(b) for b in budgets)
    print(f"{'bench':<{width}}  {'mean':>10}  {'budget':>10}")
    for bench, budget in budgets.items():
        mean = mean_ms(bench)
        limit = budget["max_ms"]
        if mean is None:
            print(f"{bench:<{width}}  {'missing':>10}  {limit:>8.1f}ms  FAIL")
            failed = True
            continue
        over = mean > limit
        failed |= over
        status = "FAIL" if over else "ok"
        print(f"{bench:<{width}}  {mean:>8.1f}ms  {limit:>8.1f}ms  {status}")
    if failed:
        print("\nA bench is over its budget (or didn't run). See crates/pm-core/benches/budget.json.")
        sys.exit(1)


if __name__ == "__main__":
    main()
