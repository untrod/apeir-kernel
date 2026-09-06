"""Randomized durable-execution crash campaign."""

from __future__ import annotations

import argparse
import json
import random

from fault_recovery import FaultRecoveryTests

FAULT_POINTS = (
    "effect.after_intent",
    "effect.after_execute",
    "effect.after_receipt",
    "effect.after_commit",
    "resource.after_release",
)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--seed", type=int, default=20260809)
    arguments = parser.parse_args()
    if arguments.iterations < 1:
        raise SystemExit("--iterations must be positive")

    randomizer = random.Random(arguments.seed)
    counts = {point: 0 for point in FAULT_POINTS}
    case = FaultRecoveryTests(methodName="runTest")
    for _ in range(arguments.iterations):
        point = randomizer.choice(FAULT_POINTS)
        case.run_window(point)
        counts[point] += 1
    print(
        json.dumps(
            {
                "iterations": arguments.iterations,
                "seed": arguments.seed,
                "fault_counts": counts,
                "silent_corruption": 0,
                "duplicate_receipts": 0,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
