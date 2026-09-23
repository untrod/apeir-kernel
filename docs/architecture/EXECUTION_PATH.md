# Production execution path

There is one daemon execution path:

```text
nous / Rust SDK / Python SDK
  -> length-prefixed NKI request
  -> envelope, deadline, and NEC validation
  -> admission, hard feasibility, placement, and fenced lease
  -> durable operation intent
  -> governed TargetBinding (effectful requests)
  -> provider process
       local worker, or external remote-provider adapter
       -> durable Distribution Relay spool
       -> authenticated Node Protocol
  -> durable OperationReceipt
  -> optional EffectContract reality path:
       adapter registry lookup -> authorized observation
       -> Artifact Runtime PUT -> EvidenceRef journal fact
       -> independent Artifact resolve and verification
       -> MATCH only
  -> durable StepCommit and workload terminal state
  -> lease release
  -> NKI result
```

A continuity plan is not a second execution path. It evaluates provider
compatibility and invokes this same path for each remaining logical step. The
CLI and SDK do not open the Journal or initialize a provider.

## Retry and recovery ownership

- SDK owns connection establishment and one bounded request deadline; it does
  not retry an operation automatically.
- Provider runtime owns transport timeout and child termination; it does not
  decide commit.
- Recovery re-executes only `IDEMPOTENT` operations without a receipt.
- A recorded receipt never calls the provider again. Effectful requests resume
  observation and verification.
- Unsafe pending outcomes without a receipt become `RECOVERY_REQUIRED`; they
  are not replayed.
- `MATCH` is the only Reality result that permits commit.
- Remote success is a signed candidate fact. Kernel revalidates Node trust,
  operation, Intent, EffectContract, TargetBinding, request, delivery, provider
  revision, output digest, and Node Protocol bindings.
- Registry TargetBindings and loopback endpoints are administrator configured,
  never taken from an NKI request.
- The HTTP verifier reconstructs health and version from immutable Artifact
  evidence. A forged observer value cannot produce `MATCH`.

`EXPLAIN_EXECUTION` is a Journal projection. It shows only durable stages that
occurred (`durable_intent`, `target_binding`, `provider`, `receipt`,
`observation`, `evidence`, `verification`, and `step_commit`) and redacts raw
provider input/output. It is not a second lifecycle state store.

## Cancellation and shutdown

Cancellation is journaled, propagated to the provider wait, terminates the
child process, records the terminal workload state, and releases its lease. An
NKI deadline narrows the operation timeout and uses the same cleanup path.
Shutdown rejects new work, cancels active operations, waits for cleanup, and
then exits.
