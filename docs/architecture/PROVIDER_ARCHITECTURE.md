# Provider architecture

A provider implements a capability outside the trusted kernel state. The
boundary consists of:

1. `Provider`, the internal asynchronous execute/probe contract;
2. `ProcessProvider`, the kernel-side timeout and child-process controller;
3. `nous-provider-worker`, the one-shot protocol host;
4. backend adapters implementing deterministic, local, edge, or remote work.

## Process boundary

The kernel starts a worker for one request and exchanges one serialized command
through standard I/O. The worker cannot access kernel memory or the journal.
Its environment is cleared, after which only required process variables and an
explicitly allowlisted credential reference are supplied. Credential names are
configured through `NOUS_ALLOWED_CREDENTIALS`; credential values must never be
written to manifests, logs, receipts, or journal records.

Process separation contains worker crashes and gives the kernel a reliable
termination boundary. It is not a filesystem, network, or operating-system
sandbox. Deployment policy must provide those controls when required.

## Result normalization

Provider-specific output does not cross NKI directly. The worker response is
validated and converted to an `OperationReceipt`. Only the kernel may append
the corresponding commit. A crash or malformed response produces a structured
provider error without terminating the daemon.

Timeout and cancellation terminate the worker and use the normal workload
cleanup path. Provider code does not retry an operation. Recovery rules are
owned by the kernel and depend on the workload's delivery semantics.

## Capability discovery

Capability probes use the same worker boundary. An adapter reports a
model-neutral capability manifest; capabilities not established by a probe are
reported as unknown. Remote adapters require HTTPS. Plain HTTP is restricted to
loopback services.

## Continuity

An observed provider failure is recorded as an abort. A continuity plan may
select a compatible provider, append a rebind record, and submit the remaining
logical step through the same authoritative execution path. It cannot change
hard capability, privacy, deadline, resource, or safety requirements.

The deterministic `reference`, `reference-delay`, and `reference-crash`
backends exist for conformance testing. They are not model implementations.
External services require separate configuration and qualification.
