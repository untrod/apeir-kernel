#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
fault_iterations="${NOUS_FAULT_ITERATIONS:-1}"
soak_seconds="${NOUS_SOAK_SECONDS:-30}"
random_faults="${NOUS_RANDOM_FAULTS:-0}"
sample_seconds="${NOUS_SOAK_SAMPLE_SECONDS:-30}"
restart_every="${NOUS_SOAK_RESTART_EVERY:-250}"
evidence_path="${NOUS_SOAK_EVIDENCE_PATH:-}"
cargo build -p nousd --features fault-injection -p nous-provider-worker --locked
NOUS_FAULT_ITERATIONS="$fault_iterations" python3 tests/e2e/fault_recovery.py
if [ "$random_faults" -gt 0 ]; then
  python3 tests/e2e/crash_campaign.py --iterations "$random_faults"
fi
python3 tests/e2e/kernel_e2e.py
if [ -n "$evidence_path" ]; then
  python3 tests/e2e/soak.py --seconds "$soak_seconds" --sample-seconds "$sample_seconds" --restart-every "$restart_every" --output "$evidence_path"
else
  python3 tests/e2e/soak.py --seconds "$soak_seconds" --sample-seconds "$sample_seconds" --restart-every "$restart_every"
fi
