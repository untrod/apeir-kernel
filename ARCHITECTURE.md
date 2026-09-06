# APEIR Kernel architecture

## Purpose

APEIR Kernel controls the lifecycle of heterogeneous workloads without
embedding product interfaces or backend-specific business logic. Its public
boundary is NKI. Its durable authority is the journal. Providers execute work
but do not own policy or state transitions.

## System boundary

```text
APEIR distribution / CLI / SDK
              |
              | NKI request and response
              v
+------------------------------------------------------+
| APEIR Kernel                                         |
| envelope validation | admission | safety             |
| scheduling | resource lease | execution lifecycle    |
| journal | recovery | receipts | effect verification  |
+------------------------------------------------------+
              |
              | provider process protocol
              v
provider / model adapter / algorithm / tool / device adapter
              |
              v
operating system / local service / remote service / device
```

NKI is the only external state-changing interface. Clients do not open the
journal, construct a kernel-owned provider, mutate a lease, or manufacture a
commit. The distribution may project committed state for presentation, but a
projection is never authoritative.

## Execution lifecycle

1. NKI validates framing, protocol version, request identity, session token,
   deadline, and payload schema.
2. Admission checks capability, safety, resource feasibility, and availability.
3. SchedulerCore selects an eligible execution target and records its decision.
4. The resource authority grants a lease with a fencing token.
5. The execution core persists intent before invoking an untrusted provider.
6. The provider runs in a child process with a bounded request and environment.
7. The kernel normalizes the result and persists a receipt.
8. A commit makes the result authoritative; terminal cleanup releases the lease.

Cancellation, timeout, shutdown, and restart use the same lifecycle. Recovery
may resume only operations whose delivery semantics make re-entry safe. A
persisted receipt without a commit is committed without invoking the provider
again.

## Authority and state

| Concern | Authority | Durable representation |
| --- | --- | --- |
| Workload and operation lifecycle | Kernel execution core | append-only journal entries |
| Admission and capability grant | security and admission authorities | decisions and receipts |
| Placement | SchedulerCore | decision trace |
| Resource ownership | lease manager | lease and fencing records |
| Provider output | execution core after normalization | operation receipt and commit |
| Effect lifecycle | effect authority | intent, receipt, commit, compensation |
| Declarative assets | control catalog | versioned records with generation checks |

In-memory maps, queues, metrics, and health observations are derived state.
They may be rebuilt from durable records and must not become a second source of
truth.

## Trust model

The trusted computing base consists of NKI validation, security and admission
decisions, SchedulerCore, lease enforcement, journal interpretation, execution
lifecycle, effect verification, and recovery. Providers, extensions, models,
tools, device adapters, user inputs, and user interfaces are untrusted.

Provider process isolation is a fault-containment boundary, not an operating
system sandbox. The daemon listener is loopback-only. Filesystem isolation,
network policy, secure secret storage, and operating-system account separation
remain deployment responsibilities.

## Contracts and compatibility

Foundation contracts under `spec/contracts/v1` define kernel execution
semantics. Open Runtime contracts under `spec/open-runtime/v1` define portable
extension declarations. Contract validation rejects unknown fields where a
misspelling could change behavior.

The APEIR brand does not change the identity of v1 contracts. `nous.*.v1`,
`NOUS_*`, persisted receipts, IDs, and compatibility executables remain valid
for the 0.1 line. Incompatible wire or durable-state changes require a new
protocol version, negotiation, conversion tests, and migration guidance.

## Dependency rules

Crate dependencies are checked by `scripts/architecture_check.py`. Authority
flows in one direction:

```text
application -> client SDK -> NKI -> kernel authorities -> provider contract
```

Domain frameworks, cloud SDKs, desktop code, and concrete robotics or device
stacks must not become dependencies of the trusted decision and state core.

Detailed specifications:

- [Kernel boundary](docs/architecture/KERNEL_BOUNDARY.md)
- [Execution path](docs/architecture/EXECUTION_PATH.md)
- [State ownership](docs/architecture/STATE_OWNERSHIP.md)
- [Scheduler](docs/architecture/SCHEDULER_ARCHITECTURE.md)
- [Provider boundary](docs/architecture/PROVIDER_ARCHITECTURE.md)
- [Security model](docs/security/SECURITY_MODEL.md)
