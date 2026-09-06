#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
python3 -m compileall -q sdk/python/src sdk/python/tests tests/e2e scripts
python3 -m ruff check sdk/python tests scripts
python3 scripts/architecture_check.py
python3 scripts/release_audit.py --output .local/audit
if [ -d .git ]; then
    git diff --check
fi
