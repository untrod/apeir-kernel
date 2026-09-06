#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked -p nousd -p nous-provider-worker -p nous-cli
mkdir -p target/distribution
cp target/release/nousd target/release/nous-provider-worker target/release/nous target/distribution/
cp LICENSE NOTICE THIRD_PARTY_NOTICES README.md README.zh-CN.md DISTRIBUTION.md target/distribution/
cp spec/distribution/kernel-release-manifest-v1.schema.json target/distribution/
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
python3 scripts/write_release_manifest.py \
  --output target/distribution/nous-kernel-release.json \
  --target "$TARGET" \
  --artifact "daemon=target/distribution/nousd" \
  --artifact "provider-worker=target/distribution/nous-provider-worker" \
  --artifact "cli=target/distribution/nous"
(cd target/distribution && sha256sum \
  nousd nous-provider-worker nous \
  LICENSE NOTICE THIRD_PARTY_NOTICES README.md README.zh-CN.md DISTRIBUTION.md \
  kernel-release-manifest-v1.schema.json nous-kernel-release.json > SHA256SUMS)
VERSION="$(python3 -c 'import json; print(json.load(open("target/distribution/nous-kernel-release.json", encoding="utf-8"))["version"])')"
mkdir -p target/packages
tar -czf "target/packages/nous-kernel-${VERSION}-${TARGET}.tar.gz" -C target/distribution .
(cd target/packages && sha256sum "nous-kernel-${VERSION}-${TARGET}.tar.gz" > "nous-kernel-${VERSION}-${TARGET}.tar.gz.sha256")
printf '%s\n' "Portable Kernel bundle written to target/distribution"
printf '%s\n' "Portable Kernel archive written to target/packages/nous-kernel-${VERSION}-${TARGET}.tar.gz"
printf '%s\n' "Portable Kernel archive checksum written to target/packages/nous-kernel-${VERSION}-${TARGET}.tar.gz.sha256"
