# ADR 0006: Reality effect verification

Status: Production-boundary implementation on `feature/reality-execution-v2`;
cross-machine acceptance pending

## Decision

`OperationReceipt` proves only that a provider returned an execution result. It
is not evidence that the requested real-world condition became true. An
operation carrying an `EffectContract` therefore follows one authoritative
path:

```text
intent -> target binding -> execute -> OperationReceipt
       -> ObservedEffect -> EffectVerification -> StepCommit
```

`ObservedEffect` binds a typed value to immutable Artifact references.
`EffectVerification` binds the exact contract and observation digests to a
verifier identity and policy revision. Only `MATCH` permits commit. `PARTIAL`,
`MISMATCH`, and `UNKNOWN` fail closed.

For `INDEPENDENT` verification, neither the Kernel-owned provider executor nor
the executor bound into an admitted remote receipt may equal the verifier.
Kernel assigns the local executor identity. A remote Node supplies a signed
candidate fact which Kernel independently checks against its trust resolver,
Intent, EffectContract, TargetBinding, request digest, delivery semantics, and
protocol version. A Node cannot create Kernel truth directly.

The daemon loads a governed adapter registry from the optional reality config
passed to `serve`. The first and only adapter family is
`apeir.http-service/v1`. Each entry binds a logical target reference to one
versioned `TargetBinding`; NKI callers never supply its endpoint. The adapter
admits only its configured target/subject and `apeir.service-health/v1`, and
retains the numeric-loopback probe restriction. Registry lookup discovers a
compatible mechanism; it does not grant authority.

Observation bytes are written first through the Distribution
`ContentAddressedArtifactStore` bridge. Only the returned `EvidenceRef` is then
journaled in `ObservedEffect`. The verifier resolves and digest-checks those
bytes through the same bridge and independently reconstructs HTTP status and
service version. A missing or modified Artifact prevents commit. An orphan
Artifact created before `ObservedEffect` is safe to garbage-collect; a Journal
reference to missing bytes is data loss and fails closed.

Reference configuration (administrator supplied, never an NKI payload):

```json
{
  "schema_version": 2,
  "artifact_bridge": {
    "program": "C:/path/to/distribution/python.exe",
    "root": "C:/apeir/artifacts"
  },
  "trusted_nodes_path": "C:/apeir/relay/trusted-nodes.json",
  "targets": [{
    "subject": "service:test-api",
    "target_binding": {
      "schema_version": 1,
      "target_ref": "node://arm64-lab/service/test-api",
      "target_kind": "http-service",
      "node_id": "node-arm64",
      "adapter_id": "apeir.http-service/v1",
      "adapter_revision": "1",
      "endpoint_binding": {"address": "127.0.0.1:8080"},
      "allowed_effect_schemas": ["apeir.service-health/v1"],
      "revision": "target-1"
    }
  }]
}
```

Start with `apeird serve JOURNAL WORKER 127.0.0.1:8771 CONFIG.json`. The bridge
program must be a Python runtime containing the installed Distribution package.
Daemon startup checks bridge health before advertising `artifact.evidence` or
Reality capabilities.

Remote execution uses the external provider boundary and Distribution's
`apeir-remote-provider` entry point. Its durable spool feeds the existing Relay
and Node Protocol rather than defining another wire protocol. The signed Node
envelope is projected into `RemoteExecutionReceipt`, then admitted by Kernel as
an `OperationReceipt` fact. `NOUS_NODE_UNCERTAIN_EFFECT` becomes the journaled
state `RECOVERY_REQUIRED`, never ordinary failure or automatic replay.

## Crash and recovery

- Durable intent without a receipt is replayed only when `DeliverySemantics`
  returns `Replay` (`IDEMPOTENT`).
- A durable `OperationReceipt` resumes observation and verification without
  invoking the provider again.
- A durable `MATCH` awaiting commit commits the existing verified result.
- Unsafe pending outcomes are recorded as `RECOVERY_REQUIRED` and not replayed.
- `MISMATCH`, `UNKNOWN`, missing evidence, and digest mismatch never commit.

Compensation is a new effect and requires a new intent, admission, receipt,
observation, and verification.

## Compatibility and authority

`OperationRequest.effect_contract` remains optional. NKI v1/v2 accept requests
without a contract; effectful requests require NKI v3. Existing enum identities
and Journal format are not renumbered; new facts use new object types.

The in-memory `TransactionalEffectEngine` is a domain/test reference. It is not
the production durability authority and does not own remote receipts, Artifact
persistence, daemon recovery, or commit. Production truth is
`DurableExecutor + Journal`.

## Acceptance status

Contract round trips, cross-language canonical digests, Node signature fixtures,
Registry/Target admission, Artifact PUT/GET and tamper rejection, receipt
recovery, and fail-closed provider behavior are locally testable. The x64
Controller to ARM64 Node real-effect experiment, independent remote observation,
reconnect/fault matrix, and final cross-platform acceptance remain pending.
Their absence must not be reported as a pass or as M2 completion.
