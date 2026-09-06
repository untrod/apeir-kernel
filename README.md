# APEIR Kernel

[![CI](https://github.com/kicoyini45-blip/apeir-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/kicoyini45-blip/apeir-kernel/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

APEIR Kernel is a systems-oriented runtime for governed execution across local
processes, model providers, algorithms, and device adapters. It provides the
control path that decides whether work may run, selects an eligible target,
records resource ownership, and commits the result to durable state.

The project is deliberately separate from the APEIR distribution. Desktop
interfaces, model accounts, document tools, agent workflows, marketplaces, and
domain integrations use the kernel through public contracts; they are not part
of its trusted computing base.

[中文说明](README.zh-CN.md)

## Status

APEIR Kernel 0.1 is a pre-release for development and evaluation on a trusted
local host. The Windows 10 x64 path is covered by the locked Rust workspace,
daemon/worker process tests, Python and C SDK tests, and Micro host tests.
Other platforms and external providers require qualification against the exact
release revision.

The NKI listener is loopback-only. Remote multi-host control and hostile
multi-tenant isolation are not supported. See the
[support matrix](docs/reference/support-matrix.md) and
[security policy](SECURITY.md) before deployment.

## What the kernel provides

- a versioned Kernel Interface (NKI) with request identity, deadlines,
  authentication tokens, and structured errors;
- workload admission with capability, safety, resource, and executor checks;
- deterministic placement and recorded decision traces;
- resource leases protected by fencing tokens;
- an append-only journal with integrity checks, replay, and recovery;
- provider execution in supervised child processes with bounded cancellation
  and timeout handling;
- normalized receipts, idempotent commit, and delivery-aware recovery;
- extension admission, approval binding, execution permits, and revocation;
- Rust, Python, and C clients plus a fixed-capacity Micro C implementation;
- machine-readable contracts, conformance fixtures, and release tooling.

## Execution model

Every state-changing request follows one authoritative path:

```text
CLI / SDK / APEIR distribution
               |
               v
              NKI
               |
               v
admission -> scheduling -> fenced lease -> durable intent
          -> isolated provider -> receipt -> commit -> lease release
```

The journal is the durable source of execution state. In-memory queues and
indexes are rebuildable projections. Providers cannot write the journal, mint
leases, grant capabilities, or commit their own results.

## Build and test

Requirements:

- Rust stable with Cargo;
- Python 3.10 or later;
- a C11 compiler;
- Visual Studio 2022 Build Tools with the C++ workload on Windows.

Windows PowerShell:

```powershell
./scripts/bootstrap.ps1
./scripts/build.ps1
./scripts/test.ps1
```

Linux and other POSIX systems:

```sh
./scripts/bootstrap.sh
./scripts/build.sh
./scripts/test.sh
```

`Cargo.lock` pins the Rust dependency graph. The test scripts cover the Rust
workspace, daemon/worker process boundary, Python SDK, C SDK, and Micro host
implementation.

## Run locally

Start the daemon with the deterministic reference worker:

```powershell
target/debug/apeird serve .local/kernel.db target/debug/nous-provider-worker
```

In another terminal:

```powershell
target/debug/apeir-kernelctl doctor
target/debug/apeir-kernelctl run "hello" --backend reference
target/debug/apeir-kernelctl inspect
```

The reference backend requires no model account or network service. For an
application-managed session, set the same random `NOUS_NKI_TOKEN` of at least
32 characters in the daemon and client environments.

## Repository map

| Path | Responsibility |
| --- | --- |
| `crates/nous-types` | Shared workload, resource, provider, and open-runtime contracts |
| `crates/nous-state` | Journal, integrity verification, replay, and durable projections |
| `crates/nous-security` | Identity, capability grants, and safety validation |
| `crates/nous-resource` | Resource admission, leases, and fencing |
| `crates/nous-scheduler` | Feasibility, deterministic placement, and decision traces |
| `crates/nous-kernel-core` | Workload lifecycle, recovery, cancellation, and provider supervision |
| `crates/nous-nki` | NKI envelopes, methods, framing, and version handling |
| `crates/nous-execution-proof` | Effect records and execution-proof structures |
| `crates/nous-runtime-client` | Rust NKI client |
| `crates/nous-intelligence` | Declarative model, graph, dataset, and project validation |
| `crates/nous-control-plane` | Versioned catalog for declarative control assets |
| `daemon/nousd` | `apeird` daemon and `nousd` compatibility executable |
| `providers/worker` | One-shot process host for reference and adapter backends |
| `tools/nous` | `apeir-kernelctl` and compatibility CLI |
| `spec` | Versioned JSON contracts and compatibility declarations |
| `sdk`, `micro` | Language clients and constrained-target implementation |
| `tests`, `conformance` | Contract, process, recovery, and fault testing |

## Compatibility

Some v1 identifiers retain the `nous` namespace. This includes crate names,
environment variables, compatibility commands, C/Python API symbols, and
`nous.*.v1` wire identifiers. They are compatibility surfaces, not a second
product name. See [COMPATIBILITY.md](COMPATIBILITY.md).

## Documentation

- [Architecture](ARCHITECTURE.md)
- [Documentation index](docs/README.md)
- [Kernel and distribution boundaries](docs/architecture/APEIR_PROJECT_BOUNDARIES.md)
- [Execution path](docs/architecture/EXECUTION_PATH.md)
- [State ownership](docs/architecture/STATE_OWNERSHIP.md)
- [Provider architecture](docs/architecture/PROVIDER_ARCHITECTURE.md)
- [Provider SDK](docs/providers/provider-sdk.md)
- [Build and conformance](docs/development/BUILD_TEST_CONFORMANCE.md)
- [Contributing](CONTRIBUTING.md)

## Security and license

Please report vulnerabilities according to [SECURITY.md](SECURITY.md), not in a
public issue. APEIR Kernel is licensed under the Apache License 2.0. Dependency
notices are listed in [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES).
