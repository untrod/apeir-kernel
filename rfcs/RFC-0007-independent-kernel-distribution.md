# RFC-0007: Independent Kernel distribution boundary

Status: Accepted

## Context

The Kernel already has an independent Git history, Cargo workspace, daemon,
Provider worker, CLI, SDKs, contracts, tests, and release process. The missing
boundary was artifact consumption: product builds still expected a sibling
Kernel source checkout and built sidecars directly from that checkout.

Repository separation alone is not sufficient. A product must be able to
consume a specific Kernel release without importing Kernel crates, opening its
journal, or depending on its source-tree layout.

## Decision

`nous-kernel` is the authoritative project for Kernel contracts and binaries.
Every portable Kernel release publishes three binaries plus
`nous-kernel-release.json`:

- `nousd`: the single execution authority and NKI endpoint;
- `nous-provider-worker`: the isolated Provider execution boundary;
- `nous`: the diagnostic and developer CLI.

The versioned manifest records the target triple, Kernel version, supported NKI
range, Provider Contract major version, source revision/dirty state, artifact
roles, sizes, and SHA-256 digests. Its schema is
`spec/distribution/kernel-release-manifest-v1.schema.json`.

Desktop, CLI distributions, and other products consume this bundle as an
external component. Building directly from a sibling source checkout remains a
developer convenience, not the release architecture.

## Compatibility

Consumers must reject an unknown manifest schema, an incompatible NKI range,
missing roles, target mismatch, or digest mismatch. Additive optional fields are
allowed within schema v1. A required-field or semantic break requires schema v2
and migration guidance.

## Security

The manifest contains references and digests, never credentials. Hashes provide
integrity, not publisher identity; signature verification and release
provenance remain required before a public production release. Dirty source is
recorded and may be accepted for development builds but must be rejected by a
future release gate.

## Validation

The manifest writer has deterministic unit coverage. Platform package scripts
build with locked dependencies and generate hashes from the exact copied
binaries. Product-side verification will be tested independently at the
consumer boundary.

## References

- systemd cgroup delegation and the single-writer rule: https://systemd.io/CGROUP_DELEGATION/
- OCI Runtime lifecycle: https://github.com/opencontainers/runtime-spec/blob/main/runtime.md
- containerd Runtime v2 separation: https://github.com/containerd/containerd/blob/main/docs/runtime-v2.md
- Nomad plugin architecture: https://developer.hashicorp.com/nomad/plugins/author
- Wasmtime security model: https://docs.wasmtime.dev/security.html
