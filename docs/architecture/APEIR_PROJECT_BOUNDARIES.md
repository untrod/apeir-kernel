# APEIR project boundaries

APEIR is organized as two independently releasable projects connected by a
versioned interface. Authority flows from the distribution through NKI to the
kernel; execution backends never receive kernel authority.

```text
APEIR Distribution
desktop / product CLI / API / nodes / relays / integrations
                         |
                         | NKI
                         v
APEIR Kernel
admission / safety / state / scheduling / leases / effects / proof
                         |
                         | provider protocol
                         v
provider or device worker / operating system / service / hardware
```

## Kernel repository

`apeir-kernel` owns:

- NKI framing, version negotiation, methods, and error semantics;
- workload admission and immutable safety constraints;
- authoritative journal state, integrity verification, and recovery;
- resource identity, lease lifecycle, fencing, and release;
- feasibility checks, placement decisions, and decision traces;
- transactional effect intent, receipt, commit, and compensation semantics;
- provider process supervision and normalized execution receipts;
- canonical contracts, conformance tests, and client protocols.

The repository may include deterministic reference providers and declarative
contract validators needed to test the kernel boundary. It does not include a
desktop interface, installer, model-account manager, marketplace, document
editor, agent product, or domain-specific provider implementation.

## Distribution repository

`apeir` owns:

- desktop and user-facing command interfaces;
- product API, workspace, installer, and update lifecycle;
- node, relay, artifact transport, and deployment services;
- model, MCP, skill, WASI, cloud, robotics, and device integrations;
- user approval flows, secret-store integration, and presentation of receipts;
- templates, workflows, examples, and product-level policy.

The distribution may cache committed kernel data for display. It must not write
the journal, mint a lease or permit, bypass admission, or represent an external
service action as kernel-authorized without a corresponding kernel record.

## Executable ownership

| Project | Canonical executable | 0.1 compatibility executable |
| --- | --- | --- |
| Kernel | `apeird` | `nousd` |
| Kernel | `apeir-kernelctl` | `nous` |
| Distribution | `apeir` | product `nous` |
| Distribution | `apeir-node` | `nous-node` |
| Distribution | `apeir-relay` | `nous-relay` |

A packaged installation must not place two different implementations under the
same executable name. Product packages should use the canonical names and keep
compatibility commands only where their ownership is unambiguous.

## Contract ownership

The kernel repository is authoritative for NKI and core execution contracts.
The distribution consumes a pinned contract artifact and must not maintain a
fork with different semantics.

Branding and protocol identity are separate. Existing `nous.*.v1` wire names,
durable IDs, receipts, and signatures retain their bytes and meaning. A future
incompatible protocol requires explicit negotiation, round-trip conversion
tests, and durable-data migration guidance.
