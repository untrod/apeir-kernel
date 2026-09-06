# Runtime and application separation

APEIR components are distributed independently. Process boundaries are part of
the security and recovery model, not packaging conveniences.

| Layer | Components | Responsibility |
| --- | --- | --- |
| Contracts | `nous-types`, `nous-nki`, JSON schemas | Stable data and wire semantics without execution authority |
| Kernel authorities | state, security, resource, scheduler, kernel core | Admission, leases, execution, effects, recovery, authoritative state |
| Kernel processes | `apeird`, provider worker | NKI service, lifecycle, process isolation, backend protocol |
| Clients | Rust/Python/C SDKs, `apeir-kernelctl` | Typed access to NKI without direct state mutation |
| Distribution | desktop, product API/CLI, nodes, relays, integrations | User workflows, deployment, accounts, approvals, extension composition |

The allowed dependency direction is:

```text
distribution -> SDK -> NKI -> kernel authorities -> provider contract
                                      |
                                      v
                               isolated worker
```

Reverse dependencies are prohibited. Kernel crates do not depend on desktop
frameworks, web servers, model vendors, robotics middleware, or product
configuration. `scripts/architecture_check.py` checks crate-level boundaries.

## State ownership

- the journal owns workload lifecycle, effects, receipts, checkpoints, and
  recovery state;
- the control catalog owns versioned Project, Model, Graph, Dataset,
  Experiment, and Artifact declarations;
- the distribution owns presentation preferences, user workflow, account
  configuration, and approval interaction;
- providers own no kernel or catalog state.

Control-catalog writes use generation-based compare-and-swap. Identical writes
are idempotent; changed updates and deletes require the observed generation.
Catalog documents contain references to credentials, never credential values.

## Runtime Control API

Transport-neutral request and response types live in `nous-types`. Durable
implementations live behind NKI; SDKs depend on public contracts only.

Execution methods:

- `SubmitWorkload`
- `GetWorkload`
- `ListWorkloads`
- `CancelWorkload`

Catalog methods:

- `PutControlAsset`
- `GetControlAsset`
- `ListControlAssets`
- `DeleteControlAsset`

The machine-readable method inventory is
`spec/runtime-control-api-v1.json`.

## Application integration

A desktop or product service launches the packaged processes, creates an
ephemeral local-session token, and communicates through an SDK or a stable
facade backed by NKI. It does not import kernel implementation crates, open the
journal, initialize a provider in-process, or duplicate scheduler and recovery
logic.

The token must not be written to the journal, catalog, event stream, or product
configuration. Remote and multi-host operation requires a separate
authenticated transport and is outside the loopback NKI boundary.
