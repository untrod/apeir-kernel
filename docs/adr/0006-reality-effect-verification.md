# ADR 0006: Reality effect verification

Status: Implemented on feature/reality-execution-v2; M1 validation pending x64 CI

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
verifier identity. Models may diagnose or propose action, but cannot issue a
Reality Verification receipt merely by claiming success. Deterministic probes
or an explicitly governed human verifier must cross this boundary.

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
