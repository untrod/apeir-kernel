#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
cargo test --workspace --locked
cargo build -p nousd -p nous-provider-worker --locked
python3 tests/e2e/kernel_e2e.py
python3 tests/test_release_manifest.py -v
PYTHONPATH="$(pwd)/sdk/python/src" python3 -m unittest discover -s sdk/python/tests -v
mkdir -p target/native-tests
cc -std=c11 -Wall -Wextra -Werror -I sdk/c/include sdk/c/src/nous_kernel.c sdk/c/tests/test_nous_kernel.c -o target/native-tests/nous-c-sdk-test
target/native-tests/nous-c-sdk-test
cc -std=c11 -Wall -Wextra -Werror -I micro/include micro/src/nous_micro.c micro/tests/test_micro.c -o target/native-tests/nous-micro-test
target/native-tests/nous-micro-test
