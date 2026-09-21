# ADR 0006: Reality effect verification

Status: Implemented on feature/reality-execution-v2

## Decision

`OperationReceipt` proves only that a provider returned an execution result. It
is not evidence that the requested real-world condition became true. An
operation carrying an `EffectContract` therefore follows one authoritative
path:

```text
intent -> authority -> execute -> OperationReceipt
       -> ObservedEffect -> EffectVerification -> StepCommit
```

`ObservedEffect` is produced by an explicitly configured observer with an
identity and capability. It binds a typed observed value to content-addressed
evidence. `EffectVerification` binds the exact contract digest and observation
digest to a verifier identity and policy revision. Only `MATCH` permits commit.
`PARTIAL`, `MISMATCH`, and `UNKNOWN` fail closed.

For `INDEPENDENT` verification, the provider executor identity cannot equal the
verifier identity. Kernel assigns the executor identity to effectful
`OperationReceipt`; the provider cannot self-assert it. Models may diagnose or propose action, but cannot issue a
Reality Verification receipt merely by claiming success. Deterministic probes
or an explicitly governed human verifier must cross this boundary.

The daemon's first adapter is an explicit, single-service reference configuration
passed as `REALITY_SERVICE_CONFIG` to `serve`. It admits only its configured
target/subject and `apeir.service-health/v1`, probes a numeric loopback socket,
stores bounded content-addressed response evidence beside the Journal, and
reconstructs the observed HTTP status and version from those bytes before
declaring `MATCH`. Without the configuration, a contracted NKI request fails
before provider execution. This is not a general observer registry or the
Distribution Artifact Runtime integration.

Reference configuration example (admin-supplied file, not an NKI payload):

```json
{
  "schema_version": 1,
  "target": "service:test",
  "subject": "service:test",
  "service_address": "127.0.0.1:8080"
}
```

Start with `apeird serve JOURNAL WORKER 127.0.0.1:8771 CONFIG.json`. The
evidence directory is derived from the Journal path (`.evidence`) and is never
client-controlled. The adapter is deliberately limited to this local service
probe; it does not authorize remote endpoints or arbitrary scripts.

## Crash and recovery

- Durable intent without a receipt is replayed only when the shared
  `DeliverySemantics` recovery matrix returns `Replay` (`IDEMPOTENT`).
- A durable `OperationReceipt` without verification resumes observation and
  verification and never invokes the provider again.
- A durable `MATCH` awaiting commit commits the existing verified result.
- `MISMATCH` never becomes success. `UNKNOWN` is fail-closed.
- `AT_MOST_ONCE`, `UNKNOWN`, compensatable, external-commit, at-least-once, and
  merely reconcilable uncertain executions are not blindly replayed.

Compensation is a new effect and requires a new intent, admission, authority,
receipt, observation, and verification.

## Compatibility

`OperationRequest.effect_contract` is optional and omitted on the wire for
legacy requests. NKI protocol versions 1 and 2 remain accepted for requests
without a contract. Contracted requests require NKI version 3 so an older
daemon rejects them rather than silently ignoring the contract. Existing enum
identities and journal format are not renumbered. New facts are appended as new
object types.
