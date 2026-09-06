# Production execution path

There is one daemon execution path:

```text
nous / Rust SDK / Python SDK
  -> length-prefixed NKI request
  -> envelope and deadline validation
  -> NEC request validation
  -> admission and hard feasibility
  -> SchedulerCore placement and DecisionTrace
  -> fenced ResourceLease
  -> durable operation intent
  -> isolated provider process
  -> durable provider receipt
  -> durable step commit and workload terminal state
  -> lease release
  -> NKI result
```

A continuity plan is not a second execution path. It sequences committed
steps, evaluates provider compatibility, records a provider Rebind after an
observed failure, and invokes the same path above for each remaining step.

The CLI and SDK do not open the journal or initialize a provider. The provider
worker receives a single serialized command over standard I/O and cannot access
kernel memory.

## Retry and recovery ownership

- The SDK owns connection establishment and one bounded request deadline. It
  does not retry an operation automatically.
- Provider runtime owns transport timeout and child termination. It does not
  retry the operation.
- Execution recovery may re-enter only `IDEMPOTENT` or `RECONCILABLE`
  operations.
- A recorded receipt without a commit is committed without calling the
  provider again.
- An observed provider failure is durably aborted and is not re-executed on
  daemon restart. A later explicit Rebind creates a new recoverable intent for
  the same logical operation.
- `AT_MOST_ONCE`, unknown, and unsafe pending operations fail recovery.

## Cancellation and shutdown

Cancellation is journaled, propagated to the provider wait, terminates the
child process, records the terminal workload state, and releases its lease. An
NKI deadline narrows the operation timeout and uses the same cleanup path.
Shutdown rejects new work, cancels active operations, waits for cleanup, and
then exits. The process-level E2E suite verifies cancellation and graceful
interrupt handling.
