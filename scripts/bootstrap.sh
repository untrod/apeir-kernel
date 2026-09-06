#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
command -v git >/dev/null
command -v cargo >/dev/null
command -v rustc >/dev/null
command -v python3 >/dev/null
cargo --version
python3 --version
python3 -m ruff --version
git --version
