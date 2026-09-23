# Distributed runtime status

Distribution contains durable Relay-to-Node dispatch, Ed25519 Node identity,
signed results, and an external provider adapter that projects a signed result
into Kernel's `RemoteExecutionReceipt` contract. Kernel performs its own trust
and binding admission before that result becomes a Journal fact.

This is production-boundary implementation, not cross-machine acceptance. The
x64 Controller to Windows ARM64 real-effect path, independent remote
observation, disconnect/reconnect matrix, and malicious/stale-node acceptance
matrix have not been run in the current environment. The only registered HTTP
observation adapter still requires an administrator-bound loopback endpoint.

The repository does not implement consensus, a replicated Journal, production
ownership transfer, or exactly-once real-world effects. NKI remains
loopback-only. Unsafe unknown outcomes become `RECOVERY_REQUIRED` and require
reconciliation or operator action rather than automatic replay.
